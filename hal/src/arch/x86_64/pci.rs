use alloc::vec::Vec;
use spin::Mutex;
use x86_64::instructions::{interrupts, port::Port};

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PciError {
    InvalidAddress,
    InvalidOffset,
    Capacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PciAddress(u16);

impl PciAddress {
    pub const fn new(bus: u8, device: u8, function: u8) -> Result<Self, PciError> {
        if device >= 32 || function >= 8 {
            return Err(PciError::InvalidAddress);
        }
        Ok(Self(
            ((bus as u16) << 8) | ((device as u16) << 3) | function as u16,
        ))
    }

    pub const fn bus(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn device(self) -> u8 {
        ((self.0 >> 3) & 31) as u8
    }

    pub const fn function(self) -> u8 {
        (self.0 & 7) as u8
    }

    pub const fn as_raw(self) -> u16 {
        self.0
    }

    pub const fn configuration_address(self, offset: u16) -> Result<u32, PciError> {
        if offset > 252 || !offset.is_multiple_of(4) {
            return Err(PciError::InvalidOffset);
        }
        Ok(0x8000_0000 | ((self.0 as u32) << 8) | offset as u32)
    }
}

/// A configuration snapshot. BAR values describe firmware assignments; they
/// provide neither a size nor authority to access or map the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciFunction {
    pub address: PciAddress,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub interface: u8,
    pub revision: u8,
    pub header: u8,
    pub command: u16,
    pub bars: [u32; 6],
    pub bar_count: u8,
}

/// Reads conventional, aligned configuration dwords without changing device
/// configuration. A reader may expose emulated configuration for host checks.
pub trait PciConfigRead {
    fn read32(&mut self, address: PciAddress, offset: u16) -> Result<u32, PciError>;
}

pub fn scan_pci_with(reader: &mut impl PciConfigRead) -> Result<Vec<PciFunction>, PciError> {
    let mut functions = Vec::new();
    // Scanning the complete conventional segment also finds functions behind
    // configured bridges without trusting their secondary/subordinate fields.
    for bus in 0..=u8::MAX {
        for device in 0..32 {
            let address = PciAddress::new(bus, device, 0)?;
            let Some(first) = snapshot(reader, address)? else {
                continue;
            };
            let multifunction = first.header & 0x80 != 0;
            push(&mut functions, first)?;
            if multifunction {
                for function in 1..8 {
                    if let Some(found) = snapshot(reader, PciAddress::new(bus, device, function)?)?
                    {
                        push(&mut functions, found)?;
                    }
                }
            }
        }
    }
    Ok(functions)
}

/// Reads segment zero through x86 configuration mechanism 1. This does not
/// enable decoding, bus mastering, interrupts, or device queues.
///
/// # Safety
/// Run at I/O privilege on a platform that implements CF8/CFC configuration
/// mechanism 1. All access to these ports must use this module's lock, and NMI
/// handlers must not use this path. Firmware must not access the ports
/// concurrently. Hotplug and removal must be excluded while taking a snapshot.
pub unsafe fn scan_pci() -> Result<Vec<PciFunction>, PciError> {
    scan_pci_with(&mut PortConfig)
}

/// # Safety
/// The platform and synchronization requirements of `scan_pci` apply.
pub unsafe fn read_pci_config(address: PciAddress, offset: u16) -> Result<u32, PciError> {
    PortConfig.read32(address, offset)
}

struct PortConfig;

impl PciConfigRead for PortConfig {
    fn read32(&mut self, address: PciAddress, offset: u16) -> Result<u32, PciError> {
        let selector = address.configuration_address(offset)?;
        Ok(interrupts::without_interrupts(|| {
            let _guard = CONFIG_LOCK.lock();
            let mut address_port = Port::<u32>::new(0xcf8);
            let mut data_port = Port::<u32>::new(0xcfc);
            // The private reader can only be reached through privileged unsafe
            // entry points. IRQ exclusion prevents re-entry on this CPU; the
            // lock serializes the selector/data pair against other CPUs.
            unsafe {
                let saved = address_port.read();
                address_port.write(selector);
                let value = data_port.read();
                address_port.write(saved);
                value
            }
        }))
    }
}

fn snapshot(
    reader: &mut impl PciConfigRead,
    address: PciAddress,
) -> Result<Option<PciFunction>, PciError> {
    let identity = reader.read32(address, 0)?;
    if identity as u16 == u16::MAX {
        return Ok(None);
    }
    let class = reader.read32(address, 8)?;
    let header = (reader.read32(address, 12)? >> 16) as u8;
    let bar_count = match header & 0x7f {
        0 => 6,
        1 => 2,
        _ => 0,
    };
    let mut bars = [0; 6];
    for (index, bar) in bars[..bar_count].iter_mut().enumerate() {
        *bar = reader.read32(address, 0x10 + index as u16 * 4)?;
    }
    Ok(Some(PciFunction {
        address,
        vendor_id: identity as u16,
        device_id: (identity >> 16) as u16,
        class: (class >> 24) as u8,
        subclass: (class >> 16) as u8,
        interface: (class >> 8) as u8,
        revision: class as u8,
        header,
        command: reader.read32(address, 4)? as u16,
        bars,
        bar_count: bar_count as u8,
    }))
}

fn push(functions: &mut Vec<PciFunction>, function: PciFunction) -> Result<(), PciError> {
    functions.try_reserve(1).map_err(|_| PciError::Capacity)?;
    functions.push(function);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    #[derive(Default)]
    struct Config {
        registers: BTreeMap<(u16, u16), u32>,
        reads: Vec<(u16, u16)>,
        fail: Option<(u16, u16)>,
    }

    impl Config {
        fn add(&mut self, address: PciAddress, header: u8) {
            let base = address.as_raw();
            for offset in (0..=252).step_by(4) {
                self.registers.insert((base, offset), 0);
            }
            self.registers.insert((base, 0), 0x1042_1af4);
            self.registers.insert((base, 4), 0x0010_0007);
            self.registers.insert((base, 8), 0x0100_0102);
            self.registers.insert((base, 12), u32::from(header) << 16);
        }
    }

    impl PciConfigRead for Config {
        fn read32(&mut self, address: PciAddress, offset: u16) -> Result<u32, PciError> {
            address.configuration_address(offset)?;
            let key = (address.as_raw(), offset);
            self.reads.push(key);
            if self.fail == Some(key) {
                return Err(PciError::Capacity);
            }
            Ok(self.registers.get(&key).copied().unwrap_or(u32::MAX))
        }
    }

    #[test]
    fn address_and_offsets_cannot_escape_conventional_configuration() {
        let address = PciAddress::new(255, 31, 7).unwrap();
        assert_eq!(address.as_raw(), u16::MAX);
        assert_eq!(
            (address.bus(), address.device(), address.function()),
            (255, 31, 7)
        );
        assert_eq!(address.configuration_address(252), Ok(0x80ff_fffc));
        assert_eq!(PciAddress::new(0, 32, 0), Err(PciError::InvalidAddress));
        assert_eq!(PciAddress::new(0, 0, 8), Err(PciError::InvalidAddress));
        for offset in [1, 2, 3, 253, 254, 255, 256, 4092, u16::MAX] {
            assert_eq!(
                address.configuration_address(offset),
                Err(PciError::InvalidOffset)
            );
        }
    }

    #[test]
    fn empty_segment_only_reads_function_zero_identity() {
        let mut config = Config::default();
        assert!(scan_pci_with(&mut config).unwrap().is_empty());
        assert_eq!(config.reads.len(), 256 * 32);
        assert!(
            config
                .reads
                .iter()
                .all(|(address, offset)| address & 7 == 0 && *offset == 0)
        );
    }

    #[test]
    fn multifunction_and_nonzero_bus_inventory_preserves_raw_registers() {
        let first = PciAddress::new(0, 2, 0).unwrap();
        let other = PciAddress::new(0, 2, 7).unwrap();
        let behind_bridge = PciAddress::new(201, 31, 0).unwrap();
        let mut config = Config::default();
        config.add(first, 0x80);
        config.add(other, 0);
        config.add(behind_bridge, 1);
        config.registers.insert((first.as_raw(), 0x10), 0x1234_000c);
        config.registers.insert((first.as_raw(), 0x14), 0x0000_0008);
        let before = config.registers.clone();
        let functions = scan_pci_with(&mut config).unwrap();
        assert_eq!(functions.len(), 3);
        assert_eq!(
            functions.iter().map(|f| f.address).collect::<Vec<_>>(),
            [first, other, behind_bridge]
        );
        let found = functions[0];
        assert_eq!((found.vendor_id, found.device_id), (0x1af4, 0x1042));
        assert_eq!(
            (found.class, found.subclass, found.interface, found.revision),
            (1, 0, 1, 2)
        );
        assert_eq!(found.command, 7);
        assert_eq!(found.bars[..2], [0x1234_000c, 8]);
        assert_eq!(functions[2].bar_count, 2);
        assert_eq!(config.registers, before);
        assert!(!config.reads.contains(&(behind_bridge.as_raw(), 0x18)));
    }

    #[test]
    fn single_function_header_does_not_probe_other_functions() {
        let address = PciAddress::new(0, 0, 0).unwrap();
        let hidden = PciAddress::new(0, 0, 1).unwrap();
        let mut config = Config::default();
        config.add(address, 0);
        config.add(hidden, 0);
        assert_eq!(scan_pci_with(&mut config).unwrap().len(), 1);
        assert!(!config.reads.iter().any(|(raw, _)| *raw == hidden.as_raw()));
    }

    #[test]
    fn unknown_header_never_reads_possible_nonbar_registers() {
        let address = PciAddress::new(0, 1, 0).unwrap();
        let mut config = Config::default();
        config.add(address, 2);
        let functions = scan_pci_with(&mut config).unwrap();
        assert_eq!(functions[0].bar_count, 0);
        assert_eq!(functions[0].bars, [0; 6]);
        assert!(
            !config
                .reads
                .iter()
                .any(|(raw, offset)| *raw == address.as_raw() && *offset >= 0x10)
        );
    }

    #[test]
    fn config_failure_returns_error_without_partial_inventory() {
        let address = PciAddress::new(0, 0, 0).unwrap();
        let mut config = Config::default();
        config.add(address, 0);
        config.fail = Some((address.as_raw(), 8));
        assert_eq!(scan_pci_with(&mut config), Err(PciError::Capacity));
    }
}
