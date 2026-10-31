use super::PhysRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    Reserved,
    Usable,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    ExecutableAndModules,
    Framebuffer,
    MappedReserved,
    Unknown(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub range: PhysRange,
    pub kind: RegionKind,
}

impl MemoryRegion {
    pub const EMPTY: Self = Self {
        range: PhysRange::EMPTY,
        kind: RegionKind::Reserved,
    };
}
