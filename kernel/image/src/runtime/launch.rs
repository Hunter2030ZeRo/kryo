use super::stacks::KernelStack;
use super::{BootFramePool, DirectMap, ExtendedState, Handles, KernelMemory, KernelRegion};
use super::{RefCell, Resources, SharedLease, UserContext, UserSpace};
use crate::boot::vm_backend::new_shared_space;
use kryo_core::exec::{ElfImage, LoadLimits};
use kryo_core::mm::vm::Permissions;
use kryo_core::process::ExitReason;
use kryo_interfaces::syscall::Error;

const USER_STACK: u64 = 0x80_0000;
const STACK_PAGES: u64 = 4;
const TLS: u64 = 0x90_0000;

pub(crate) struct PendingLoad<'kernel, 'objects> {
    space: Option<UserSpace<'kernel, SharedLease<'objects>>>,
    stack: Option<KernelStack>,
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
    let mut pending = PendingLoad {
        space: None,
        stack: None,
    };
    let result = (|| {
        let image = ElfImage::<8>::parse(bytes, LoadLimits { max_pages: 8 })
            .map_err(|_| Error::InvalidArgument)?;
        for page in image.pages() {
            if super::threads::reserved_address(page.address()) {
                return Err(Error::InvalidArgument);
            }
        }
        let extended = ExtendedState::new().map_err(|_| Error::InvalidArgument)?;
        pending.space = Some(new_shared_space(pool, memory).map_err(|_| Error::OutOfResources)?);
        let space = pending.space.as_mut().unwrap();
        let mut buffer = [0; 4096];
        for page in image.pages() {
            page.initialize(&mut buffer);
            space
                .map_page(page.address(), page.permissions(), &buffer)
                .map_err(|_| Error::OutOfResources)?;
        }
        for index in 0..STACK_PAGES {
            space
                .map_page(USER_STACK + index * 4096, Permissions::ReadWrite, &[])
                .map_err(|_| Error::OutOfResources)?;
        }
        space
            .map_page(TLS, Permissions::ReadWrite, &[])
            .map_err(|_| Error::OutOfResources)?;
        let mut context = UserContext::new(image.entry(), USER_STACK + STACK_PAGES * 4096)
            .map_err(|_| Error::InvalidArgument)?;
        context.rdx = u64::from(extended.avx_enabled());
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
            space: pending.space.take().unwrap(),
            handles: Handles::growing(),
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
    })
}
