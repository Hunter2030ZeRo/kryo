use spin::Once;
use x86_64::{VirtAddr, structures::tss::TaskStateSegment};

pub(super) const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct Stack([u8; STACK_SIZE]);

static mut DF_STACK: Stack = Stack([0; STACK_SIZE]);

static BOOT_TSS: Once<TaskStateSegment> = Once::new();

pub(super) fn boot_tss() -> &'static TaskStateSegment {
    BOOT_TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();

        let stack_start = VirtAddr::from_ptr(&raw mut DF_STACK);
        let stack_top = stack_start + STACK_SIZE as u64;

        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_top;

        tss
    })
}
