use super::super::tss::DOUBLE_FAULT_IST_INDEX;
use crate::{emergency_write_hex as hex, emergency_write_str as text};
use core::arch::asm;
use spin::Once;
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init_idt() {
    let idt = IDT.call_once(|| {
        let mut table = InterruptDescriptorTable::new();
        table.breakpoint.set_handler_fn(breakpoint_handler);
        table
            .general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        table.page_fault.set_handler_fn(page_fault_handler);
        unsafe {
            table
                .double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(DOUBLE_FAULT_IST_INDEX);
        }
        table
    });

    idt.load();
}

extern "x86-interrupt" fn breakpoint_handler(_frame: InterruptStackFrame) {
    crate::emergency_write_str("B");
}

extern "x86-interrupt" fn double_fault_handler(_frame: InterruptStackFrame, _error_code: u64) -> ! {
    crate::emergency_write_str("D");

    loop {
        unsafe { asm!("cli; hlt") }
    }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    frame: InterruptStackFrame,
    error_code: u64,
) {
    fatal_exception("#GP", &frame, error_code, None);
}

extern "x86-interrupt" fn page_fault_handler(
    frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let address = Cr2::read_raw();

    fatal_exception("#PF", &frame, error_code.bits(), Some(address));
}

fn fatal_exception(
    name: &str,
    frame: &InterruptStackFrame,
    error_code: u64,
    fault_address: Option<u64>,
) -> ! {
    x86_64::instructions::interrupts::disable();

    text("\n");
    text(name);

    text("\nRIP=");
    hex(frame.instruction_pointer.as_u64());

    text("\nRSP=");
    hex(frame.stack_pointer.as_u64());

    text("\nERROR=");
    hex(error_code);

    if let Some(address) = fault_address {
        text("\nCR2=");
        hex(address);
    }

    text("\n");

    loop {
        unsafe {
            asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
