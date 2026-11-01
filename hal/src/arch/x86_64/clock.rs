use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use x86_64::VirtAddr;
use x86_64::instructions::interrupts;

const GENERAL_CONFIG: usize = 0x10;
const MAIN_COUNTER: usize = 0xf0;
const TIMER_CONFIG: usize = 0x100;
const TIMER_STRIDE: usize = 0x20;
const ENABLED: u64 = 1;
const LEGACY: u64 = 1 << 1;
const TIMER_IRQ: u64 = 1 << 2;
const TIMER_FSB: u64 = 1 << 14;
const PROGRESS_POLLS: usize = 100_000;

static MMIO: AtomicUsize = AtomicUsize::new(0);
static INITIAL: AtomicU64 = AtomicU64::new(0);
static PERIOD_FS: AtomicU64 = AtomicU64::new(0);
static READERS: AtomicUsize = AtomicUsize::new(0);

struct ClockRead;
impl Drop for ClockRead {
    fn drop(&mut self) {
        READERS.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Clock {
    general: u64,
    timers: [u64; 32],
    timer_count: usize,
}

struct ClockState(UnsafeCell<Option<Clock>>);
// Only BSP initialization/shutdown touches saved firmware state. Readers use
// the atomically published immutable counter parameters and a bounded read guard.
unsafe impl Sync for ClockState {}
static STATE: ClockState = ClockState(UnsafeCell::new(None));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    InterruptsEnabled,
    UnsupportedCpu,
    InvalidAddress,
    AlreadyInitialized,
    NotInitialized,
    UnsupportedCounter,
    InvalidPeriod,
    Stalled,
    CounterWrapped,
    Exhausted,
    Busy,
}

pub fn clock_initialized() -> bool {
    MMIO.load(Ordering::Acquire) != 0
}

/// # Safety
/// Call on the BSP with IRQs disabled and exclusive HPET authority. The supplied
/// page must map the ACPI HPET block supervisor writable, NX and uncached, with
/// compatible aliases. Retain it until shutdown. Comparator access is exclusive
/// except for a timer explicitly lent through claim_timer_device.
pub unsafe fn init_clock(mmio_virtual: u64) -> Result<(), ClockError> {
    check_owner()?;
    if clock_initialized() {
        return Err(ClockError::AlreadyInitialized);
    }
    validate_address(mmio_virtual)?;
    let pointer = mmio_virtual as usize;
    let (period_fs, timer_count) = decode_capabilities(unsafe { read(pointer, 0) })?;
    let mut timers = [0; 32];
    for (index, config) in timers.iter_mut().enumerate().take(timer_count) {
        *config = unsafe { read(pointer, TIMER_CONFIG + index * TIMER_STRIDE) };
    }
    let general = unsafe { read(pointer, GENERAL_CONFIG) };
    for (index, config) in timers.iter().copied().enumerate().take(timer_count) {
        unsafe {
            write(
                pointer,
                TIMER_CONFIG + index * TIMER_STRIDE,
                config & !(TIMER_IRQ | TIMER_FSB),
            )
        };
    }
    unsafe { write(pointer, GENERAL_CONFIG, (general & !LEGACY) | ENABLED) };
    let initial = unsafe { read(pointer, MAIN_COUNTER) };
    let state = Clock {
        general,
        timers,
        timer_count,
    };
    let mut progressed = false;
    for _ in 0..PROGRESS_POLLS {
        if unsafe { read(pointer, MAIN_COUNTER) } > initial {
            progressed = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !progressed {
        unsafe { restore(pointer, &state) };
        return Err(ClockError::Stalled);
    }
    unsafe { *STATE.0.get() = Some(state) };
    INITIAL.store(initial, Ordering::Relaxed);
    PERIOD_FS.store(u64::from(period_fs), Ordering::Relaxed);
    MMIO.store(pointer, Ordering::Release);
    Ok(())
}

/// Returns elapsed nanoseconds since this retained clock was initialized.
/// Registered CPUs read with IRQs disabled. The clock excludes suspend/resume and
/// firmware counter resets; either requires a separate clock-epoch policy.
pub fn monotonic_time() -> Result<u64, ClockError> {
    check_context()?;
    let pointer = MMIO.load(Ordering::Acquire);
    if pointer == 0 {
        return Err(ClockError::NotInitialized);
    }
    // Sequential ordering closes the reader-registration/shutdown race: a
    // reader missed by shutdown's count check must observe the withdrawn page.
    READERS.fetch_add(1, Ordering::SeqCst);
    let _read = ClockRead;
    if MMIO.load(Ordering::SeqCst) != pointer {
        return Err(ClockError::NotInitialized);
    }
    let initial = INITIAL.load(Ordering::Relaxed);
    let period = PERIOD_FS.load(Ordering::Relaxed) as u32;
    let counter = unsafe { read(pointer, MAIN_COUNTER) };
    elapsed_ns(initial, counter, period)
}

/// # Safety
/// No pending deadline, calibration, task, or idle operation may still use this
/// clock. Call on the owning BSP with IRQs disabled before removing its mapping.
/// This restores comparator/general settings without resetting the main counter.
pub unsafe fn shutdown_clock() -> Result<(), ClockError> {
    check_owner()?;
    if super::smp::cpu_online_mask().iter().any(|cpu| cpu != 0)
        || !super::smp::all_users_quiescent()
        || super::hpet_device::device_claimed()
    {
        return Err(ClockError::Busy);
    }
    let pointer = MMIO.swap(0, Ordering::SeqCst);
    if pointer == 0 {
        return Err(ClockError::NotInitialized);
    }
    for _ in 0..PROGRESS_POLLS {
        if READERS.load(Ordering::SeqCst) == 0 {
            let state = unsafe { (*STATE.0.get()).take() }.ok_or(ClockError::NotInitialized)?;
            unsafe { restore(pointer, &state) };
            return Ok(());
        }
        core::hint::spin_loop();
    }
    MMIO.store(pointer, Ordering::Release);
    Err(ClockError::Busy)
}

pub(super) fn device_pointer() -> Result<usize, ClockError> {
    check_owner()?;
    if !super::smp::all_users_quiescent() || super::context::user_active() {
        return Err(ClockError::Busy);
    }
    let pointer = MMIO.load(Ordering::Acquire);
    if pointer == 0 {
        return Err(ClockError::NotInitialized);
    }
    Ok(pointer)
}

fn check_owner() -> Result<(), ClockError> {
    check_context()?;
    if super::cpu::current_cpu().is_none_or(|cpu| cpu.index() != 0) {
        return Err(ClockError::UnsupportedCpu);
    }
    Ok(())
}

fn check_context() -> Result<(), ClockError> {
    if interrupts::are_enabled() {
        return Err(ClockError::InterruptsEnabled);
    }
    if super::cpu::current_cpu().is_none() {
        return Err(ClockError::UnsupportedCpu);
    }
    Ok(())
}

fn validate_address(address: u64) -> Result<(), ClockError> {
    if !address.is_multiple_of(4096)
        || address < 0xffff_8000_0000_0000
        || VirtAddr::try_new(address).is_err()
        || address
            .checked_add(4095)
            .and_then(|end| VirtAddr::try_new(end).ok())
            .is_none()
    {
        return Err(ClockError::InvalidAddress);
    }
    Ok(())
}

fn decode_capabilities(raw: u64) -> Result<(u32, usize), ClockError> {
    if raw & 0xff == 0 || raw & (1 << 13) == 0 {
        return Err(ClockError::UnsupportedCounter);
    }
    let period = (raw >> 32) as u32;
    if period == 0 || period > 100_000_000 {
        return Err(ClockError::InvalidPeriod);
    }
    Ok((period, ((raw >> 8) & 31) as usize + 1))
}

fn elapsed_ns(initial: u64, counter: u64, period: u32) -> Result<u64, ClockError> {
    let ticks = counter
        .checked_sub(initial)
        .ok_or(ClockError::CounterWrapped)?;
    let nanos = ticks as u128 * period as u128 / 1_000_000;
    u64::try_from(nanos).map_err(|_| ClockError::Exhausted)
}

unsafe fn restore(pointer: usize, saved: &Clock) {
    unsafe { write(pointer, GENERAL_CONFIG, saved.general & !(ENABLED | LEGACY)) };
    for (index, config) in saved
        .timers
        .iter()
        .copied()
        .enumerate()
        .take(saved.timer_count)
    {
        unsafe { write(pointer, TIMER_CONFIG + index * TIMER_STRIDE, config) };
    }
    unsafe { write(pointer, GENERAL_CONFIG, saved.general) };
}

unsafe fn read(pointer: usize, offset: usize) -> u64 {
    unsafe { ((pointer + offset) as *const u64).read_volatile() }
}

unsafe fn write(pointer: usize, offset: usize, value: u64) {
    unsafe { ((pointer + offset) as *mut u64).write_volatile(value) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_requires_64bit_counter_and_valid_declared_period() {
        let caps = (10_000_000u64 << 32) | (2 << 8) | (1 << 13) | 1;
        assert_eq!(decode_capabilities(caps), Ok((10_000_000, 3)));
        assert_eq!(
            decode_capabilities(caps & !(1 << 13)),
            Err(ClockError::UnsupportedCounter)
        );
        assert_eq!(
            decode_capabilities(caps & !0xff),
            Err(ClockError::UnsupportedCounter)
        );
        assert_eq!(
            decode_capabilities(caps & u32::MAX as u64),
            Err(ClockError::InvalidPeriod)
        );
        assert_eq!(
            decode_capabilities((100_000_001u64 << 32) | (1 << 13) | 1),
            Err(ClockError::InvalidPeriod)
        );
        assert_eq!(
            decode_capabilities((1u64 << 32) | (31 << 8) | (1 << 13) | 1),
            Ok((1, 32))
        );
    }

    #[test]
    fn counter_conversion_checks_wrap_and_overflow() {
        assert_eq!(elapsed_ns(100, 112, 41_666_667), Ok(500));
        assert_eq!(elapsed_ns(100, 100, 100_000_000), Ok(0));
        assert_eq!(
            elapsed_ns(100, 99, 100_000_000),
            Err(ClockError::CounterWrapped)
        );
        assert_eq!(
            elapsed_ns(0, u64::MAX, 100_000_000),
            Err(ClockError::Exhausted)
        );
        assert_eq!(elapsed_ns(0, u64::MAX, 1), Ok(u64::MAX / 1_000_000));
    }

    #[test]
    fn clock_mapping_requires_a_full_aligned_kernel_page() {
        assert_eq!(validate_address(0xffff_ff00_0000_c000), Ok(()));
        assert_eq!(validate_address(0xffff_ffff_ffff_f000), Ok(()));
        assert_eq!(validate_address(0x1000), Err(ClockError::InvalidAddress));
        assert_eq!(
            validate_address(0xffff_ff00_0000_c001),
            Err(ClockError::InvalidAddress)
        );
    }

    #[test]
    fn restoring_configuration_retains_the_running_counter_value() {
        let mut registers = [0u64; 512];
        registers[MAIN_COUNTER / 8] = 123_456;
        let pointer = registers.as_mut_ptr() as usize;
        let mut timers = [0; 32];
        timers[0] = TIMER_IRQ | TIMER_FSB | (1 << 7);
        timers[1] = 1 << 4;
        let saved = Clock {
            general: LEGACY | ENABLED | (1 << 8),
            timers,
            timer_count: 2,
        };
        // This RAM-backed register model covers the cleanup ordering's retained
        // values; hardware progress and interrupt delivery need guest coverage.
        unsafe { restore(pointer, &saved) };
        assert_eq!(registers[GENERAL_CONFIG / 8], saved.general);
        assert_eq!(registers[TIMER_CONFIG / 8], saved.timers[0]);
        assert_eq!(
            registers[(TIMER_CONFIG + TIMER_STRIDE) / 8],
            saved.timers[1]
        );
        assert_eq!(registers[MAIN_COUNTER / 8], 123_456);
    }
}
