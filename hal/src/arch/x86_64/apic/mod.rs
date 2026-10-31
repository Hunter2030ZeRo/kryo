use core::arch::asm;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use x86_64::VirtAddr;
use x86_64::instructions::{interrupts, port::Port};
use x86_64::registers::model_specific::Msr;

const APIC_BASE_MSR: u32 = 0x1b;
const APIC_ENABLE: u64 = 1 << 11;
const X2APIC_ENABLE: u64 = 1 << 10;
const BSP: u64 = 1 << 8;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const TIMER_VECTOR: u32 = 0xe0;
const SPURIOUS_VECTOR: u32 = 0xff;
const MASKED: u32 = 1 << 16;
const PERIODIC: u32 = 1 << 17;

const VERSION: usize = 0x30;
const TASK_PRIORITY: usize = 0x80;
const EOI: usize = 0xb0;
const IN_SERVICE: usize = 0x100;
const PENDING: usize = 0x200;
const SVR: usize = 0xf0;
const TIMER_LVT: usize = 0x320;
const INITIAL_COUNT: usize = 0x380;
const DIVIDE: usize = 0x3e0;
const OTHER_LVTS: [usize; 6] = [0x330, 0x340, 0x350, 0x360, 0x370, 0x2f0];

static MMIO: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);
static IRQ_EPOCH: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct SavedTimer {
    apic_base: u64,
    svr: u32,
    priority: u32,
    timer: u32,
    divide: u32,
    initial: u32,
    others: [u32; 6],
    other_count: usize,
    pic: [u8; 2],
}

struct SavedState(UnsafeCell<Option<SavedTimer>>);
// Only the BSP accesses saved configuration, with local IRQs disabled.
unsafe impl Sync for SavedState {}

static SAVED: SavedState = SavedState(UnsafeCell::new(None));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    UnsupportedController,
    InterruptsEnabled,
    InvalidAddress,
    AlreadyInitialized,
    NotInitialized,
    InvalidCount,
    ControllerBusy,
    NotArmed,
    EpochExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleCause {
    Timer,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleWake {
    pub cause: IdleCause,
    pub before: u64,
    pub after: u64,
}

/// Number of timer IRQs actually delivered to the installed handler.
/// The epoch remains monotonic across controller initialization and shutdown.
pub fn timer_irq_epoch() -> u64 {
    IRQ_EPOCH.load(Ordering::Acquire)
}

/// Sleeps on the BSP until an interrupt, returning with maskable IRQs disabled.
///
/// Check that no task is runnable with IRQs disabled before calling. Hold no
/// allocator, scheduler, or object locks across idle. The initialized timer and
/// its mapping must remain valid. Arm a fresh one-shot or periodic event; armed
/// state alone does not promise another event after a one-shot was delivered.
/// This single-CPU primitive does not provide SMP wakeup or idle coordination.
pub fn idle_until_interrupt() -> Result<IdleWake, TimerError> {
    let before = timer_irq_epoch();
    validate_idle(
        interrupts::are_enabled(),
        timer_initialized(),
        timer_ready(),
        before,
    )?;
    timer_physical_address()?;
    // STI's interrupt shadow covers HLT, closing the check-to-sleep race.
    unsafe { asm!("sti", "hlt", "cli", options(nostack)) };
    let after = timer_irq_epoch();
    Ok(classify_wake(before, after))
}

pub(crate) fn record_timer_interrupt() {
    advance_epoch(&IRQ_EPOCH);
}

fn advance_epoch(epoch: &AtomicU64) {
    let _ = epoch.try_update(Ordering::Release, Ordering::Relaxed, |value| {
        value.checked_add(1)
    });
}

fn validate_idle(
    interrupts_enabled: bool,
    initialized: bool,
    armed: bool,
    epoch: u64,
) -> Result<(), TimerError> {
    if interrupts_enabled {
        return Err(TimerError::InterruptsEnabled);
    }
    if !initialized {
        return Err(TimerError::NotInitialized);
    }
    if !armed {
        return Err(TimerError::NotArmed);
    }
    if epoch == u64::MAX {
        return Err(TimerError::EpochExhausted);
    }
    Ok(())
}

fn classify_wake(before: u64, after: u64) -> IdleWake {
    IdleWake {
        cause: if after > before {
            IdleCause::Timer
        } else {
            IdleCause::Other
        },
        before,
        after,
    }
}

pub fn timer_physical_address() -> Result<u64, TimerError> {
    if core::arch::x86_64::__cpuid(1).edx & (1 << 9) == 0 {
        return Err(TimerError::UnsupportedController);
    }
    let base = unsafe { Msr::new(APIC_BASE_MSR).read() };
    decode_base(base)
}

pub fn timer_initialized() -> bool {
    MMIO.load(Ordering::Acquire) != 0
}

/// True only when the initialized timer is armed for a user time slice.
pub fn timer_ready() -> bool {
    timer_initialized() && ARMED.load(Ordering::Acquire)
}

/// Reports a queued timer vector without changing controller state.
pub fn timer_pending() -> bool {
    let pointer = MMIO.load(Ordering::Acquire);
    if pointer == 0 {
        return false;
    }
    let register = TIMER_VECTOR as usize / 32 * 16;
    let bit = 1u32 << (TIMER_VECTOR % 32);
    unsafe { read(pointer, PENDING + register) & bit != 0 }
}

/// # Safety
/// Call once on the BSP with IRQs disabled. The virtual page must map the
/// current xAPIC MMIO base as supervisor writable, NX and uncached. Retain the
/// mapping until shutdown. Install timer vector 0xe0 and spurious vector 0xff
/// handlers before allowing interrupts. No APs may use this state.
pub unsafe fn init_timer(mmio_virtual: u64) -> Result<(), TimerError> {
    check_irqs()?;
    if timer_initialized() {
        return Err(TimerError::AlreadyInitialized);
    }
    if !mmio_virtual.is_multiple_of(4096)
        || mmio_virtual < 0xffff_8000_0000_0000
        || VirtAddr::try_new(mmio_virtual).is_err()
        || mmio_virtual
            .checked_add(4095)
            .and_then(|end| VirtAddr::try_new(end).ok())
            .is_none()
    {
        return Err(TimerError::InvalidAddress);
    }
    timer_physical_address()?;
    let base = unsafe { Msr::new(APIC_BASE_MSR).read() };
    unsafe { Msr::new(APIC_BASE_MSR).write(base | APIC_ENABLE) };
    let pointer = mmio_virtual as usize;
    let version = unsafe { read(pointer, VERSION) };
    let max_lvt = (version >> 16) & 0xff;
    if version & 0xff < 0x10 || max_lvt < 5 {
        unsafe { Msr::new(APIC_BASE_MSR).write(base) };
        return Err(TimerError::UnsupportedController);
    }

    let count = if max_lvt >= 6 { 6 } else { 5 };
    let mut others = [0; 6];
    for index in 0..count {
        others[index] = unsafe { read(pointer, OTHER_LVTS[index]) };
    }
    let saved = SavedTimer {
        apic_base: base,
        svr: unsafe { read(pointer, SVR) },
        priority: unsafe { read(pointer, TASK_PRIORITY) },
        timer: unsafe { read(pointer, TIMER_LVT) },
        divide: unsafe { read(pointer, DIVIDE) },
        initial: unsafe { read(pointer, INITIAL_COUNT) },
        others,
        other_count: count,
        pic: unsafe { [Port::<u8>::new(0x21).read(), Port::<u8>::new(0xa1).read()] },
    };
    unsafe {
        write(pointer, TIMER_LVT, MASKED | TIMER_VECTOR);
        write(pointer, INITIAL_COUNT, 0);
        for index in 0..count {
            write(pointer, OTHER_LVTS[index], others[index] | MASKED);
        }
        write(pointer, SVR, (saved.svr & !0xff) | 0x100 | SPURIOUS_VECTOR);
        write(pointer, TASK_PRIORITY, 0);
        write(pointer, DIVIDE, 0x3);
        Port::<u8>::new(0x21).write(0xff);
        Port::<u8>::new(0xa1).write(0xff);
        *SAVED.0.get() = Some(saved);
    }
    MMIO.store(pointer, Ordering::Release);
    ARMED.store(false, Ordering::Release);
    Ok(())
}

/// # Safety
/// Run on the owning BSP with IRQs disabled and retain the MMIO mapping.
/// initial_count defines hardware ticks, not a calibrated duration.
pub unsafe fn arm_timer(initial_count: u32) -> Result<(), TimerError> {
    unsafe { arm(initial_count, true) }
}

/// # Safety
/// Run on the owning BSP with IRQs disabled and retain the MMIO mapping.
/// Use one-shot mode to queue a bounded interrupt for shutdown-drain checks.
pub unsafe fn arm_timer_once(initial_count: u32) -> Result<(), TimerError> {
    unsafe { arm(initial_count, false) }
}

unsafe fn arm(initial_count: u32, periodic: bool) -> Result<(), TimerError> {
    check_irqs()?;
    if initial_count == 0 {
        return Err(TimerError::InvalidCount);
    }
    let pointer = active_pointer()?;
    unsafe {
        write(pointer, DIVIDE, 0x3);
        write(
            pointer,
            TIMER_LVT,
            TIMER_VECTOR | if periodic { PERIODIC } else { 0 },
        );
        write(pointer, INITIAL_COUNT, initial_count);
    }
    ARMED.store(true, Ordering::Release);
    Ok(())
}

/// # Safety
/// Run on the owning BSP with IRQs disabled and the MMIO mapping retained.
pub unsafe fn stop_timer() -> Result<(), TimerError> {
    check_irqs()?;
    let pointer = active_pointer()?;
    unsafe {
        write(pointer, TIMER_LVT, read(pointer, TIMER_LVT) | MASKED);
        write(pointer, INITIAL_COUNT, 0);
    }
    ARMED.store(false, Ordering::Release);
    Ok(())
}

/// # Safety
/// No user context or pending kernel operation may use the timer. Call on the
/// owning BSP with IRQs disabled, before unmapping its MMIO page.
/// No allocator/object locks may be held: a bounded IRQ window drains pending
/// timer delivery through the installed CPL0 acknowledge-and-return handler.
pub unsafe fn shutdown_timer() -> Result<(), TimerError> {
    check_irqs()?;
    let pointer = active_pointer()?;
    unsafe { stop_timer() }?;
    unsafe { drain_pending(pointer) }?;
    let saved = unsafe { (*SAVED.0.get()).take() }.ok_or(TimerError::NotInitialized)?;
    MMIO.store(0, Ordering::Release);
    unsafe {
        for index in 0..saved.other_count {
            write(pointer, OTHER_LVTS[index], saved.others[index]);
        }
        write(pointer, DIVIDE, saved.divide);
        write(pointer, INITIAL_COUNT, saved.initial);
        write(pointer, TIMER_LVT, saved.timer);
        write(pointer, SVR, saved.svr);
        write(pointer, TASK_PRIORITY, saved.priority);
        Port::<u8>::new(0x21).write(saved.pic[0]);
        Port::<u8>::new(0xa1).write(saved.pic[1]);
        Msr::new(APIC_BASE_MSR).write(saved.apic_base);
    }
    Ok(())
}

/// Acknowledge only this CPU's local APIC interrupt. Valid after init_timer.
pub fn timer_acknowledge() {
    let pointer = MMIO.load(Ordering::Acquire);
    if pointer != 0 {
        unsafe { write(pointer, EOI, 0) };
    }
}

fn check_irqs() -> Result<(), TimerError> {
    if interrupts::are_enabled() {
        return Err(TimerError::InterruptsEnabled);
    }
    Ok(())
}

fn active_pointer() -> Result<usize, TimerError> {
    let pointer = MMIO.load(Ordering::Acquire);
    if pointer == 0 {
        return Err(TimerError::NotInitialized);
    }
    Ok(pointer)
}

fn decode_base(raw: u64) -> Result<u64, TimerError> {
    if raw & X2APIC_ENABLE != 0 || raw & BSP == 0 {
        return Err(TimerError::UnsupportedController);
    }
    let address = raw & ADDRESS_MASK;
    if address == 0 {
        return Err(TimerError::InvalidAddress);
    }
    Ok(address)
}

unsafe fn drain_pending(pointer: usize) -> Result<(), TimerError> {
    let register = TIMER_VECTOR as usize / 32 * 16;
    let bit = 1u32 << (TIMER_VECTOR % 32);
    for _ in 0..64 {
        let in_service = unsafe { read(pointer, IN_SERVICE + register) };
        if in_service & bit != 0 {
            return Err(TimerError::ControllerBusy);
        }
        let pending = unsafe { read(pointer, PENDING + register) };
        if pending & bit == 0 {
            return Ok(());
        }
        // All other local sources and the PIC are masked. The CPL0 stub only
        // acknowledges the pending timer and IRETs; it cannot schedule a task.
        unsafe { asm!("sti", "nop", "cli") };
    }
    Err(TimerError::ControllerBusy)
}

unsafe fn read(base: usize, register: usize) -> u32 {
    unsafe { ((base + register) as *const u32).read_volatile() }
}

unsafe fn write(base: usize, register: usize, value: u32) {
    unsafe { ((base + register) as *mut u32).write_volatile(value) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_requires_disabled_interrupts_and_an_armed_controller() {
        assert_eq!(validate_idle(false, true, true, 0), Ok(()));
        assert_eq!(
            validate_idle(true, true, true, 0),
            Err(TimerError::InterruptsEnabled)
        );
        assert_eq!(
            validate_idle(false, false, false, 0),
            Err(TimerError::NotInitialized)
        );
        assert_eq!(
            validate_idle(false, true, false, 0),
            Err(TimerError::NotArmed)
        );
        assert_eq!(
            validate_idle(false, true, true, u64::MAX),
            Err(TimerError::EpochExhausted)
        );
    }

    #[test]
    fn delivered_timer_epoch_distinguishes_other_wakeups() {
        assert_eq!(
            classify_wake(12, 12),
            IdleWake {
                cause: IdleCause::Other,
                before: 12,
                after: 12
            }
        );
        assert_eq!(
            classify_wake(12, 13),
            IdleWake {
                cause: IdleCause::Timer,
                before: 12,
                after: 13
            }
        );
        assert_eq!(classify_wake(12, 15).cause, IdleCause::Timer);
    }

    #[test]
    fn delivery_epoch_never_wraps() {
        let epoch = AtomicU64::new(u64::MAX - 1);
        advance_epoch(&epoch);
        assert_eq!(epoch.load(Ordering::Acquire), u64::MAX);
        advance_epoch(&epoch);
        assert_eq!(epoch.load(Ordering::Acquire), u64::MAX);
    }

    #[test]
    fn base_decoder_separates_address_mode_and_boot_cpu() {
        let physical = 0xfee0_0000;
        assert_eq!(decode_base(physical | BSP | APIC_ENABLE), Ok(physical));
        assert_eq!(decode_base(physical | BSP), Ok(physical));
        assert_eq!(
            decode_base(physical | BSP | X2APIC_ENABLE),
            Err(TimerError::UnsupportedController)
        );
        assert_eq!(
            decode_base(physical | APIC_ENABLE),
            Err(TimerError::UnsupportedController)
        );
        assert_eq!(
            decode_base(BSP | APIC_ENABLE),
            Err(TimerError::InvalidAddress)
        );
    }
}
