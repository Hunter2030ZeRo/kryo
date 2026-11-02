use core::sync::atomic::{AtomicBool, Ordering};

use super::pci::{ConfigAccess, PciAddress, PciError, PciFunction, with_pci_config};

const VERSION_1: u64 = 1 << 32;
const DEVICE_FEATURES: u64 = (1 << 24) - 1;
const INTX_DISABLE: u16 = 1 << 10;
const BUS_MASTER: u16 = 1 << 2;
const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const FEATURES_OK: u8 = 8;

static CLAIMED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtioError {
    Pci(PciError),
    UnsupportedDevice,
    Busy,
    InvalidCapability,
    MissingCapability,
    DuplicateCapability,
    InterruptsEnabled,
    InvalidAccess,
    InvalidFeatures,
    InvalidQueue,
    InvalidState,
    ResetPending,
    FeaturesRejected,
    DeviceFailure,
    ConfigurationChanged,
    Closed,
}

impl From<PciError> for VirtioError {
    fn from(error: PciError) -> Self {
        Self::Pci(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtioQueue {
    pub size: u16,
    pub descriptors: u64,
    pub available: u64,
    pub used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Region {
    bar: u8,
    offset: u32,
    length: u32,
}

impl Region {
    fn address(self, offset: u32, width: u32) -> Result<u32, VirtioError> {
        if !matches!(width, 1 | 2 | 4)
            || !offset.is_multiple_of(width)
            || u64::from(offset) + u64::from(width) > u64::from(self.length)
        {
            return Err(VirtioError::InvalidAccess);
        }
        self.offset
            .checked_add(offset)
            .filter(|address| address.is_multiple_of(width))
            .ok_or(VirtioError::InvalidAccess)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Capabilities {
    common: Region,
    notify: Region,
    isr: Region,
    device: Region,
    window: u16,
    multiplier: u32,
}

/// A modern PCI transport using the mandatory PCI configuration window.
/// No raw register, BAR, or DMA-address programming interface is exposed to users.
/// Abandoning this owner retains its global claim; only explicit close releases it.
pub struct VirtioPci {
    address: PciAddress,
    caps: Capabilities,
    command: u16,
    resetting: bool,
    reset_complete: bool,
    prepared: bool,
    features_ok: bool,
    running: bool,
    closed: bool,
    notify_offset: u32,
}

impl VirtioPci {
    /// # Safety
    /// The caller exclusively owns this function on a CF8/CFC PC platform, and
    /// excludes firmware, hotplug and other drivers. Existing device DMA owners
    /// must stay retained until reset completes. No NMI may use PCI config I/O.
    pub unsafe fn new(function: PciFunction) -> Result<Self, VirtioError> {
        if function.vendor_id != 0x1af4
            || !(0x1040..=0x107f).contains(&function.device_id)
            || function.header & 0x7f != 0
        {
            return Err(VirtioError::UnsupportedDevice);
        }
        CLAIMED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VirtioError::Busy)?;
        let prepared = unsafe {
            with_pci_config(function.address, |config| {
                if config.read32(0)?
                    != (u32::from(function.device_id) << 16) | u32::from(function.vendor_id)
                {
                    return Err(VirtioError::UnsupportedDevice);
                }
                let caps = capabilities(|offset| config.read32(offset).map_err(Into::into))?;
                let command = config.read16(4)?;
                // A word write avoids PCI status W1C bits in the upper half.
                config.write16(4, command | INTX_DISABLE)?;
                Ok((caps, command))
            })
        };
        match prepared {
            Ok((caps, command)) => Ok(Self {
                address: function.address,
                caps,
                command,
                resetting: false,
                reset_complete: false,
                prepared: false,
                features_ok: false,
                running: false,
                closed: false,
                notify_offset: 0,
            }),
            Err(error) => {
                CLAIMED.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    /// Initiates reset once, then polls its status without spinning. A false
    /// result retains every device/queue/buffer owner for a later call.
    pub fn reset(&mut self) -> Result<bool, VirtioError> {
        self.check_open()?;
        if self.reset_complete {
            return Ok(true);
        }
        self.running = false;
        self.prepared = false;
        self.features_ok = false;
        if !self.resetting {
            self.write(self.caps.common, 20, 1, 0)?;
            self.resetting = true;
        }
        if self.read(self.caps.common, 20, 1)? != 0 {
            return Ok(false);
        }
        self.resetting = false;
        self.reset_complete = true;
        Ok(true)
    }

    /// Acknowledges the reset device before feature discovery. Failed writes
    /// leave reset required; this never enables queues or device DMA.
    pub fn prepare(&mut self) -> Result<(), VirtioError> {
        self.check_open()?;
        if !self.reset_complete || self.prepared || self.running {
            return Err(VirtioError::InvalidState);
        }
        self.reset_complete = false;
        self.write(self.caps.common, 20, 1, u32::from(ACKNOWLEDGE))?;
        self.write(self.caps.common, 20, 1, u32::from(ACKNOWLEDGE | DRIVER))?;
        self.prepared = true;
        Ok(())
    }

    pub fn features(&mut self) -> Result<u64, VirtioError> {
        self.check_open()?;
        if !self.prepared && !self.running {
            return Err(VirtioError::InvalidState);
        }
        self.write(self.caps.common, 0, 4, 0)?;
        let low = self.read(self.caps.common, 4, 4)?;
        self.write(self.caps.common, 0, 4, 1)?;
        let high = self.read(self.caps.common, 4, 4)?;
        Ok(u64::from(low) | (u64::from(high) << 32))
    }

    pub fn queue_size(&mut self) -> Result<u16, VirtioError> {
        self.check_open()?;
        if !self.features_ok && !self.running {
            return Err(VirtioError::InvalidState);
        }
        self.write(self.caps.common, 22, 2, 0)?;
        Ok(self.read(self.caps.common, 24, 2)? as u16)
    }

    pub fn read_config64(&mut self, offset: u32) -> Result<u64, VirtioError> {
        self.check_open()?;
        if !self.features_ok && !self.running {
            return Err(VirtioError::InvalidState);
        }
        let high_offset = offset.checked_add(4).ok_or(VirtioError::InvalidAccess)?;
        self.caps.device.address(offset, 4)?;
        self.caps.device.address(high_offset, 4)?;
        for _ in 0..3 {
            let before = self.read(self.caps.common, 21, 1)?;
            let low = self.read(self.caps.device, offset, 4)?;
            let high = self.read(self.caps.device, high_offset, 4)?;
            if before == self.read(self.caps.common, 21, 1)? {
                return Ok(u64::from(low) | (u64::from(high) << 32));
            }
        }
        Err(VirtioError::ConfigurationChanged)
    }

    /// # Safety
    /// All three queue regions are initialized, exclusively kernel-owned DMA
    /// storage with the required lengths for this size. They must remain pinned
    /// and inaccessible to untrusted writers until reset reports completion.
    /// The caller must record publication before this can enable device access.
    pub unsafe fn start(
        &mut self,
        features: u64,
        allowed_features: u64,
        queue: VirtioQueue,
    ) -> Result<(), VirtioError> {
        self.check_open()?;
        if !self.prepared || self.features_ok || self.running {
            return Err(VirtioError::InvalidState);
        }
        let offered = self.features();
        // Every failed start requires explicit reset. The caller retains
        // published DMA backing even if DRIVER_OK is never reached.
        self.prepared = false;
        validate_features(features, allowed_features, offered?)?;
        validate_queue(queue, u16::MAX)?;
        self.write(self.caps.common, 8, 4, 0)?;
        self.write(self.caps.common, 12, 4, features as u32)?;
        self.write(self.caps.common, 8, 4, 1)?;
        self.write(self.caps.common, 12, 4, (features >> 32) as u32)?;
        let status = ACKNOWLEDGE | DRIVER | FEATURES_OK;
        self.write(self.caps.common, 20, 1, u32::from(status))?;
        validate_status(self.read(self.caps.common, 20, 1)? as u8, status)?;
        self.features_ok = true;
        validate_queue(queue, self.queue_size()?)?;
        let notify = self.read(self.caps.common, 30, 2)?;
        let notify_offset = notify
            .checked_mul(self.caps.multiplier)
            .ok_or(VirtioError::InvalidQueue)?;
        self.caps.notify.address(notify_offset, 2)?;
        self.write(self.caps.common, 16, 2, u32::from(u16::MAX))?;
        self.write(self.caps.common, 22, 2, 0)?;
        self.write(self.caps.common, 24, 2, u32::from(queue.size))?;
        self.write(self.caps.common, 26, 2, u32::from(u16::MAX))?;
        self.write64(self.caps.common, 32, queue.descriptors)?;
        self.write64(self.caps.common, 40, queue.available)?;
        self.write64(self.caps.common, 48, queue.used)?;
        self.write(self.caps.common, 28, 2, 1)?;
        unsafe {
            with_pci_config(self.address, |config| {
                config.write16(4, self.command | INTX_DISABLE | BUS_MASTER)
            })
        }?;
        self.write(self.caps.common, 20, 1, u32::from(status | DRIVER_OK))?;
        validate_status(
            self.read(self.caps.common, 20, 1)? as u8,
            status | DRIVER_OK,
        )?;
        self.notify_offset = notify_offset;
        self.running = true;
        Ok(())
    }

    pub fn notify(&mut self) -> Result<(), VirtioError> {
        self.check_open()?;
        if !self.running {
            return Err(VirtioError::InvalidState);
        }
        self.write(self.caps.notify, self.notify_offset, 2, 0)
    }

    /// Reads and acknowledges any coalesced INTx cause; polling callers still
    /// keep INTx disabled and use their private used ring for completion.
    pub fn acknowledge(&mut self) -> Result<u8, VirtioError> {
        self.check_open()?;
        Ok(self.read(self.caps.isr, 0, 1)? as u8)
    }

    /// Restores PCI command state only after reset forbids further queue access.
    /// False means the caller must retain this owner and every DMA owner.
    pub fn close(&mut self) -> Result<bool, VirtioError> {
        if self.closed {
            return Ok(true);
        }
        if !self.reset()? {
            return Ok(false);
        }
        unsafe { with_pci_config(self.address, |config| config.write16(4, self.command)) }?;
        self.closed = true;
        CLAIMED.store(false, Ordering::Release);
        Ok(true)
    }

    fn check_open(&self) -> Result<(), VirtioError> {
        if self.closed {
            Err(VirtioError::Closed)
        } else {
            Ok(())
        }
    }

    fn read(&self, region: Region, offset: u32, width: u32) -> Result<u32, VirtioError> {
        self.check_open()?;
        let absolute = region.address(offset, width)?;
        unsafe {
            with_pci_config(self.address, |config| {
                select_window(config, self.caps.window, region.bar, absolute, width)?;
                let data = self.caps.window + 16;
                match width {
                    1 => config.read8(data).map(u32::from),
                    2 => config.read16(data).map(u32::from),
                    4 => config.read32(data),
                    _ => unreachable!(),
                }
                .map_err(Into::into)
            })
        }
    }

    fn write(
        &self,
        region: Region,
        offset: u32,
        width: u32,
        value: u32,
    ) -> Result<(), VirtioError> {
        self.check_open()?;
        let absolute = region.address(offset, width)?;
        unsafe {
            with_pci_config(self.address, |config| {
                select_window(config, self.caps.window, region.bar, absolute, width)?;
                let data = self.caps.window + 16;
                match width {
                    1 => config.write8(data, value as u8),
                    2 => config.write16(data, value as u16),
                    4 => config.write32(data, value),
                    _ => unreachable!(),
                }
                .map_err(Into::into)
            })
        }
    }

    fn write64(&self, region: Region, offset: u32, value: u64) -> Result<(), VirtioError> {
        self.write(region, offset, 4, value as u32)?;
        self.write(region, offset + 4, 4, (value >> 32) as u32)
    }
}

fn select_window(
    config: &mut ConfigAccess,
    window: u16,
    bar: u8,
    offset: u32,
    width: u32,
) -> Result<(), VirtioError> {
    config.write8(window + 4, bar)?;
    config.write32(window + 8, offset)?;
    config.write32(window + 12, width)?;
    Ok(())
}

fn validate_features(features: u64, allowed: u64, offered: u64) -> Result<(), VirtioError> {
    if features & VERSION_1 == 0
        || features & !(VERSION_1 | DEVICE_FEATURES) != 0
        || features & !allowed != 0
        || features & !offered != 0
    {
        return Err(VirtioError::InvalidFeatures);
    }
    Ok(())
}

fn validate_status(status: u8, required: u8) -> Result<(), VirtioError> {
    if status & (64 | 128) != 0 {
        return Err(VirtioError::DeviceFailure);
    }
    if status & required != required {
        return Err(VirtioError::FeaturesRejected);
    }
    Ok(())
}

fn validate_queue(queue: VirtioQueue, maximum: u16) -> Result<(), VirtioError> {
    if queue.size == 0
        || !queue.size.is_power_of_two()
        || queue.size > maximum
        || queue.descriptors == 0
        || !queue.descriptors.is_multiple_of(16)
        || queue.available == 0
        || !queue.available.is_multiple_of(2)
        || queue.used == 0
        || !queue.used.is_multiple_of(4)
        || queue
            .descriptors
            .checked_add(u64::from(queue.size) * 16)
            .is_none()
        || queue
            .available
            .checked_add(6 + u64::from(queue.size) * 2)
            .is_none()
        || queue
            .used
            .checked_add(6 + u64::from(queue.size) * 8)
            .is_none()
    {
        return Err(VirtioError::InvalidQueue);
    }
    let ranges = [
        (queue.descriptors, u64::from(queue.size) * 16),
        (queue.available, 6 + u64::from(queue.size) * 2),
        (queue.used, 6 + u64::from(queue.size) * 8),
    ];
    for (index, &(start, length)) in ranges.iter().enumerate() {
        for &(other, other_length) in &ranges[..index] {
            if start < other + other_length && other < start + length {
                return Err(VirtioError::InvalidQueue);
            }
        }
    }
    Ok(())
}

fn capabilities(
    mut read: impl FnMut(u16) -> Result<u32, VirtioError>,
) -> Result<Capabilities, VirtioError> {
    if read(4)? & (1 << 20) == 0 {
        return Err(VirtioError::MissingCapability);
    }
    let mut next = (read(0x34)? & 0xff) as u16;
    let mut visited = 0u64;
    let mut regions: [Option<Region>; 4] = [None; 4];
    let mut window = None;
    let mut multiplier = 0;
    while next != 0 {
        if !(0x40..=0xfc).contains(&next) || !next.is_multiple_of(4) {
            return Err(VirtioError::InvalidCapability);
        }
        let bit = 1u64 << (next / 4);
        if visited & bit != 0 {
            return Err(VirtioError::InvalidCapability);
        }
        visited |= bit;
        let header = read(next)?;
        match header as u8 {
            5 if header & (1 << 16) != 0 => return Err(VirtioError::InterruptsEnabled),
            0x11 if header & (1 << 31) != 0 => return Err(VirtioError::InterruptsEnabled),
            9 => {
                let length = ((header >> 16) & 0xff) as u16;
                let kind = (header >> 24) as u8;
                if length < 4 || next + length > 256 {
                    return Err(VirtioError::InvalidCapability);
                }
                if (1..=5).contains(&kind) && length < 16 {
                    return Err(VirtioError::InvalidCapability);
                }
                if kind == 5 {
                    if length < 20 {
                        return Err(VirtioError::InvalidCapability);
                    }
                    if window.replace(next).is_some() {
                        return Err(VirtioError::DuplicateCapability);
                    }
                } else if (1..=4).contains(&kind) {
                    let bar = read(next + 4)? as u8;
                    let offset = read(next + 8)?;
                    let bytes = read(next + 12)?;
                    let alignment = match kind {
                        1 | 4 => 4,
                        2 => 2,
                        _ => 1,
                    };
                    let minimum = match kind {
                        1 => 56,
                        2 => 2,
                        3 => 1,
                        _ => 8,
                    };
                    if bar >= 6
                        || !offset.is_multiple_of(alignment)
                        || bytes < minimum
                        || u64::from(offset) + u64::from(bytes) > 1 << 32
                    {
                        return Err(VirtioError::InvalidCapability);
                    }
                    let region = Region {
                        bar,
                        offset,
                        length: bytes,
                    };
                    if regions[usize::from(kind - 1)].replace(region).is_some() {
                        return Err(VirtioError::DuplicateCapability);
                    }
                    if kind == 2 {
                        if length < 20 {
                            return Err(VirtioError::InvalidCapability);
                        }
                        multiplier = read(next + 16)?;
                    }
                }
            }
            _ => {}
        }
        next = ((header >> 8) & 0xff) as u16;
    }
    Ok(Capabilities {
        common: regions[0].ok_or(VirtioError::MissingCapability)?,
        notify: regions[1].ok_or(VirtioError::MissingCapability)?,
        isr: regions[2].ok_or(VirtioError::MissingCapability)?,
        device: regions[3].ok_or(VirtioError::MissingCapability)?,
        window: window.ok_or(VirtioError::MissingCapability)?,
        multiplier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> [u32; 64] {
        let mut config = [0; 64];
        config[1] = 1 << 20;
        config[0x34 / 4] = 0x40;
        let caps = [
            (0x40, 1, 16, 56),
            (0x50, 2, 20, 4),
            (0x64, 3, 16, 1),
            (0x74, 4, 16, 8),
            (0x84, 5, 20, 0),
        ];
        for (index, &(offset, kind, length, bytes)) in caps.iter().enumerate() {
            let next = caps.get(index + 1).map_or(0, |next| next.0);
            config[offset / 4] =
                9 | ((next as u32) << 8) | ((length as u32) << 16) | ((kind as u32) << 24);
            config[offset / 4 + 1] = 4;
            config[offset / 4 + 2] = (kind as u32 - 1) * 4096;
            config[offset / 4 + 3] = bytes;
        }
        config[0x60 / 4] = 4;
        config
    }

    fn parse(config: &[u32; 64]) -> Result<Capabilities, VirtioError> {
        capabilities(|offset| Ok(config[usize::from(offset) / 4]))
    }

    #[test]
    fn configuration_window_and_regions_are_bounded() {
        let caps = parse(&config()).unwrap();
        assert_eq!(caps.window, 0x84);
        assert_eq!(caps.multiplier, 4);
        assert_eq!(caps.common.address(52, 4), Ok(52));
        assert_eq!(caps.common.address(53, 4), Err(VirtioError::InvalidAccess));
        assert_eq!(caps.common.address(56, 1), Err(VirtioError::InvalidAccess));
        assert_eq!(caps.device.address(4, 4), Ok(3 * 4096 + 4));
    }

    #[test]
    fn initialization_phases_reject_early_register_access() {
        let mut device = VirtioPci {
            address: PciAddress::new(0, 0, 0).unwrap(),
            caps: parse(&config()).unwrap(),
            command: 0,
            resetting: false,
            reset_complete: false,
            prepared: false,
            features_ok: false,
            running: false,
            closed: false,
            notify_offset: 0,
        };
        assert_eq!(device.prepare(), Err(VirtioError::InvalidState));
        for reset_complete in [false, true] {
            device.reset_complete = reset_complete;
            // These must return before privileged I/O, even after reset.
            assert_eq!(device.features(), Err(VirtioError::InvalidState));
            assert_eq!(device.queue_size(), Err(VirtioError::InvalidState));
            assert_eq!(device.read_config64(0), Err(VirtioError::InvalidState));
        }
        device.prepared = true;
        assert_eq!(device.queue_size(), Err(VirtioError::InvalidState));
        assert_eq!(device.read_config64(0), Err(VirtioError::InvalidState));
    }

    #[test]
    fn missing_cyclic_or_misaligned_capabilities_fail() {
        let mut value = config();
        value[1] = 0;
        assert_eq!(parse(&value), Err(VirtioError::MissingCapability));
        let mut value = config();
        value[0x84 / 4] = (value[0x84 / 4] & !0xff00) | (0x40 << 8);
        assert_eq!(parse(&value), Err(VirtioError::InvalidCapability));
        let mut value = config();
        value[0x34 / 4] = 0x41;
        assert_eq!(parse(&value), Err(VirtioError::InvalidCapability));
    }

    #[test]
    fn reserved_vendor_capabilities_are_ignored() {
        let mut value = config();
        value[0x34 / 4] = 0xa0;
        value[0xa0 / 4] = 0x0704_4009;
        assert!(parse(&value).is_ok());
    }

    #[test]
    fn malformed_lengths_bars_and_regions_fail() {
        for (index, invalid) in [
            (0x40 / 4, 0x010f_5009),
            (0x44 / 4, 6),
            (0x48 / 4, 1),
            (0x4c / 4, 55),
            (0x54 / 4, 6),
            (0x58 / 4, u32::MAX),
            (0x5c / 4, 0),
            (0x84 / 4, 0x0513_0009),
        ] {
            let mut value = config();
            value[index] = invalid;
            assert_eq!(parse(&value), Err(VirtioError::InvalidCapability));
        }
    }

    #[test]
    fn duplicate_regions_and_enabled_interrupts_fail() {
        let mut value = config();
        value[0x64 / 4] = 0x0110_7409;
        value[0x70 / 4] = 56;
        assert_eq!(parse(&value), Err(VirtioError::DuplicateCapability));
        for interrupt in [0x0001_0005, 0x8000_0011] {
            let mut value = config();
            value[0x84 / 4] = (value[0x84 / 4] & !0xff00) | (0xa0 << 8);
            value[0xa0 / 4] = interrupt;
            assert_eq!(parse(&value), Err(VirtioError::InterruptsEnabled));
        }
    }

    #[test]
    fn only_selected_device_features_and_split_ring_transport_are_allowed() {
        let allowed = VERSION_1 | (1 << 5) | (1 << 9);
        assert_eq!(validate_features(allowed, allowed, allowed), Ok(()));
        assert_eq!(
            validate_features(1 << 9, allowed, allowed),
            Err(VirtioError::InvalidFeatures)
        );
        for bit in [24, 28, 29, 33, 34, 35, 36, 38] {
            assert_eq!(
                validate_features(VERSION_1 | (1 << bit), u64::MAX, u64::MAX),
                Err(VirtioError::InvalidFeatures)
            );
        }
        assert_eq!(
            validate_features(VERSION_1 | (1 << 8), allowed, u64::MAX),
            Err(VirtioError::InvalidFeatures)
        );
        assert_eq!(
            validate_features(allowed, allowed, VERSION_1),
            Err(VirtioError::InvalidFeatures)
        );
    }

    #[test]
    fn queue_checks_sizes_alignment_and_full_range_overflow() {
        let queue = VirtioQueue {
            size: 8,
            descriptors: 0x1000,
            available: 0x1100,
            used: 0x1200,
        };
        assert_eq!(validate_queue(queue, 256), Ok(()));
        for invalid in [
            VirtioQueue { size: 0, ..queue },
            VirtioQueue { size: 3, ..queue },
            VirtioQueue { size: 512, ..queue },
            VirtioQueue {
                descriptors: 0x1001,
                ..queue
            },
            VirtioQueue {
                available: 0x1101,
                ..queue
            },
            VirtioQueue {
                used: 0x1202,
                ..queue
            },
            VirtioQueue {
                used: u64::MAX - 3,
                ..queue
            },
            VirtioQueue {
                used: queue.descriptors,
                ..queue
            },
        ] {
            assert_eq!(validate_queue(invalid, 256), Err(VirtioError::InvalidQueue));
        }
    }

    #[test]
    fn failed_or_reset_needed_status_never_becomes_ready() {
        let required = ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK;
        assert_eq!(validate_status(required, required), Ok(()));
        assert_eq!(
            validate_status(required & !FEATURES_OK, required),
            Err(VirtioError::FeaturesRejected)
        );
        for failure in [64, 128] {
            assert_eq!(
                validate_status(required | failure, required),
                Err(VirtioError::DeviceFailure)
            );
        }
    }
}
