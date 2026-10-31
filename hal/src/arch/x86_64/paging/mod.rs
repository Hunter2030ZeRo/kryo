mod boot;
mod direct_map;

pub use boot::BootPageSlot;
pub use direct_map::DirectMap;

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
}
