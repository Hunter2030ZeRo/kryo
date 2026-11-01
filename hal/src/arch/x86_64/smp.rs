use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering, fence};
use x86_64::instructions::{interrupts, tlb};
use x86_64::registers::control::{Cr4, Cr4Flags};

use super::cpu::{AtomicCpuSet, CpuSet, MAX_CPUS, cpu_apic_id, cpu_by_index, current_cpu};

const RESCHEDULE_VECTOR: u8 = 0xe1;
const SHOOTDOWN_VECTOR: u8 = 0xe2;
const ACK_POLLS: usize = 2_000_000;
const ACK_TIMEOUT_NS: u64 = 100_000_000;

#[cfg(feature = "boot-tests")]
static MISS_ACK: [core::sync::atomic::AtomicBool; MAX_CPUS] =
    [const { core::sync::atomic::AtomicBool::new(false) }; MAX_CPUS];

static ONLINE: AtomicCpuSet = AtomicCpuSet::new(CpuSet::singleton(0));
static LEASED: AtomicCpuSet = AtomicCpuSet::new(CpuSet::empty());
static TLB_EPOCH: AtomicU64 = AtomicU64::new(0);
static TLB_ACK: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static RESCHEDULE_EPOCH: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmpError {
    InvalidCpu,
    InterruptsEnabled,
    NotInitialized,
    Offline,
    Busy,
    AlreadyLeased,
    NotLeased,
    Delivery,
    TimedOut,
    EpochExhausted,
}

pub fn cpu_online_mask() -> CpuSet {
    ONLINE.snapshot(Ordering::Acquire)
}
pub fn active_user_mask() -> CpuSet {
    LEASED.snapshot(Ordering::Acquire)
}
pub fn all_users_quiescent() -> bool {
    active_user_mask().is_empty()
}
pub fn tlb_epoch() -> u64 {
    TLB_EPOCH.load(Ordering::Acquire)
}

pub fn cpu_tlb_epoch(cpu: usize) -> Result<u64, SmpError> {
    validate_cpu(cpu)?;
    Ok(TLB_ACK[cpu].load(Ordering::Acquire))
}

pub fn reschedule_epoch(cpu: usize) -> Result<u64, SmpError> {
    validate_cpu(cpu)?;
    Ok(RESCHEDULE_EPOCH[cpu].load(Ordering::Acquire))
}

/// # Safety
/// Online publication follows this CPU's boot-root, IDT, and local-APIC setup.
/// Serialize CPU startup/shutdown with BSP page-table policy and user leases.
/// Offline removal requires a completed AP handshake: no user job, mapping,
/// clock access, or future work may survive it. Retain the AP's parked stack.
pub unsafe fn set_cpu_online(cpu: usize, online: bool) -> Result<(), SmpError> {
    check_irqs()?;
    validate_cpu(cpu)?;
    if !all_users_quiescent() {
        return Err(SmpError::Busy);
    }
    let local = current_cpu().ok_or(SmpError::InvalidCpu)?.index();
    if online {
        if local != cpu || !super::apic::timer_initialized() {
            return Err(SmpError::NotInitialized);
        }
        local_tlb_flush();
        TLB_ACK[cpu].store(tlb_epoch(), Ordering::Release);
        ONLINE.insert(cpu, Ordering::AcqRel);
    } else {
        if local != 0 && local != cpu {
            return Err(SmpError::InvalidCpu);
        }
        if cpu == 0 {
            return Err(SmpError::InvalidCpu);
        }
        ONLINE.remove(cpu, Ordering::AcqRel);
    }
    Ok(())
}

/// # Safety
/// The BSP owns the task/context and sets this lease before publishing an AP
/// mailbox. No page-table operation or other execution owner may overlap it.
pub unsafe fn begin_user_lease(cpu: usize) -> Result<(), SmpError> {
    check_bsp()?;
    validate_cpu(cpu)?;
    if !cpu_online_mask().contains(cpu) {
        return Err(SmpError::Offline);
    }
    if !acknowledged(cpu_online_mask(), tlb_epoch(), &TLB_ACK) {
        return Err(SmpError::Busy);
    }
    claim_lease(&LEASED, cpu)
}

/// # Safety
/// The BSP has acquired the completed mailbox and restored its task owner. The
/// AP has restored the boot CR3 and ended every access through the leased root.
pub unsafe fn end_user_lease(cpu: usize) -> Result<(), SmpError> {
    check_bsp()?;
    validate_cpu(cpu)?;
    release_lease(&LEASED, cpu)
}

/// Drain queued reschedule delivery before arming the next user slice. The
/// caller holds no locks and has completed its notification barrier, so no old
/// producer can enqueue another E1 after this check. E0/E2 handlers remain live.
pub fn drain_reschedule() -> Result<(), SmpError> {
    check_irqs()?;
    if current_cpu().is_none() || !super::apic::timer_initialized() {
        return Err(SmpError::NotInitialized);
    }
    if super::context::user_active() {
        return Err(SmpError::Busy);
    }
    for _ in 0..64 {
        let (pending, in_service) =
            super::apic::vector_state(RESCHEDULE_VECTOR).map_err(|_| SmpError::Delivery)?;
        if in_service {
            return Err(SmpError::Busy);
        }
        if !pending {
            return Ok(());
        }
        unsafe { asm!("sti", "nop", "cli", options(nostack)) };
    }
    Err(SmpError::TimedOut)
}

pub fn send_reschedule(cpu: usize) -> Result<(), SmpError> {
    check_irqs()?;
    send(cpu, RESCHEDULE_VECTOR)
}

/// Wake one registered AP from its early, offline boot wait.
///
/// # Safety
/// The BSP calls with IRQs disabled after acquiring the AP's parked handshake.
/// That handshake proves the target loaded its GDT/IDT and initialized its local
/// APIC, including E1 handling. Retain its parked stack and the shared APIC
/// mapping. Publish the boot state before this wake, serialize startup with VM
/// policy, and do not use this path after online publication or for CPU restart.
pub unsafe fn wake_boot_cpu(cpu: usize) -> Result<(), SmpError> {
    check_bsp()?;
    let id = validate_cpu(cpu)?;
    validate_boot_wake(cpu, cpu_online_mask(), active_user_mask())?;
    if super::context::user_active() {
        return Err(SmpError::Busy);
    }
    if !super::apic::timer_initialized() {
        return Err(SmpError::NotInitialized);
    }
    super::apic::send_ipi(cpu_apic_id(id) as u8, RESCHEDULE_VECTOR).map_err(|_| SmpError::Delivery)
}

/// Idle with IRQs enabled only across STI/HLT; the caller checks its mailbox and
/// runnable state with IRQs off, holds no locks, and retains local APIC setup.
pub fn idle_event() -> Result<(), SmpError> {
    check_irqs()?;
    if current_cpu().is_none() || !super::apic::timer_initialized() {
        return Err(SmpError::NotInitialized);
    }
    unsafe { asm!("sti", "hlt", "cli", options(nostack)) };
    Ok(())
}

pub(crate) fn reschedule_interrupt() {
    if let Some(cpu) = current_cpu() {
        let _ = RESCHEDULE_EPOCH[cpu.index()].try_update(
            Ordering::Release,
            Ordering::Relaxed,
            |value| value.checked_add(1),
        );
    }
    super::apic::timer_acknowledge();
}

pub(crate) fn shootdown_interrupt() {
    if let Some(cpu) = current_cpu() {
        let epoch = TLB_EPOCH.load(Ordering::Acquire);
        local_tlb_flush();
        #[cfg(feature = "boot-tests")]
        if MISS_ACK[cpu.index()].swap(false, Ordering::AcqRel) {
            super::apic::timer_acknowledge();
            return;
        }
        TLB_ACK[cpu.index()].store(epoch, Ordering::Release);
    }
    super::apic::timer_acknowledge();
}

/// Complete a BSP kernel-mapping update on every online CPU. User-root access
/// must first return to boot CR3; its non-global translations retire at return.
pub fn global_tlb_flush() -> Result<(), SmpError> {
    check_bsp()?;
    if !all_users_quiescent() {
        return Err(SmpError::Busy);
    }
    let epoch = TLB_EPOCH
        .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map_err(|_| SmpError::EpochExhausted)?
        + 1;
    local_tlb_flush();
    TLB_ACK[0].store(epoch, Ordering::Release);
    let mut targets = cpu_online_mask();
    targets.remove(0);
    if targets.is_empty() {
        return Ok(());
    }
    fence(Ordering::Release);
    for cpu in targets.iter() {
        send(cpu, SHOOTDOWN_VECTOR)?;
    }
    let start = super::clock::monotonic_time().ok();
    for _ in 0..ACK_POLLS {
        if acknowledged(targets, epoch, &TLB_ACK) {
            return Ok(());
        }
        if let Some(start) = start
            && let Ok(now) = super::clock::monotonic_time()
            && now.saturating_sub(start) >= ACK_TIMEOUT_NS
        {
            break;
        }
        core::hint::spin_loop();
    }
    Err(SmpError::TimedOut)
}

#[cfg(feature = "boot-tests")]
/// # Safety
/// Run on the BSP with all user jobs quiescent. This suppresses exactly one
/// remote flush acknowledgement to verify retained ownership and explicit retry.
pub unsafe fn inject_tlb_miss(cpu: usize) -> Result<(), SmpError> {
    check_bsp()?;
    validate_cpu(cpu)?;
    if cpu == 0 || !cpu_online_mask().contains(cpu) {
        return Err(SmpError::Offline);
    }
    if !all_users_quiescent() {
        return Err(SmpError::Busy);
    }
    MISS_ACK[cpu].store(true, Ordering::Release);
    Ok(())
}

pub(crate) fn local_tlb_flush() {
    let flags = Cr4::read();
    if flags.contains(Cr4Flags::PAGE_GLOBAL) {
        unsafe {
            Cr4::write(flags & !Cr4Flags::PAGE_GLOBAL);
            Cr4::write(flags);
        }
    } else {
        tlb::flush_all();
    }
}

fn claim_lease(leases: &AtomicCpuSet, cpu: usize) -> Result<(), SmpError> {
    if !leases.insert(cpu, Ordering::AcqRel) {
        return Err(SmpError::AlreadyLeased);
    }
    Ok(())
}

fn release_lease(leases: &AtomicCpuSet, cpu: usize) -> Result<(), SmpError> {
    if !leases.remove(cpu, Ordering::AcqRel) {
        return Err(SmpError::NotLeased);
    }
    Ok(())
}

fn acknowledged(mask: CpuSet, epoch: u64, acknowledgements: &[AtomicU64; MAX_CPUS]) -> bool {
    mask.iter()
        .all(|cpu| acknowledgements[cpu].load(Ordering::Acquire) >= epoch)
}

fn send(cpu: usize, vector: u8) -> Result<(), SmpError> {
    let id = validate_cpu(cpu)?;
    if !cpu_online_mask().contains(cpu) {
        return Err(SmpError::Offline);
    }
    super::apic::send_ipi(cpu_apic_id(id) as u8, vector).map_err(|_| SmpError::Delivery)
}

fn validate_cpu(cpu: usize) -> Result<super::cpu::CpuId, SmpError> {
    cpu_by_index(cpu).ok_or(SmpError::InvalidCpu)
}

fn validate_boot_wake(cpu: usize, online: CpuSet, leased: CpuSet) -> Result<(), SmpError> {
    if cpu == 0 || cpu >= MAX_CPUS {
        return Err(SmpError::InvalidCpu);
    }
    if online.contains(cpu) || !leased.is_empty() {
        return Err(SmpError::Busy);
    }
    Ok(())
}

fn check_irqs() -> Result<(), SmpError> {
    if interrupts::are_enabled() {
        return Err(SmpError::InterruptsEnabled);
    }
    Ok(())
}

fn check_bsp() -> Result<(), SmpError> {
    check_irqs()?;
    if current_cpu().map(|cpu| cpu.index()) != Some(0) {
        return Err(SmpError::InvalidCpu);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(cpus: &[usize]) -> CpuSet {
        let mut set = CpuSet::empty();
        for cpu in cpus {
            set.insert(*cpu);
        }
        set
    }

    #[test]
    fn early_wake_is_only_for_offline_aps_without_user_leases() {
        let boot = CpuSet::singleton(0);
        let empty = CpuSet::empty();
        assert_eq!(validate_boot_wake(1, boot, empty), Ok(()));
        assert_eq!(validate_boot_wake(MAX_CPUS - 1, boot, empty), Ok(()));
        assert_eq!(
            validate_boot_wake(0, boot, empty),
            Err(SmpError::InvalidCpu)
        );
        assert_eq!(
            validate_boot_wake(MAX_CPUS, boot, empty),
            Err(SmpError::InvalidCpu)
        );
        assert_eq!(
            validate_boot_wake(usize::MAX, boot, empty),
            Err(SmpError::InvalidCpu)
        );
        assert_eq!(
            validate_boot_wake(1, set(&[0, 1]), empty),
            Err(SmpError::Busy)
        );
        for cpu in [0, 63, 64, 127, 254] {
            assert_eq!(
                validate_boot_wake(1, boot, CpuSet::singleton(cpu)),
                Err(SmpError::Busy)
            );
        }
        assert_eq!(
            validate_boot_wake(254, set(&[0, 254]), empty),
            Err(SmpError::Busy)
        );
    }

    #[test]
    fn leases_reject_duplicate_ownership_across_words() {
        let leases = AtomicCpuSet::new(CpuSet::empty());
        let cpus = [1, 63, 64, 127, 128, 254];
        for cpu in cpus {
            assert_eq!(claim_lease(&leases, cpu), Ok(()));
            assert_eq!(claim_lease(&leases, cpu), Err(SmpError::AlreadyLeased));
        }
        for cpu in cpus {
            assert_eq!(release_lease(&leases, cpu), Ok(()));
            assert_eq!(release_lease(&leases, cpu), Err(SmpError::NotLeased));
            assert!(!leases.snapshot(Ordering::Acquire).contains(cpu));
            if cpu != 254 {
                assert!(!leases.snapshot(Ordering::Acquire).is_empty());
            }
        }
        assert!(leases.snapshot(Ordering::Acquire).is_empty());
    }

    #[test]
    fn acknowledgements_require_every_target_across_words_and_current_epoch() {
        let acks = [const { AtomicU64::new(0) }; MAX_CPUS];
        let targets = set(&[1, 63, 64, 127, 128, 254]);
        assert!(acknowledged(CpuSet::empty(), 7, &acks));
        for cpu in targets.iter() {
            acks[cpu].store(7, Ordering::Relaxed);
        }
        acks[254].store(6, Ordering::Relaxed);
        assert!(!acknowledged(targets, 7, &acks));
        acks[254].store(7, Ordering::Relaxed);
        assert!(acknowledged(targets, 7, &acks));
        assert!(!acknowledged(targets, 8, &acks));
        let mut extended = targets;
        extended.insert(192);
        assert!(!acknowledged(extended, 7, &acks));
    }
}
