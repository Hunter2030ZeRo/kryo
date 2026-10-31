#![no_std]
#![feature(abi_x86_interrupt)]

mod arch;
mod debug;
mod io;

pub use arch::x86_64::apic::{
    IdleCause, IdleWake, TimerError, arm_timer, arm_timer_once, idle_until_interrupt, init_timer,
    shutdown_timer, stop_timer, timer_acknowledge, timer_initialized, timer_irq_epoch,
    timer_pending, timer_physical_address, timer_ready,
};

pub use arch::x86_64::context::{
    UserContext, UserEntryError, UserOutcome, UserReason, run_context, run_user,
};
pub use arch::x86_64::gdt::init_gdt;
pub use arch::x86_64::interrupts::idt::init_idt;
pub use arch::x86_64::paging::{
    BootPageSlot, DirectMap, KernelDirectMap, KernelRegion, PagePermissions, PagingError,
    RegionPermissions, UserPageTable,
};
pub use debug::console::{emergency_write_hex, emergency_write_str, print};
pub use io::framebuffer::Framebuffer32;
