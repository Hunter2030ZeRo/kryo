use spin::Once;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init_idt() {
    let idt = IDT.call_once(|| {
        let mut table = InterruptDescriptorTable::new();
        table.breakpoint.set_handler_fn(breakpoint_handler);
        super::super::context::install_handlers(&mut table);
        table
    });
    idt.load();
}

extern "x86-interrupt" fn breakpoint_handler(_frame: InterruptStackFrame) {
    crate::emergency_write_str("B");
}
