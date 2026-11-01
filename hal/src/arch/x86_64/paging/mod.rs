mod boot;
mod direct_map;
mod kernel_map;
mod region;
mod user;

pub use boot::BootPageSlot;
pub use direct_map::DirectMap;
pub use kernel_map::KernelDirectMap;
pub use region::{KernelRegion, RegionPermissions};
pub use user::{PagePermissions, UserPageTable};

const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagingError {
    InvalidAddress,
    UnalignedAddress,
    AddressOverflow,
    AliasedFrame,
    UnsupportedPaging,
    InterruptsEnabled,
    SlotOccupied,
    AlreadyMapped,
    NotMapped,
    ContextChanged,
    ActiveAddressSpace,
    InvalidPermissions,
    InsufficientTables,
    ShootdownPending,
    UsersActive,
}
