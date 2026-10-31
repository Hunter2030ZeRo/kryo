#![no_std]
#![feature(abi_x86_interrupt)]

mod arch;
mod debug;
mod io;

pub use arch::x86_64::gdt::init_gdt;
pub use arch::x86_64::interrupts::idt::init_idt;
pub use arch::x86_64::paging::{BootPageSlot, DirectMap, PagingError};
pub use debug::console::{emergency_write_hex, emergency_write_str, print};
pub use io::framebuffer::Framebuffer32;
