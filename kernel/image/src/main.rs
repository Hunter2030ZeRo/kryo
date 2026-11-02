#![no_std]
#![no_main]
extern crate alloc;
mod boot;
mod heap;
mod runtime;

use core::arch::asm;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};
use kryo_hal::{init_gdt, init_idt};
use x86_64::instructions::interrupts::int3;

static PANICKING: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    use kryo_hal::{emergency_write_hex as hex, emergency_write_str as text};

    x86_64::instructions::interrupts::disable();

    if PANICKING.swap(true, Ordering::AcqRel) {
        text("\nSecondary kernel panic\n");
        loop {
            unsafe { asm!("cli", "hlt", options(nostack)) }
        }
    }

    text("\nPANIC\n");

    if let Some(location) = info.location() {
        text("file: ");
        text(location.file());

        text("\nline: ");
        hex(u64::from(location.line()));
        text("\n");
    }

    text("message: ");
    if let Some(message) = info.message().as_str() {
        text(message);
    } else {
        let _ = kryo_hal::print(format_args!("{}", info.message()));
    }
    text("\n");

    loop {
        unsafe {
            asm!("cli; hlt", options(nomem, nostack));
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    x86_64::instructions::interrupts::disable();
    kryo_hal::validate_cpu_baseline().expect("x86-64-v3 hardware required");
    kryo_hal::emergency_write_str("CPU: x86-64-v3 hardware admitted cpu=0\n");
    init_gdt();
    init_idt();
    boot::check_protocol();
    boot::platform::prepare();
    runtime::smp::prepare();

    boot::init_physical_memory();

    // This is the sole writer and the boot mapping remains valid until halt.
    let mut framebuffer = unsafe { boot::init_framebuffer() };

    if let Some(fb) = framebuffer.as_mut() {
        for y in 32..96 {
            for x in 32..160 {
                fb.put_pixel(x, y, 255, 0, 0);
            }
        }
    }

    let _ = kryo_hal::print(format_args!("Kryo: value={}, hex={:#x}\n", 42u64, 42u64,));

    kryo_hal::emergency_write_hex(42);
    kryo_hal::emergency_write_str("\n");

    kryo_hal::emergency_write_str("A");

    int3();

    kryo_hal::emergency_write_str("C");
    #[cfg(not(feature = "boot-tests"))]
    kryo_hal::emergency_write_str("\n");

    #[cfg(not(feature = "boot-tests"))]
    boot::start_services();

    #[cfg(feature = "boot-tests")]
    loop {
        unsafe {
            asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
