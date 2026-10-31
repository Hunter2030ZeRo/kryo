use super::memory::BootFramePool;
use core::cell::RefCell;
use kryo_core::mm::frame::{FrameAllocError, OwnedFrame};
use kryo_core::mm::vm::{AddressSpace, Permissions, VmBackend, VmError};
use kryo_hal::{DirectMap, KernelDirectMap, PagePermissions, PagingError, UserPageTable};

const MAX_KERNEL_TABLES: usize = 32;
pub(super) type UserSpace<'kernel> =
    AddressSpace<'static, BootFramePool, UserBackend<'kernel>, 16, 64>;

static KERNEL_TABLES: spin::Mutex<Option<[Option<OwnedFrame<'static>>; MAX_KERNEL_TABLES]>> =
    spin::Mutex::new(None);

pub(super) enum BackendError {
    Paging(PagingError),
    Allocation(FrameAllocError),
    TableCapacity,
}

impl core::fmt::Debug for BackendError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Paging(error) => formatter.debug_tuple("Paging").field(error).finish(),
            Self::Allocation(error) => formatter.debug_tuple("Allocation").field(error).finish(),
            Self::TableCapacity => formatter.write_str("TableCapacity"),
        }
    }
}

impl From<PagingError> for BackendError {
    fn from(error: PagingError) -> Self {
        Self::Paging(error)
    }
}

impl From<FrameAllocError> for BackendError {
    fn from(error: FrameAllocError) -> Self {
        Self::Allocation(error)
    }
}

pub(super) struct KernelMemory {
    pool: &'static BootFramePool,
    direct: DirectMap,
    aliases: KernelDirectMap,
    tables: [Option<OwnedFrame<'static>>; MAX_KERNEL_TABLES],
    count: usize,
    table_limit: usize,
}

impl KernelMemory {
    pub(super) unsafe fn new(
        pool: &'static BootFramePool,
        direct: DirectMap,
        physical_end: u64,
    ) -> Result<Self, BackendError> {
        // Establish supervisor NX direct-map policy before any user roots are cloned.
        let aliases = unsafe { KernelDirectMap::new(direct, physical_end) }?;
        Ok(Self {
            pool,
            direct,
            aliases,
            tables: [const { None }; MAX_KERNEL_TABLES],
            count: 0,
            table_limit: MAX_KERNEL_TABLES,
        })
    }

    fn prepare(&mut self, physical: u64) -> Result<(), BackendError> {
        let needed = self.aliases.tables_needed(physical)?;
        if self.count + needed > self.table_limit {
            return Err(BackendError::TableCapacity);
        }
        let mut frames: [Option<OwnedFrame<'static>>; 2] = [None, None];
        for index in 0..needed {
            match self.pool.allocate() {
                Ok(frame) => frames[index] = Some(frame),
                Err(error) => {
                    for frame in frames.into_iter().flatten() {
                        // These table frames have not been published.
                        unsafe { self.pool.release(frame) }
                            .expect("Cannot roll back kernel tables");
                    }
                    return Err(error.into());
                }
            }
        }
        let mut addresses = [0; 2];
        for index in 0..needed {
            addresses[index] = frames[index].as_ref().unwrap().address();
        }
        // HAL validates the complete split before publishing table links.
        let prepared = unsafe { self.aliases.prepare_frame(physical, &addresses[..needed]) };
        match prepared {
            Ok(used) => {
                for (index, frame) in frames.into_iter().enumerate() {
                    if let Some(frame) = frame {
                        if index < used {
                            self.tables[self.count] = Some(frame);
                            self.count += 1;
                        } else {
                            unsafe { self.pool.release(frame) }
                                .expect("Cannot return unused kernel table");
                        }
                    }
                }
                Ok(())
            }
            Err(error) => {
                for frame in frames.into_iter().flatten() {
                    unsafe { self.pool.release(frame) }
                        .expect("Cannot return rejected kernel table");
                }
                Err(error.into())
            }
        }
    }

    pub(super) fn retain_tables(self) -> usize {
        let mut retained = KERNEL_TABLES.lock();
        assert!(
            retained.is_none(),
            "Kernel table ownership already retained"
        );
        *retained = Some(self.tables);
        self.count
    }
}

pub(super) fn check_preparation_failure(
    pool: &'static BootFramePool,
    kernel: &RefCell<KernelMemory>,
) {
    let before = pool.stats();
    kernel.borrow_mut().table_limit = 0;
    let mut space = new_space(pool, kernel).expect("Cannot create failure-test space");
    let result = space.map_page(0x40_0000, Permissions::ReadExecute, &[0xcc]);
    assert!(matches!(
        result,
        Err(VmError::Backend(BackendError::TableCapacity))
    ));
    assert_eq!(space.mapped_pages(), 0);
    space.close().expect("Cannot close rejected user mapping");
    kernel.borrow_mut().table_limit = MAX_KERNEL_TABLES;
    assert_eq!(pool.stats(), before);
    kryo_hal::emergency_write_str("VM: alias preparation rollback OK\n");
}

pub(super) struct UserBackend<'kernel> {
    tables: UserPageTable,
    kernel: &'kernel RefCell<KernelMemory>,
}

pub(super) fn new_space<'kernel>(
    pool: &'static BootFramePool,
    kernel: &'kernel RefCell<KernelMemory>,
) -> Result<UserSpace<'kernel>, VmError<BackendError>> {
    AddressSpace::new(pool, |root| {
        let direct = kernel.borrow().direct;
        // Core owns this new root frame, and the BSP boot root remains active.
        let tables = unsafe { UserPageTable::new(direct, root) }?;
        Ok(UserBackend { tables, kernel })
    })
}

// Core retains every root/table/data token. Kernel direct-map splits are separately
// retained for the kernel lifetime. All calls run on the BSP with IRQs disabled.
unsafe impl VmBackend for UserBackend<'_> {
    type Error = BackendError;

    fn root_frame(&self) -> u64 {
        self.tables.root_frame()
    }

    fn check_inactive(&self) -> Result<(), Self::Error> {
        self.tables.check_inactive().map_err(Into::into)
    }

    fn tables_needed(&self, address: u64) -> Result<usize, Self::Error> {
        self.tables.tables_needed(address).map_err(Into::into)
    }

    unsafe fn initialize_frame(
        &mut self,
        physical: u64,
        contents: &[u8],
    ) -> Result<(), Self::Error> {
        let mut kernel = self.kernel.borrow_mut();
        kernel.prepare(physical)?;
        unsafe {
            kernel.direct.zero_frame(physical)?;
            kernel.direct.write_frame(physical, 0, contents)?;
        }
        Ok(())
    }

    unsafe fn seal_frame(
        &mut self,
        physical: u64,
        permissions: Permissions,
    ) -> Result<(), Self::Error> {
        let mut kernel = self.kernel.borrow_mut();
        unsafe {
            kernel
                .aliases
                .protect_frame(physical, !matches!(permissions, Permissions::ReadWrite))
        }?;
        let alias = kernel.aliases.frame_permissions(physical)?;
        assert!(!alias.executable);
        assert_eq!(
            alias.writable,
            matches!(permissions, Permissions::ReadWrite)
        );
        Ok(())
    }

    unsafe fn restore_frame(&mut self, physical: u64) -> Result<(), Self::Error> {
        let mut kernel = self.kernel.borrow_mut();
        let alias = kernel.aliases.frame_permissions(physical)?;
        // Preparation can fail before splitting a huge, already safe RW+NX alias.
        if alias.writable && !alias.executable {
            return Ok(());
        }
        unsafe { kernel.aliases.protect_frame(physical, false) }?;
        Ok(())
    }

    unsafe fn map(
        &mut self,
        address: u64,
        physical: u64,
        permissions: Permissions,
        tables: &[u64],
    ) -> Result<usize, Self::Error> {
        let flags = PagePermissions {
            writable: matches!(permissions, Permissions::ReadWrite),
            executable: matches!(permissions, Permissions::ReadExecute),
        };
        unsafe { self.tables.map(address, physical, flags, tables) }.map_err(Into::into)
    }

    unsafe fn unmap(&mut self, address: u64) -> Result<u64, Self::Error> {
        unsafe { self.tables.unmap(address) }.map_err(Into::into)
    }

    unsafe fn read_frame(
        &self,
        physical: u64,
        offset: usize,
        output: &mut [u8],
    ) -> Result<(), Self::Error> {
        let kernel = self.kernel.borrow();
        unsafe { kernel.direct.read_frame(physical, offset, output) }.map_err(Into::into)
    }

    unsafe fn retire(&mut self) -> Result<(), Self::Error> {
        unsafe { self.tables.retire() }.map_err(Into::into)
    }
}
