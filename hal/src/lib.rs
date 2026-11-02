#![no_std]
#![feature(abi_x86_interrupt)]
extern crate alloc;

mod arch;
mod debug;
mod io;

pub use arch::x86_64::apic::{
    IdleCause, IdleWake, TimerCalibration, TimerError, arm_deadline, arm_timer, arm_timer_once,
    calibrate_timer, idle_until_interrupt, init_timer, shutdown_timer, stop_timer,
    timer_acknowledge, timer_calibration, timer_initialized, timer_irq_epoch, timer_pending,
    timer_physical_address, timer_ready,
};

pub use arch::x86_64::acpi::{AcpiError, find_hpet};
pub use arch::x86_64::clock::{
    ClockError, clock_initialized, init_clock, monotonic_time, shutdown_clock,
};
pub use arch::x86_64::hpet_device::{
    DeviceError, TimerDevice, claim_timer_device, device_irq_epoch, device_irq_pending,
    take_device_irq,
};

pub use arch::x86_64::smp::{
    SmpError, active_user_mask, all_users_quiescent, begin_user_lease, cpu_online_mask,
    cpu_tlb_epoch, drain_reschedule, end_user_lease, global_tlb_flush, idle_event,
    reschedule_epoch, send_reschedule, set_cpu_online, tlb_epoch, wake_boot_cpu,
};

pub use arch::x86_64::context::{
    ExtendedState, UserContext, UserEntryError, UserOutcome, UserReason, run_context,
    run_context_extended, run_context_until, run_user,
};
pub use arch::x86_64::cpu::{
    CpuError, CpuId, CpuSet, MAX_CPUS, cpu_apic_id, cpu_by_index, current_cpu, init_boot_cpu,
    register_cpu,
};
pub use arch::x86_64::gdt::init_gdt;
pub use arch::x86_64::interrupts::idt::init_idt;
pub use arch::x86_64::paging::{
    BootPageSlot, DirectMap, KernelDirectMap, KernelRegion, PagePermissions, PagingError,
    RegionPermissions, UserPageTable,
};
pub use arch::x86_64::pci::{
    PciAddress, PciConfigRead, PciError, PciFunction, read_pci_config, scan_pci, scan_pci_with,
};
pub use arch::x86_64::virtio::{VirtioError, VirtioPci, VirtioQueue};
pub use debug::console::{emergency_write_hex, emergency_write_str, print};
pub use io::framebuffer::Framebuffer32;

#[cfg(feature = "boot-tests")]
pub use arch::x86_64::hpet_device::{device_irq_queued, fail_next_device_close};
#[cfg(feature = "boot-tests")]
pub use arch::x86_64::smp::inject_tlb_miss;
