use super::stacks::KernelStack;
use super::{BootFramePool, DirectMap, ExtendedState, Handles, KernelMemory, KernelRegion};
use super::{RefCell, Resources, SharedLease, UserContext, UserSpace};
use crate::boot::vm_backend::new_shared_space_limited;
use kryo_core::exec::{AuxEntry, ElfImage, LoadLimits, build_initial_stack};
use kryo_core::mm::vm::Permissions;
use kryo_core::process::ExitReason;
use kryo_core::resource::{Budget, Charge, Resource};
use kryo_interfaces::syscall::Error;

const USER_STACK: u64 = 0x80_0000;
const STACK_PAGES: u64 = 4;
const TLS: u64 = 0x90_0000;
const MAX_IMAGE_PAGES: usize = 16;

pub(crate) struct NativeImage<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) argv: &'a [&'a [u8]],
    pub(crate) envp: &'a [&'a [u8]],
}

struct Image<'a> {
    bytes: &'a [u8],
    arguments: Option<NativeImage<'a>>,
}

pub(crate) struct PendingLoad<'kernel, 'objects> {
    space: Option<UserSpace<'kernel, SharedLease<'objects>>>,
    stack: Option<KernelStack>,
    thread_charge: Option<Charge>,
}

pub(crate) struct LoadFailure<'kernel, 'objects> {
    pub(crate) error: Error,
    pub(crate) pending: PendingLoad<'kernel, 'objects>,
}

impl PendingLoad<'_, '_> {
    pub(crate) fn close(&mut self, pool: &BootFramePool, region: &mut KernelRegion) -> bool {
        if let Some(space) = self.space.as_mut() {
            if space.close().is_err() {
                return false;
            }
            self.space = None;
        }
        if let Some(stack) = self.stack.as_mut() {
            // No thread has entered this unpublished stack. Its user root has
            // retired before the shared kernel alias and frame owner end.
            if unsafe { stack.close(pool, region) }.is_err() {
                return false;
            }
            self.stack = None;
        }
        self.thread_charge = None;
        true
    }
}

impl<'kernel, 'objects> PendingLoad<'kernel, 'objects> {
    pub(crate) fn from_resources(resources: Resources<'kernel, 'objects>) -> Self {
        Self {
            space: if resources.process.space_closed {
                None
            } else {
                Some(resources.process.space)
            },
            stack: Some(resources.stack),
            thread_charge: resources.thread_charge,
        }
    }
}

#[expect(
    clippy::result_large_err,
    reason = "The creation transaction returns retained mapping owners for cleanup retry"
)]
pub(crate) fn prepare<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    first_slot: usize,
    bytes: &[u8],
) -> Result<Resources<'kernel, 'objects>, LoadFailure<'kernel, 'objects>> {
    prepare_budgeted(pool, direct, memory, region, first_slot, bytes, None)
}

#[expect(
    clippy::result_large_err,
    reason = "Failed preparation retains mapping owners"
)]
pub(crate) fn prepare_budgeted<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    first_slot: usize,
    bytes: &[u8],
    account: Option<Budget>,
) -> Result<Resources<'kernel, 'objects>, LoadFailure<'kernel, 'objects>> {
    prepare_image(
        pool,
        direct,
        memory,
        region,
        first_slot,
        Image {
            bytes,
            arguments: None,
        },
        account,
    )
}

#[expect(
    clippy::result_large_err,
    reason = "Failed preparation retains mapping owners"
)]
pub(crate) fn prepare_native<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    first_slot: usize,
    image: NativeImage<'_>,
    account: Option<Budget>,
) -> Result<Resources<'kernel, 'objects>, LoadFailure<'kernel, 'objects>> {
    prepare_image(
        pool,
        direct,
        memory,
        region,
        first_slot,
        Image {
            bytes: image.bytes,
            arguments: Some(image),
        },
        account,
    )
}

#[expect(
    clippy::result_large_err,
    reason = "Failed preparation retains mapping owners"
)]
fn prepare_image<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    first_slot: usize,
    request: Image<'_>,
    account: Option<Budget>,
) -> Result<Resources<'kernel, 'objects>, LoadFailure<'kernel, 'objects>> {
    let mut pending = PendingLoad {
        space: None,
        stack: None,
        thread_charge: None,
    };
    let result = (|| {
        pending.thread_charge = account
            .as_ref()
            .map(|budget| budget.try_charge(Resource::Threads, 1))
            .transpose()
            .map_err(|_| Error::OutOfResources)?;
        let image = ElfImage::<8>::parse(
            request.bytes,
            LoadLimits {
                max_pages: MAX_IMAGE_PAGES,
            },
        )
        .map_err(|_| Error::InvalidArgument)?;
        for page in image.pages() {
            if super::threads::reserved_address(page.address()) {
                return Err(Error::InvalidArgument);
            }
        }
        let native_headers = if request.arguments.is_some() {
            let header = image.elf_header_address().ok_or(Error::InvalidArgument)?;
            let headers = image.program_headers().ok_or(Error::InvalidArgument)?;
            if header.checked_add(headers.file_offset) != Some(headers.address) {
                return Err(Error::InvalidArgument);
            }
            Some(headers)
        } else {
            None
        };
        let extended = ExtendedState::new().map_err(|_| Error::InvalidArgument)?;
        let (mappings, tables) = account.as_ref().map_or((usize::MAX, usize::MAX), |budget| {
            (budget.limits().mappings, budget.limits().tables)
        });
        pending.space = Some(
            new_shared_space_limited(pool, memory, mappings, tables)
                .map_err(|_| Error::OutOfResources)?,
        );
        let space = pending.space.as_mut().unwrap();
        let mut buffer = [0; 4096];
        for page in image.pages() {
            page.initialize(&mut buffer);
            space
                .map_page(page.address(), page.permissions(), &buffer)
                .map_err(|_| Error::OutOfResources)?;
        }
        let native = request.arguments.is_some();
        let stack_pointer = if let Some(arguments) = request.arguments {
            let headers = native_headers.expect("Native ELF metadata validation invariant");
            let mut stack = [0u8; STACK_PAGES as usize * 4096];
            let pointer = build_initial_stack(
                &mut stack,
                USER_STACK,
                arguments.argv,
                arguments.envp,
                &[
                    AuxEntry {
                        kind: 3,
                        value: headers.address,
                    },
                    AuxEntry {
                        kind: 4,
                        value: headers.entry_size,
                    },
                    AuxEntry {
                        kind: 5,
                        value: headers.count,
                    },
                    AuxEntry {
                        kind: 6,
                        value: 4096,
                    },
                    AuxEntry {
                        kind: 9,
                        value: image.entry(),
                    },
                ],
            )
            .map_err(|_| Error::InvalidArgument)?;
            for (index, page) in stack.as_chunks::<4096>().0.iter().enumerate() {
                space
                    .map_page(
                        USER_STACK + index as u64 * 4096,
                        Permissions::ReadWrite,
                        page,
                    )
                    .map_err(|_| Error::OutOfResources)?;
            }
            pointer
        } else {
            for index in 0..STACK_PAGES {
                space
                    .map_page(USER_STACK + index * 4096, Permissions::ReadWrite, &[])
                    .map_err(|_| Error::OutOfResources)?;
            }
            USER_STACK + STACK_PAGES * 4096
        };
        space
            .map_page(TLS, Permissions::ReadWrite, &[])
            .map_err(|_| Error::OutOfResources)?;
        let mut context =
            UserContext::new(image.entry(), stack_pointer).map_err(|_| Error::InvalidArgument)?;
        // SysV entry reserves RDX for the runtime finalizer, absent here. The
        // older boot convention instead reports its optional AVX probe flag.
        context.rdx = if native {
            0
        } else {
            u64::from(extended.avx_enabled())
        };
        match KernelStack::try_new(pool, direct, region, first_slot) {
            Ok(stack) => pending.stack = Some(stack),
            Err(failure) => {
                let _ = kryo_hal::print(format_args!(
                    "PROCESS: entry stack creation failed error={:?} cleanup={:?}\n",
                    failure.error, failure.cleanup_error,
                ));
                pending.stack = failure.retained;
                return Err(Error::OutOfResources);
            }
        }
        Ok((context, extended))
    })();
    let (context, extended) = match result {
        Ok(result) => result,
        Err(error) => {
            return Err(LoadFailure { error, pending });
        }
    };
    Ok(Resources {
        process: super::ProcessResources {
            private_vm: kryo_core::mm::vm::PrivateVm::new(),
            space: pending.space.take().unwrap(),
            handles: account
                .clone()
                .map_or_else(Handles::growing, Handles::budgeted),
            account,
            control: None,
            pending: None,
            pending_thread: None,
            space_closed: false,
        },
        stack: pending.stack.take().unwrap(),
        context,
        ticks: 0,
        exit_code: u64::MAX,
        exit_reason: ExitReason::Exited,
        wait: None,
        receive_slots: None,
        received_wait: None,
        received_handles: [None; 2],
        pool,
        memory,
        direct,
        extended: Some(extended),
        user_slot: 0,
        user_stack_owned: true,
        thread_control: None,
        thread_charge: pending.thread_charge.take(),
    })
}

impl Drop for PendingLoad<'_, '_> {
    fn drop(&mut self) {
        if self.space.is_some() || self.stack.is_some() {
            core::mem::forget(self.thread_charge.take());
        }
    }
}
