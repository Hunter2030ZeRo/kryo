use core::cell::UnsafeCell;
use spin::Once;
use x86_64::{VirtAddr, structures::tss::TaskStateSegment};

pub(super) const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct Stack([u8; STACK_SIZE]);

static mut DF_STACK: Stack = Stack([0; STACK_SIZE]);
static mut USER_ENTRY_STACK: Stack = Stack([0; STACK_SIZE]);

struct BootTss(UnsafeCell<TaskStateSegment>);

// CPU 0 owns this TSS; RSP0 changes only with local IRQs disabled.
unsafe impl Sync for BootTss {}

static BOOT_TSS: Once<BootTss> = Once::new();

pub(super) fn boot_tss() -> *const TaskStateSegment {
    BOOT_TSS
        .call_once(|| {
            let mut tss = TaskStateSegment::new();
            tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
                VirtAddr::from_ptr(&raw mut DF_STACK) + STACK_SIZE as u64;
            tss.privilege_stack_table[0] =
                VirtAddr::from_ptr(&raw mut USER_ENTRY_STACK) + STACK_SIZE as u64;
            BootTss(UnsafeCell::new(tss))
        })
        .0
        .get()
        .cast_const()
}

pub(super) fn kernel_stack() -> VirtAddr {
    // The BSP reads RSP0 with IRQs disabled; the packed field need not be aligned.
    unsafe {
        core::ptr::addr_of!((*boot_tss()).privilege_stack_table)
            .cast::<VirtAddr>()
            .read_unaligned()
    }
}

pub(super) unsafe fn set_kernel_stack(stack: VirtAddr) {
    let tss = boot_tss().cast_mut();
    unsafe {
        core::ptr::addr_of_mut!((*tss).privilege_stack_table)
            .cast::<VirtAddr>()
            .write_unaligned(stack);
    }
}
