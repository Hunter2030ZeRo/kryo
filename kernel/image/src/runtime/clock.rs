use super::KernelRegion;
use crate::boot::{MEMORY_MAP, RSDP};
use kryo_hal::AcpiError;
use limine::memmap;

const HPET_SLOT: usize = 12;
const FIRMWARE_SLOT: usize = 13;

pub(crate) fn initialize(region: &mut KernelRegion) -> u64 {
    let response = RSDP.response().expect("Missing firmware RSDP");
    assert!(!response.address.is_null(), "Null firmware RSDP");
    let mut rsdp = [0u8; 36];
    // Base revision 6 returns a bootloader-mapped virtual RSDP. Only its
    // standard prefix is read; referenced ACPI tables use physical addresses.
    unsafe {
        let pointer = response.address.cast::<u8>();
        for (offset, byte) in rsdp[..20].iter_mut().enumerate() {
            *byte = pointer.add(offset).read_volatile();
        }
        if rsdp[15] >= 2 {
            for (offset, byte) in rsdp[20..].iter_mut().enumerate() {
                *byte = pointer.add(20 + offset).read_volatile();
            }
        }
    }
    let physical = kryo_hal::find_hpet(&rsdp, |address, bytes| {
        read_firmware(region, address, bytes)
    })
    .unwrap_or_else(|error| {
        let _ = kryo_hal::print(format_args!("CLOCK discovery failed: {:?}\n", error));
        panic!("Cannot discover supported HPET");
    });
    let end = physical.checked_add(4096).expect("HPET range overflow");
    for entry in MEMORY_MAP.response().unwrap().entries() {
        let entry_end = entry
            .base
            .checked_add(entry.length)
            .expect("Invalid memory map");
        if entry.base >= end || physical >= entry_end {
            continue;
        }
        assert!(
            matches!(
                entry.type_,
                memmap::MEMMAP_RESERVED | memmap::MEMMAP_MAPPED_RESERVED
            ),
            "HPET overlaps managed RAM or retained boot data"
        );
    }
    let mapped = unsafe { region.map_mmio(HPET_SLOT, physical) }.expect("Cannot map HPET");
    let flags = region.permissions(HPET_SLOT).unwrap();
    assert!(flags.writable && flags.uncached && !flags.executable && !flags.user);
    unsafe { kryo_hal::init_clock(mapped) }.expect("Cannot initialize monotonic clock");
    let calibration = unsafe { kryo_hal::calibrate_timer() }.expect("Cannot calibrate local timer");
    let _ = kryo_hal::print(format_args!(
        "CLOCK: HPET monotonic ready lapic_counts={} sample_ns={}\n",
        calibration.counts, calibration.elapsed_ns
    ));
    physical
}

#[cfg(feature = "boot-tests")]
pub(crate) fn shutdown(region: &mut KernelRegion, physical: u64) {
    // All tasks, deadline operations, and APIC IRQs have retired before this.
    unsafe {
        kryo_hal::shutdown_clock().expect("Cannot restore HPET configuration");
        assert_eq!(
            region.unmap(HPET_SLOT).expect("Cannot unmap HPET"),
            physical
        );
    }
}

fn read_firmware(
    region: &mut KernelRegion,
    address: u64,
    bytes: &mut [u8],
) -> Result<(), AcpiError> {
    let end = address
        .checked_add(bytes.len() as u64)
        .ok_or(AcpiError::Read)?;
    if address == 0 || end <= address {
        return Err(AcpiError::Read);
    }
    let entries = MEMORY_MAP.response().ok_or(AcpiError::Read)?.entries();
    if !entries.iter().any(|entry| {
        matches!(
            entry.type_,
            memmap::MEMMAP_ACPI_RECLAIMABLE
                | memmap::MEMMAP_ACPI_NVS
                | memmap::MEMMAP_RESERVED
                | memmap::MEMMAP_MAPPED_RESERVED
        ) && entry.base <= address
            && entry
                .base
                .checked_add(entry.length)
                .is_some_and(|limit| end <= limit)
    }) {
        return Err(AcpiError::Read);
    }
    let mut copied = 0;
    while copied < bytes.len() {
        let current = address + copied as u64;
        let page = current & !4095;
        let offset = (current - page) as usize;
        let count = (4096 - offset).min(bytes.len() - copied);
        // Firmware pages remain reserved. This temporary read-only alias has
        // no Rust references and is retired before the next physical page.
        let mapped =
            unsafe { region.map_firmware(FIRMWARE_SLOT, page) }.map_err(|_| AcpiError::Read)?;
        unsafe {
            let pointer = (mapped as *const u8).add(offset);
            for index in 0..count {
                bytes[copied + index] = pointer.add(index).read_volatile();
            }
            region.unmap(FIRMWARE_SLOT).map_err(|_| AcpiError::Read)?;
        }
        copied += count;
    }
    Ok(())
}
