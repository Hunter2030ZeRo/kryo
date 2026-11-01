use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const COUNTER: u64 = 0xf0;
const CONFIG: u64 = 0x120;
const COMPARATOR: u64 = 0x128;
const ROUTE: usize = 0x130;
const STATUS: usize = 0x20;
const IRQ: u64 = 1 << 2;
const PERIODIC: u64 = 1 << 3;
const SIZE_CAP: u64 = 1 << 5;
const SET_VALUE: u64 = 1 << 6;
const MODE_32: u64 = 1 << 8;
const FSB: u64 = 1 << 14;
const FSB_CAP: u64 = 1 << 15;
const FIXED_CONFIG: u32 = (MODE_32 | FSB) as u32;
const CONFIG_WRITABLE: u64 = (1 << 1) | IRQ | PERIODIC | SET_VALUE | MODE_32 | (31 << 9) | FSB;
const VECTOR: u8 = 0xd0;

static CLAIMED: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicBool = AtomicBool::new(false);
static IRQ_EPOCH: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "boot-tests")]
static FAIL_CLOSE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    InterruptsEnabled,
    UnsupportedCpu,
    NotInitialized,
    UnsupportedTimer,
    Busy,
    InvalidOffset,
    InvalidValue,
    Closed,
    ControllerUnavailable,
    DrainPending,
    EpochExhausted,
}

#[derive(Clone, Copy)]
struct SavedTimer {
    config: u64,
    comparator: u64,
    route: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Active,
    Retiring,
    Closed,
}

/// Exclusive comparator-1 authority. Dropping it retains the claim and clock
/// mapping; the owner must explicitly close it before releasing either mapping.
#[must_use = "retain the timer owner until explicit close succeeds"]
pub struct TimerDevice {
    pointer: usize,
    saved: SavedTimer,
    route: u64,
    state: State,
}

/// # Safety
/// Call on the BSP with IRQs disabled, all user leases retired and no object or
/// allocator locks held. The clock and BSP LAPIC mappings must remain retained
/// until this owner closes. Vector D0 must have its acknowledge handler installed
/// and must belong exclusively to this comparator. No firmware or other driver
/// may modify this HPET block while it is owned by the clock and this device.
pub unsafe fn claim_timer_device() -> Result<TimerDevice, DeviceError> {
    let pointer = device_pointer()?;
    if device_claimed() {
        return Err(DeviceError::Busy);
    }
    if device_irq_epoch() == u64::MAX {
        return Err(DeviceError::EpochExhausted);
    }
    let caps = unsafe { read64(pointer, 0) };
    if (caps >> 8) & 31 == 0 {
        return Err(DeviceError::UnsupportedTimer);
    }
    let config = unsafe { read64(pointer, CONFIG as usize) };
    let status = unsafe { read64(pointer, STATUS) };
    validate_timer(caps, config, status)?;
    drain_irq()?;
    let bsp = super::cpu::cpu_by_index(0).ok_or(DeviceError::UnsupportedCpu)?;
    let route = fsb_route(super::cpu::cpu_apic_id(bsp))?;
    let saved = SavedTimer {
        config,
        comparator: unsafe { read64(pointer, COMPARATOR as usize) },
        route: unsafe { read64(pointer, ROUTE) },
    };
    CLAIMED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| DeviceError::Busy)?;
    PENDING.store(false, Ordering::Release);
    unsafe {
        write64(pointer, CONFIG as usize, u64::from(FIXED_CONFIG));
        write64(pointer, ROUTE, route);
    }
    Ok(TimerDevice {
        pointer,
        saved,
        route,
        state: State::Active,
    })
}

impl TimerDevice {
    /// Accesses exactly one allowed 32-bit register, with no user-selected width.
    pub fn read32(&self, offset: u64) -> Result<u32, DeviceError> {
        validate_read(offset)?;
        self.active_pointer()?;
        Ok(unsafe { read32(self.pointer, offset as usize) })
    }

    /// Accepts only counter-independent comparator writes and the fixed
    /// edge-triggered, non-periodic FSB configuration, optionally enabling IRQs.
    pub fn write32(&mut self, offset: u64, value: u32) -> Result<(), DeviceError> {
        validate_write(offset, value)?;
        self.active_pointer()?;
        if offset == CONFIG {
            if device_irq_epoch() == u64::MAX {
                return Err(DeviceError::EpochExhausted);
            }
            if unsafe { read64(self.pointer, ROUTE) } != self.route {
                self.state = State::Retiring;
                return Err(DeviceError::Busy);
            }
            unsafe { write32(self.pointer, CONFIG as usize, value) };
            let actual = unsafe { read64(self.pointer, CONFIG as usize) };
            if actual & CONFIG_WRITABLE != u64::from(value) {
                self.state = State::Retiring;
                return Err(DeviceError::Busy);
            }
        } else {
            unsafe { write32(self.pointer, COMPARATOR as usize, value) };
        }
        Ok(())
    }

    /// Kernel-side timing metadata for constructing the driver's bootstrap data.
    pub fn period_fs(&self) -> Result<u32, DeviceError> {
        self.active_pointer()?;
        Ok((unsafe { read64(self.pointer, 0) } >> 32) as u32)
    }

    /// Disable the source, drain delivered/queued D0 IRQs, then restore its
    /// disabled state. Failure retains this owner in a retry-only state.
    ///
    /// No allocator or object locks may be held: bounded IRQ windows run the
    /// installed CPL0 acknowledge handlers. The kernel clock keeps running.
    pub fn close(&mut self) -> Result<(), DeviceError> {
        if self.state == State::Closed {
            return Ok(());
        }
        self.check_pointer()?;
        self.state = State::Retiring;
        unsafe {
            let config = read64(self.pointer, CONFIG as usize);
            write64(self.pointer, CONFIG as usize, config & !(IRQ | FSB));
        }
        if unsafe { read64(self.pointer, CONFIG as usize) } & (IRQ | FSB) != 0 {
            return Err(DeviceError::Busy);
        }
        unsafe { write64(self.pointer, STATUS, 1 << 1) };
        if unsafe { read64(self.pointer, STATUS) } & (1 << 1) != 0 {
            return Err(DeviceError::Busy);
        }
        #[cfg(feature = "boot-tests")]
        if FAIL_CLOSE.swap(false, Ordering::AcqRel) {
            return Err(DeviceError::DrainPending);
        }
        drain_irq()?;
        unsafe { restore(self.pointer, self.saved) };
        let config = unsafe { read64(self.pointer, CONFIG as usize) };
        if config & CONFIG_WRITABLE != self.saved.config & CONFIG_WRITABLE
            || unsafe { read64(self.pointer, COMPARATOR as usize) } != self.saved.comparator
            || unsafe { read64(self.pointer, ROUTE) } != self.saved.route
        {
            return Err(DeviceError::Busy);
        }
        PENDING.store(false, Ordering::Release);
        self.state = State::Closed;
        CLAIMED.store(false, Ordering::Release);
        Ok(())
    }

    fn check_pointer(&self) -> Result<(), DeviceError> {
        if device_pointer()? != self.pointer || !device_claimed() {
            return Err(DeviceError::NotInitialized);
        }
        Ok(())
    }

    fn active_pointer(&self) -> Result<(), DeviceError> {
        if self.state != State::Active {
            return Err(DeviceError::Closed);
        }
        self.check_pointer()
    }
}

pub fn device_irq_epoch() -> u64 {
    IRQ_EPOCH.load(Ordering::Acquire)
}

pub fn device_irq_pending() -> bool {
    PENDING.load(Ordering::Acquire)
}

/// Consume a coalesced IRQ only after user execution has returned to BSP policy.
/// The returned epoch counts actual D0 deliveries, not comparator expirations.
pub fn take_device_irq() -> Result<Option<u64>, DeviceError> {
    device_pointer()?;
    if !device_claimed() {
        return Err(DeviceError::NotInitialized);
    }
    let epoch = device_irq_epoch();
    if epoch == u64::MAX {
        return Err(DeviceError::EpochExhausted);
    }
    Ok(PENDING.swap(false, Ordering::AcqRel).then_some(epoch))
}

/// Inject one close failure after disabling the source and before retiring D0.
///
/// # Safety
/// Call on the BSP with IRQs disabled and all user execution retired. Retain the
/// timer owner and both controller mappings until the subsequent close retry
/// succeeds. This changes only boot-test failure policy, not hardware state.
#[cfg(feature = "boot-tests")]
pub unsafe fn fail_next_device_close() -> Result<(), DeviceError> {
    device_pointer()?;
    if !device_claimed() {
        return Err(DeviceError::NotInitialized);
    }
    FAIL_CLOSE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| DeviceError::Busy)
}

/// Observe a queued D0 vector with IRQs disabled, before a close-drain check.
#[cfg(feature = "boot-tests")]
pub fn device_irq_queued() -> Result<bool, DeviceError> {
    device_pointer()?;
    let (pending, in_service) =
        super::apic::vector_state(VECTOR).map_err(|_| DeviceError::ControllerUnavailable)?;
    if in_service {
        return Err(DeviceError::DrainPending);
    }
    Ok(pending)
}

pub(crate) fn device_claimed() -> bool {
    CLAIMED.load(Ordering::Acquire)
}

pub(crate) fn record_interrupt() {
    let _ = IRQ_EPOCH.try_update(Ordering::Release, Ordering::Relaxed, |epoch| {
        epoch.checked_add(1)
    });
    if device_claimed() {
        PENDING.store(true, Ordering::Release);
    }
}

fn device_pointer() -> Result<usize, DeviceError> {
    super::clock::device_pointer().map_err(|error| match error {
        super::clock::ClockError::InterruptsEnabled => DeviceError::InterruptsEnabled,
        super::clock::ClockError::UnsupportedCpu => DeviceError::UnsupportedCpu,
        super::clock::ClockError::Busy => DeviceError::Busy,
        _ => DeviceError::NotInitialized,
    })
}

fn validate_timer(caps: u64, config: u64, status: u64) -> Result<(), DeviceError> {
    if (caps >> 8) & 31 == 0 || config & (SIZE_CAP | FSB_CAP) != SIZE_CAP | FSB_CAP {
        return Err(DeviceError::UnsupportedTimer);
    }
    // A periodic timer's unreadable accumulator cannot be saved and restored.
    if config & (IRQ | PERIODIC | SET_VALUE) != 0 || status & (1 << 1) != 0 {
        return Err(DeviceError::Busy);
    }
    Ok(())
}

fn validate_read(offset: u64) -> Result<(), DeviceError> {
    if matches!(offset, COUNTER | CONFIG | COMPARATOR) {
        Ok(())
    } else {
        Err(DeviceError::InvalidOffset)
    }
}

fn validate_write(offset: u64, value: u32) -> Result<(), DeviceError> {
    match offset {
        CONFIG if value == FIXED_CONFIG || value == FIXED_CONFIG | IRQ as u32 => Ok(()),
        CONFIG => Err(DeviceError::InvalidValue),
        COMPARATOR => Ok(()),
        _ => Err(DeviceError::InvalidOffset),
    }
}

fn fsb_route(apic_id: u32) -> Result<u64, DeviceError> {
    if apic_id >= 255 {
        return Err(DeviceError::UnsupportedCpu);
    }
    let address = 0xfee0_0000 | (apic_id << 12);
    Ok((u64::from(address) << 32) | u64::from(VECTOR))
}

fn drain_irq() -> Result<(), DeviceError> {
    for _ in 0..64 {
        let (pending, in_service) =
            super::apic::vector_state(VECTOR).map_err(|_| DeviceError::ControllerUnavailable)?;
        if in_service {
            return Err(DeviceError::DrainPending);
        }
        if !pending {
            return Ok(());
        }
        unsafe { asm!("sti", "nop", "cli", options(nostack)) };
    }
    Err(DeviceError::DrainPending)
}

unsafe fn restore(pointer: usize, saved: SavedTimer) {
    unsafe {
        write64(pointer, CONFIG as usize, saved.config & !(IRQ | FSB));
        write64(pointer, COMPARATOR as usize, saved.comparator);
        write64(pointer, ROUTE, saved.route);
        write64(pointer, CONFIG as usize, saved.config);
    }
}

unsafe fn read32(pointer: usize, offset: usize) -> u32 {
    unsafe { ((pointer + offset) as *const u32).read_volatile() }
}

unsafe fn write32(pointer: usize, offset: usize, value: u32) {
    unsafe { ((pointer + offset) as *mut u32).write_volatile(value) };
}

unsafe fn read64(pointer: usize, offset: usize) -> u64 {
    unsafe { ((pointer + offset) as *const u64).read_volatile() }
}

unsafe fn write64(pointer: usize, offset: usize, value: u64) {
    unsafe { ((pointer + offset) as *mut u64).write_volatile(value) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_authority_has_exact_offsets_and_values() {
        for offset in [COUNTER, CONFIG, COMPARATOR] {
            assert_eq!(validate_read(offset), Ok(()));
        }
        for offset in [0, 0x10, 0x20, 0xf4, 0x121, 0x124, 0x12c, 0x130, u64::MAX] {
            assert_eq!(validate_read(offset), Err(DeviceError::InvalidOffset));
            assert_eq!(validate_write(offset, 0), Err(DeviceError::InvalidOffset));
        }
        assert_eq!(validate_write(COUNTER, 1), Err(DeviceError::InvalidOffset));
        assert_eq!(validate_write(CONFIG, FIXED_CONFIG), Ok(()));
        assert_eq!(validate_write(CONFIG, FIXED_CONFIG | IRQ as u32), Ok(()));
        for bit in [0, 1, 3, 6, 9, 15, 31] {
            assert_eq!(
                validate_write(CONFIG, FIXED_CONFIG | (1 << bit)),
                Err(DeviceError::InvalidValue)
            );
        }
        assert_eq!(validate_write(CONFIG, 0), Err(DeviceError::InvalidValue));
        assert_eq!(validate_write(COMPARATOR, u32::MAX), Ok(()));
    }

    #[test]
    fn claim_requires_second_64bit_fsb_timer_and_no_active_source() {
        let caps = 1 << 8;
        let config = SIZE_CAP | FSB_CAP;
        assert_eq!(validate_timer(caps, config, 0), Ok(()));
        for bad in [config & !SIZE_CAP, config & !FSB_CAP] {
            assert_eq!(
                validate_timer(caps, bad, 0),
                Err(DeviceError::UnsupportedTimer)
            );
        }
        assert_eq!(
            validate_timer(0, config, 0),
            Err(DeviceError::UnsupportedTimer)
        );
        for bit in [IRQ, PERIODIC, SET_VALUE] {
            assert_eq!(
                validate_timer(caps, config | bit, 0),
                Err(DeviceError::Busy)
            );
        }
        assert_eq!(validate_timer(caps, config, 2), Err(DeviceError::Busy));
    }

    #[test]
    fn route_is_a_fixed_bsp_destination_and_device_vector() {
        assert_eq!(fsb_route(0), Ok(0xfee0_0000_0000_00d0));
        assert_eq!(fsb_route(63), Ok(0xfee3_f000_0000_00d0));
        assert_eq!(fsb_route(254), Ok(0xfeef_e000_0000_00d0));
        assert_eq!(fsb_route(255), Err(DeviceError::UnsupportedCpu));
        assert_eq!(fsb_route(u32::MAX), Err(DeviceError::UnsupportedCpu));
    }

    #[test]
    fn restoration_keeps_clock_and_saved_retirement_metadata() {
        let mut registers = [0u64; 512];
        registers[0x10 / 8] = 1;
        registers[COUNTER as usize / 8] = 12345;
        let saved = SavedTimer {
            config: SIZE_CAP | FSB_CAP,
            comparator: 0x1234_5678_9876_5432,
            route: 0xfee0_0000_0000_0070,
        };
        unsafe { restore(registers.as_mut_ptr() as usize, saved) };
        assert_eq!(registers[CONFIG as usize / 8], saved.config);
        assert_eq!(registers[COMPARATOR as usize / 8], saved.comparator);
        assert_eq!(registers[ROUTE / 8], saved.route);
        assert_eq!(registers[0x10 / 8], 1);
        assert_eq!(registers[COUNTER as usize / 8], 12345);
        let owner = TimerDevice {
            pointer: registers.as_mut_ptr() as usize,
            saved,
            route: 0,
            state: State::Retiring,
        };
        assert_eq!(owner.read32(COUNTER), Err(DeviceError::Closed));
        assert_eq!(owner.saved.comparator, saved.comparator);
    }
}
