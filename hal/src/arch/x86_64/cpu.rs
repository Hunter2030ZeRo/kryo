use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use x86_64::instructions::interrupts;
use x86_64::registers::model_specific::Msr;

pub const MAX_CPUS: usize = 255;
const CPU_WORDS: usize = MAX_CPUS.div_ceil(64);
const APIC_IDS: usize = 256;
const UNREGISTERED: u32 = u32::MAX;
const UNREGISTERED_SLOT: u8 = u8::MAX;

const _: () = assert!(MAX_CPUS > 0 && MAX_CPUS <= UNREGISTERED_SLOT as usize);

/// Logical CPU membership; independent of the machine word and APIC ID widths.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CpuSet {
    words: [u64; CPU_WORDS],
}

impl CpuSet {
    pub const fn empty() -> Self {
        Self {
            words: [0; CPU_WORDS],
        }
    }

    pub const fn singleton(cpu: usize) -> Self {
        assert!(cpu < MAX_CPUS);
        let mut set = Self::empty();
        set.words[cpu / 64] = 1 << (cpu % 64);
        set
    }

    pub fn contains(self, cpu: usize) -> bool {
        cpu < MAX_CPUS && self.words[cpu / 64] & (1 << (cpu % 64)) != 0
    }

    /// Returns false for an invalid CPU or one already present.
    pub fn insert(&mut self, cpu: usize) -> bool {
        if cpu >= MAX_CPUS {
            return false;
        }
        let changed = !self.contains(cpu);
        self.words[cpu / 64] |= 1 << (cpu % 64);
        changed
    }

    pub fn remove(&mut self, cpu: usize) -> bool {
        if cpu >= MAX_CPUS {
            return false;
        }
        let changed = self.contains(cpu);
        self.words[cpu / 64] &= !(1 << (cpu % 64));
        changed
    }

    pub fn is_empty(self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    pub fn count_ones(self) -> u32 {
        self.words.iter().map(|word| word.count_ones()).sum()
    }

    pub fn iter(self) -> impl Iterator<Item = usize> {
        self.words.into_iter().enumerate().flat_map(|(word, bits)| {
            let mut remaining = bits;
            core::iter::from_fn(move || {
                if remaining == 0 {
                    return None;
                }
                let bit = remaining.trailing_zeros() as usize;
                remaining &= remaining - 1;
                Some(word * 64 + bit)
            })
        })
    }
}

impl Default for CpuSet {
    fn default() -> Self {
        Self::empty()
    }
}

impl core::fmt::Debug for CpuSet {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_set().entries(self.iter()).finish()
    }
}

/// Each membership bit publishes independently. A snapshot is not a transaction
/// across words; callers serialize membership changes against user/VM policy.
pub(crate) struct AtomicCpuSet {
    words: [AtomicU64; CPU_WORDS],
}

impl AtomicCpuSet {
    pub(crate) const fn new(set: CpuSet) -> Self {
        let mut words = [const { AtomicU64::new(0) }; CPU_WORDS];
        let mut index = 0;
        while index < CPU_WORDS {
            words[index] = AtomicU64::new(set.words[index]);
            index += 1;
        }
        Self { words }
    }

    pub(crate) fn snapshot(&self, ordering: Ordering) -> CpuSet {
        CpuSet {
            words: core::array::from_fn(|index| self.words[index].load(ordering)),
        }
    }

    pub(crate) fn insert(&self, cpu: usize, ordering: Ordering) -> bool {
        assert!(cpu < MAX_CPUS);
        let bit = 1 << (cpu % 64);
        self.words[cpu / 64].fetch_or(bit, ordering) & bit == 0
    }

    pub(crate) fn remove(&self, cpu: usize, ordering: Ordering) -> bool {
        assert!(cpu < MAX_CPUS);
        let bit = 1 << (cpu % 64);
        self.words[cpu / 64].fetch_and(!bit, ordering) & bit != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuId(usize);

impl CpuId {
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuError {
    InterruptsEnabled,
    NotBootstrapCpu,
    UnsupportedApic,
    InvalidSlot,
    InvalidApicId,
    DuplicateApicId,
    SlotOccupied,
    CpuMismatch,
}

struct CpuRegistry {
    apic_ids: [AtomicU32; MAX_CPUS],
    slots: [AtomicU8; APIC_IDS],
}

impl CpuRegistry {
    const fn new() -> Self {
        Self {
            apic_ids: [const { AtomicU32::new(UNREGISTERED) }; MAX_CPUS],
            slots: [const { AtomicU8::new(UNREGISTERED_SLOT) }; APIC_IDS],
        }
    }

    fn register(&self, slot: usize, apic_id: u32) -> Result<CpuId, CpuError> {
        if slot >= MAX_CPUS {
            return Err(CpuError::InvalidSlot);
        }
        // Physical xAPIC destination 0xff broadcasts instead of naming one CPU.
        if apic_id >= u8::MAX as u32 {
            return Err(CpuError::InvalidApicId);
        }
        let registered = self.slots[apic_id as usize].load(Ordering::Acquire);
        if registered != UNREGISTERED_SLOT && usize::from(registered) != slot {
            return Err(CpuError::DuplicateApicId);
        }
        let previous = self.apic_ids[slot].load(Ordering::Acquire);
        if previous != UNREGISTERED && previous != apic_id {
            return Err(CpuError::SlotOccupied);
        }
        if previous == apic_id && usize::from(registered) == slot {
            return Ok(CpuId(slot));
        }
        // The reverse lookup is the publication point. Its acquire reader must
        // observe the forward identity before using any CPU-local slot.
        self.apic_ids[slot].store(apic_id, Ordering::Release);
        self.slots[apic_id as usize].store(slot as u8, Ordering::Release);
        Ok(CpuId(slot))
    }

    fn find(&self, apic_id: u32) -> Option<CpuId> {
        let slot = self.slots.get(apic_id as usize)?.load(Ordering::Acquire);
        (slot != UNREGISTERED_SLOT).then_some(CpuId(usize::from(slot)))
    }

    fn by_index(&self, slot: usize) -> Option<CpuId> {
        let id = self.apic_ids.get(slot)?.load(Ordering::Acquire);
        self.find(id).filter(|cpu| cpu.index() == slot)
    }
}

static CPUS: CpuRegistry = CpuRegistry::new();

/// # Safety
/// Call on the BSP with IRQs disabled before publishing any AP entry point.
/// Slot zero remains assigned to this CPU for the kernel's entire lifetime.
pub unsafe fn init_boot_cpu() -> Result<CpuId, CpuError> {
    check_registration_context()?;
    CPUS.register(0, initial_apic_id().ok_or(CpuError::UnsupportedApic)?)
}

/// # Safety
/// The BSP is the sole writer, with IRQs disabled and no NMI registration. Assign
/// each AP before publishing its entry point. Never reuse a CPU slot or APIC ID;
/// every per-CPU pointer relies on this immutable lifetime assignment.
pub unsafe fn register_cpu(slot: usize, apic_id: u32) -> Result<CpuId, CpuError> {
    check_registration_context()?;
    if slot == 0 && initial_apic_id() != Some(apic_id) {
        return Err(CpuError::CpuMismatch);
    }
    CPUS.register(slot, apic_id)
}

pub fn current_cpu() -> Option<CpuId> {
    // x86-64 guarantees leaf 1. Registration already checked APIC support;
    // identity lookup on a trap path needs only this single serializing query.
    CPUS.find(core::arch::x86_64::__cpuid(1).ebx >> 24)
}

pub fn cpu_by_index(slot: usize) -> Option<CpuId> {
    CPUS.by_index(slot)
}

pub fn cpu_apic_id(cpu: CpuId) -> u32 {
    CPUS.apic_ids[cpu.index()].load(Ordering::Acquire)
}

fn initial_apic_id() -> Option<u32> {
    if core::arch::x86_64::__cpuid(0).eax < 1 {
        return None;
    }
    let basic = core::arch::x86_64::__cpuid(1);
    if basic.edx & (1 << 9) == 0 {
        return None;
    }
    Some(basic.ebx >> 24)
}

fn check_registration_context() -> Result<(), CpuError> {
    if interrupts::are_enabled() {
        return Err(CpuError::InterruptsEnabled);
    }
    initial_apic_id().ok_or(CpuError::UnsupportedApic)?;
    let apic = unsafe { Msr::new(0x1b).read() };
    if apic & (1 << 8) == 0 {
        return Err(CpuError::NotBootstrapCpu);
    }
    if apic & (1 << 10) != 0 {
        return Err(CpuError::UnsupportedApic);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_preserves_sparse_apic_identity_and_slot_lifetime() {
        let cpus = CpuRegistry::new();
        assert_eq!(cpus.find(7), None);
        assert_eq!(cpus.by_index(0), None);
        assert_eq!(cpus.register(0, 7), Ok(CpuId(0)));
        assert_eq!(cpus.register(1, 31), Ok(CpuId(1)));
        assert_eq!(cpus.find(7), Some(CpuId(0)));
        assert_eq!(cpus.find(31), Some(CpuId(1)));
        assert_eq!(cpus.find(0), None);
        assert_eq!(cpus.by_index(1), Some(CpuId(1)));
        assert_eq!(cpus.register(0, 7), Ok(CpuId(0)));
        assert_eq!(cpus.register(1, 7), Err(CpuError::DuplicateApicId));
        assert_eq!(cpus.register(0, 8), Err(CpuError::SlotOccupied));
        assert_eq!(cpus.find(7), Some(CpuId(0)));
    }

    #[test]
    fn unrepresentable_ids_and_slots_do_not_publish_entries() {
        let cpus = CpuRegistry::new();
        assert_eq!(cpus.register(MAX_CPUS, 1), Err(CpuError::InvalidSlot));
        assert_eq!(cpus.register(0, 256), Err(CpuError::InvalidApicId));
        assert_eq!(cpus.register(0, u32::MAX), Err(CpuError::InvalidApicId));
        assert_eq!(cpus.by_index(MAX_CPUS), None);
        assert_eq!(cpus.by_index(0), None);
        assert_eq!(cpus.find(256), None);
        assert_eq!(cpus.find(u32::MAX), None);
        for slot in 0..MAX_CPUS {
            assert_eq!(cpus.register(slot, slot as u32), Ok(CpuId(slot)));
        }
    }

    #[test]
    fn last_logical_cpu_supports_the_last_directed_xapic_id() {
        let cpus = CpuRegistry::new();
        assert_eq!(MAX_CPUS, 255);
        assert_eq!(cpus.register(254, 254), Ok(CpuId(254)));
        assert_eq!(cpus.find(254), Some(CpuId(254)));
        assert_eq!(cpus.by_index(254), Some(CpuId(254)));
        assert_eq!(cpus.register(255, 253), Err(CpuError::InvalidSlot));
        assert_eq!(cpus.register(253, 255), Err(CpuError::InvalidApicId));
        assert_eq!(cpus.register(253, 256), Err(CpuError::InvalidApicId));
        assert_eq!(cpus.by_index(253), None);
        assert_eq!(cpus.find(255), None);
        assert_eq!(cpus.find(253), None);
    }

    #[test]
    fn reverse_lookup_preserves_every_id_and_rejects_reassignment() {
        let cpus = CpuRegistry::new();
        for slot in 0..MAX_CPUS {
            let apic_id = 254 - slot as u32;
            assert_eq!(cpus.register(slot, apic_id), Ok(CpuId(slot)));
            assert_eq!(cpus.find(apic_id), Some(CpuId(slot)));
        }
        assert_eq!(cpus.register(254, 254), Err(CpuError::DuplicateApicId));
        for slot in 0..MAX_CPUS {
            let apic_id = 254 - slot as u32;
            assert_eq!(cpus.find(apic_id), Some(CpuId(slot)));
            assert_eq!(cpus.by_index(slot), Some(CpuId(slot)));
            assert_eq!(cpus.apic_ids[slot].load(Ordering::Acquire), apic_id);
        }
    }

    #[test]
    fn cpu_sets_cross_word_boundaries_without_aliasing_or_tail_bits() {
        let mut set = CpuSet::empty();
        assert!(set.is_empty());
        for cpu in [0, 63, 64, 127, 128, 191, 192, 254] {
            assert!(set.insert(cpu));
            assert!(!set.insert(cpu));
            assert!(set.contains(cpu));
        }
        assert_eq!(set.count_ones(), 8);
        assert!(!set.contains(MAX_CPUS));
        assert!(!set.contains(usize::MAX));
        assert!(!set.insert(MAX_CPUS));
        assert!(!set.remove(MAX_CPUS));
        assert!(set.iter().eq([0, 63, 64, 127, 128, 191, 192, 254]));
        for cpu in [0, 63, 64, 127, 128, 191, 192] {
            assert!(set.remove(cpu));
            assert!(!set.is_empty());
        }
        assert_eq!(set, CpuSet::singleton(254));
        assert!(set.remove(254));
        assert!(set.is_empty());
    }
}
