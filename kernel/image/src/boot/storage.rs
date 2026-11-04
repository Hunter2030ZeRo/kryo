use super::memory::BootFramePool;
use super::vm_backend::{BackendError, KernelMemory};
use alloc::vec::Vec;
use core::cell::RefCell;
use kryo_core::device::dma::{DmaBackend, DmaBuffer, DmaLimits};
use kryo_core::mm::frame::{FRAME_SIZE, PhysRange};
use kryo_core::mm::shared::SharedBackend;
use kryo_hal::DirectMap;
use kryo_hal::PciFunction;
use spin::Once;

static INVENTORY: Once<Vec<PciFunction>> = Once::new();

pub(crate) fn block_device() -> Option<PciFunction> {
    INVENTORY
        .get()?
        .iter()
        .find(|device| device.vendor_id == 0x1af4 && device.device_id == 0x1042)
        .copied()
}

pub(crate) fn discover() {
    assert_eq!(kryo_hal::current_cpu().map(|cpu| cpu.index()), Some(0));
    assert!(!x86_64::instructions::interrupts::are_enabled());
    // This PC bootstrap owns configuration access. No driver or user execution
    // runs here; the scan changes selectors, never PCI device configuration.
    let functions = match INVENTORY.try_call_once(|| unsafe { kryo_hal::scan_pci() }) {
        Ok(functions) => functions,
        Err(error) => {
            let _ = kryo_hal::print(format_args!("PCI: discovery unavailable {:?}\n", error));
            return;
        }
    };
    let mut unchanged = true;
    let mut blocks = 0;
    for device in functions {
        // Same exclusive bootstrap context as the scan; this does not enable
        // decoding or bus mastering, or infer a BAR's length by writing to it.
        let command =
            unsafe { kryo_hal::read_pci_config(device.address, 4) }.map(|value| value as u16);
        unchanged &= command == Ok(device.command);
        blocks +=
            usize::from(device.vendor_id == 0x1af4 && matches!(device.device_id, 0x1001 | 0x1042));
        let _ = kryo_hal::print(format_args!(
            "PCI: function bus={} slot={} function={} vendor={:#06x} device={:#06x} class={:#04x} command={:#06x} bars={}\n",
            device.address.bus(),
            device.address.device(),
            device.address.function(),
            device.vendor_id,
            device.device_id,
            device.class,
            device.command,
            device.bar_count,
        ));
    }
    let _ = kryo_hal::print(format_args!(
        "PCI: inventory complete functions={} virtio_block={} command_unchanged={}\n",
        functions.len(),
        blocks,
        unchanged,
    ));
}

pub(crate) fn check_buffers(
    pool: &BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
) {
    let before = pool.stats();
    let retained_before = kernel.borrow().retained_pages();
    let limits = DmaLimits {
        max_pages: 8,
        physical: PhysRange::from_base_len(0, 1 << 32).unwrap(),
        address_limit: u64::from(u32::MAX),
    };
    for _ in 0..2 {
        let buffer = match DmaBuffer::new(pool, 3, limits, Unpublished { direct, kernel }) {
            Ok(buffer) => buffer,
            Err(failure) => {
                if let Some(buffer) = failure.retained {
                    buffer
                        .retire()
                        .expect("Cannot retire unpublished DMA buffer");
                }
                let _ = kryo_hal::print(format_args!(
                    "DMA: bootstrap buffer unavailable {:?}\n",
                    failure.error
                ));
                return;
            }
        };
        let range = buffer.range();
        assert_eq!(range.address, buffer.physical_address());
        assert_eq!(range.bytes, 3 * FRAME_SIZE);
        // This private backend never gives the range to a device or user. The
        // buffer owns the RAM throughout these accesses; no Rust refs alias it.
        for page in (0..range.bytes).step_by(FRAME_SIZE as usize) {
            let alias = direct
                .frame_address(buffer.physical_address() + page)
                .expect("Invalid unpublished DMA alias");
            for offset in 0..FRAME_SIZE {
                unsafe {
                    assert_eq!(((alias + offset) as *const u8).read_volatile(), 0);
                    ((alias + offset) as *mut u8).write_volatile(0xa5);
                }
            }
        }
        buffer
            .retire()
            .expect("Cannot retire unpublished DMA buffer");
    }
    assert_eq!(
        pool.stats().free + kernel.borrow().retained_pages(),
        before.free + retained_before
    );
    kryo_hal::emergency_write_str("DMA: unpublished buffer zeroing and retirement OK pages=3\n");
}

struct Unpublished<'a> {
    direct: DirectMap,
    kernel: &'a RefCell<KernelMemory>,
}

// This checks buffer ownership only. No device receives an address, no device
// queue exists, and no DMA or IOMMU mapping is installed by this backend.
unsafe impl DmaBackend for Unpublished<'_> {
    type Mapping = u64;
    type Error = BackendError;

    unsafe fn initialize(&mut self, physical: u64, bytes: u64) -> Result<(), Self::Error> {
        for offset in (0..bytes).step_by(FRAME_SIZE as usize) {
            unsafe {
                self.kernel
                    .borrow_mut()
                    .initialize_shared(physical + offset)
            }?;
        }
        Ok(())
    }

    unsafe fn map(&mut self, physical: u64, _: u64) -> Result<Self::Mapping, Self::Error> {
        // Identity addressing is a bootstrap description, not DMA permission.
        self.direct.frame_address(physical)?;
        Ok(physical)
    }

    fn address(&self, mapping: &Self::Mapping) -> u64 {
        *mapping
    }

    fn quiesce(&mut self, _: &mut Self::Mapping) -> Result<(), Self::Error> {
        Ok(())
    }

    fn unmap(&mut self, _: &mut Self::Mapping) -> Result<(), Self::Error> {
        Ok(())
    }
}
