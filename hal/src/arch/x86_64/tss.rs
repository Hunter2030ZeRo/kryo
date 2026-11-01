use super::cpu::{CpuId, MAX_CPUS, current_cpu};
use core::cell::UnsafeCell;
use spin::Once;
use x86_64::{VirtAddr, structures::tss::TaskStateSegment};

pub(super) const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct Stack(UnsafeCell<[u8; STACK_SIZE]>);

// Each owning CPU alone uses its stack cells; their static addresses never move.
unsafe impl Sync for Stack {}
static DF_STACKS: [Stack; MAX_CPUS] = [const { Stack(UnsafeCell::new([0; STACK_SIZE])) }; MAX_CPUS];
static ENTRY_STACKS: [Stack; MAX_CPUS] =
    [const { Stack(UnsafeCell::new([0; STACK_SIZE])) }; MAX_CPUS];

struct LocalTss(UnsafeCell<TaskStateSegment>);

// Only the indexed CPU changes RSP0, with local IRQs disabled.
unsafe impl Sync for LocalTss {}

static TSS: [Once<LocalTss>; MAX_CPUS] = [const { Once::new() }; MAX_CPUS];

pub(super) fn local_tss(cpu: CpuId) -> *const TaskStateSegment {
    TSS[cpu.index()]
        .call_once(|| {
            let mut tss = TaskStateSegment::new();
            tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
                VirtAddr::from_ptr(DF_STACKS[cpu.index()].0.get()) + STACK_SIZE as u64;
            tss.privilege_stack_table[0] =
                VirtAddr::from_ptr(ENTRY_STACKS[cpu.index()].0.get()) + STACK_SIZE as u64;
            LocalTss(UnsafeCell::new(tss))
        })
        .0
        .get()
        .cast_const()
}

pub(super) fn kernel_stack() -> VirtAddr {
    let cpu = current_cpu().expect("Unregistered TSS owner");
    // Read CPU-local RSP0 with IRQs off; the packed field need not be aligned.
    unsafe {
        core::ptr::addr_of!((*local_tss(cpu)).privilege_stack_table)
            .cast::<VirtAddr>()
            .read_unaligned()
    }
}

pub(super) unsafe fn set_kernel_stack(stack: VirtAddr) {
    let tss = local_tss(current_cpu().expect("Unregistered TSS owner")).cast_mut();
    unsafe {
        core::ptr::addr_of_mut!((*tss).privilege_stack_table)
            .cast::<VirtAddr>()
            .write_unaligned(stack);
    }
}
