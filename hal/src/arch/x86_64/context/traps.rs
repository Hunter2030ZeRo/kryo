use super::{RunState, UserOutcome, UserReason, finish, local_state};
use crate::{emergency_write_hex as hex, emergency_write_str as text};
use core::arch::asm;
use x86_64::registers::model_specific::{FsBase, GsBase};
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::{PrivilegeLevel, VirtAddr};

#[repr(C)]
pub(super) struct TrapFrame {
    registers: [u64; 15],
    vector: u64,
    error: u64,
    instruction: u64,
    code: u64,
    flags: u64,
    stack: u64,
    stack_selector: u64,
}

pub(super) const FRAME_CODE_OFFSET: usize = core::mem::offset_of!(TrapFrame, code);

unsafe extern "C" {
    fn kryo_user_exit();
    fn kryo_device_interrupt();
    fn kryo_timer_interrupt();
    fn kryo_reschedule_interrupt();
    fn kryo_shootdown_interrupt();
    fn kryo_spurious_interrupt();
    fn kryo_fault_0();
    fn kryo_fault_1();
    fn kryo_fault_4();
    fn kryo_fault_5();
    fn kryo_fault_6();
    fn kryo_fault_7();
    fn kryo_fault_8();
    fn kryo_fault_10();
    fn kryo_fault_11();
    fn kryo_fault_12();
    fn kryo_fault_13();
    fn kryo_fault_14();
    fn kryo_fault_16();
    fn kryo_fault_17();
    fn kryo_fault_19();
    fn kryo_fault_20();
    fn kryo_fault_21();
    fn kryo_fault_29();
    fn kryo_fault_30();
}

pub(crate) fn install_handlers(table: &mut InterruptDescriptorTable) {
    // The assembly stubs normalize error codes and clear DF before calling Rust.
    unsafe {
        table
            .divide_error
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_0 as *const ()));
        table
            .debug
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_1 as *const ()));
        table
            .overflow
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_4 as *const ()));
        table
            .bound_range_exceeded
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_5 as *const ()));
        table
            .invalid_opcode
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_6 as *const ()));
        table
            .device_not_available
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_7 as *const ()));
        table
            .double_fault
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_8 as *const ()))
            .set_stack_index(super::super::tss::DOUBLE_FAULT_IST_INDEX);
        table
            .invalid_tss
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_10 as *const ()));
        table
            .segment_not_present
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_11 as *const ()));
        table
            .stack_segment_fault
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_12 as *const ()));
        table
            .general_protection_fault
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_13 as *const ()));
        table
            .page_fault
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_14 as *const ()));
        table
            .x87_floating_point
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_16 as *const ()));
        table
            .alignment_check
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_17 as *const ()));
        table
            .simd_floating_point
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_19 as *const ()));
        table
            .virtualization
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_20 as *const ()));
        table
            .cp_protection_exception
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_21 as *const ()));
        table
            .vmm_communication_exception
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_29 as *const ()));
        table
            .security_exception
            .set_handler_addr(VirtAddr::from_ptr(kryo_fault_30 as *const ()));
        table[0xd0].set_handler_addr(VirtAddr::from_ptr(kryo_device_interrupt as *const ()));
        table[0xe0].set_handler_addr(VirtAddr::from_ptr(kryo_timer_interrupt as *const ()));
        table[0xe1].set_handler_addr(VirtAddr::from_ptr(kryo_reschedule_interrupt as *const ()));
        table[0xe2].set_handler_addr(VirtAddr::from_ptr(kryo_shootdown_interrupt as *const ()));
        table[0xff].set_handler_addr(VirtAddr::from_ptr(kryo_spurious_interrupt as *const ()));
        table[0x80]
            .set_handler_addr(VirtAddr::from_ptr(kryo_user_exit as *const ()))
            .set_privilege_level(PrivilegeLevel::Ring3);
    }
}

pub(super) unsafe extern "C" fn exit_dispatch(frame: *const TrapFrame) -> ! {
    let frame = unsafe { &*frame };
    let state = state_pointer();
    if frame.code & 3 == 3 && unsafe { (*state).active } {
        unsafe {
            save_context(frame);
            finish(UserOutcome {
                reason: if (*state).legacy_exit {
                    UserReason::Exited
                } else {
                    UserReason::Call
                },
                status: frame.registers[5],
                vector: 0x80,
                error: 0,
                address: 0,
                instruction: frame.instruction,
            })
        }
    }
    text("\nUnexpected user exit gate\n");
    halt()
}

pub(super) unsafe extern "C" fn device_dispatch(frame: *const TrapFrame) {
    super::super::hpet_device::record_interrupt();
    crate::timer_acknowledge();
    unsafe { preempt(frame) };
}

pub(super) unsafe extern "C" fn timer_dispatch(frame: *const TrapFrame) {
    super::super::apic::record_timer_interrupt();
    crate::timer_acknowledge();
    unsafe { preempt(frame) };
}

pub(super) unsafe extern "C" fn reschedule_dispatch(frame: *const TrapFrame) {
    super::super::smp::reschedule_interrupt();
    unsafe { preempt(frame) };
}

pub(super) unsafe extern "C" fn shootdown_dispatch(frame: *const TrapFrame) {
    super::super::smp::shootdown_interrupt();
    // CPL3 paths saved FP and set TS before dispatch. Return through the normal
    // scheduler boundary so user state is restored before execution resumes.
    unsafe { preempt(frame) };
}

unsafe fn preempt(frame: *const TrapFrame) {
    let frame = unsafe { &*frame };
    if frame.code & 3 == 3 {
        let state = state_pointer();
        if !unsafe { (*state).active && (*state).preemptible } {
            text("\nUnexpected user hardware interrupt\n");
            halt();
        }
        unsafe {
            save_context(frame);
            finish(UserOutcome {
                reason: UserReason::Preempted,
                status: 0,
                vector: frame.vector,
                error: 0,
                address: 0,
                instruction: frame.instruction,
            });
        }
    }
}

unsafe fn save_context(frame: &TrapFrame) {
    let context = unsafe { (*state_pointer()).context };
    assert!(!context.is_null(), "Missing active user context");
    // The register prefix and hardware frame were captured before Rust could clobber them.
    unsafe {
        core::ptr::copy_nonoverlapping(frame.registers.as_ptr(), context.cast::<u64>(), 15);
        (*context).rip = frame.instruction;
        (*context).rsp = frame.stack;
        (*context).rflags = frame.flags;
        (*context).fs_base = FsBase::read().as_u64();
        (*context).gs_base = GsBase::read().as_u64();
    }
}

pub(super) unsafe extern "C" fn fault_dispatch(frame: *const TrapFrame, cr2: u64) -> ! {
    let frame = unsafe { &*frame };
    if frame.vector != 8 && frame.code & 3 == 3 && unsafe { (*state_pointer()).active } {
        unsafe {
            save_context(frame);
            finish(UserOutcome {
                reason: UserReason::Fault,
                status: 0,
                vector: frame.vector,
                error: frame.error,
                address: if frame.vector == 14 { cr2 } else { 0 },
                instruction: frame.instruction,
            })
        }
    }
    if frame.vector == 8 {
        text("D");
    }
    text(match frame.vector {
        0 => "\n#DE\nVECTOR=",
        6 => "\n#UD\nVECTOR=",
        7 => "\n#NM\nVECTOR=",
        8 => "\n#DF\nVECTOR=",
        13 => "\n#GP\nVECTOR=",
        14 => "\n#PF\nVECTOR=",
        _ => "\nEXCEPTION=",
    });
    hex(frame.vector);
    text("\nRIP=");
    hex(frame.instruction);
    text("\nRSP=");
    hex(frame.stack);
    text("\nERROR=");
    hex(frame.error);
    if frame.vector == 14 {
        text("\nCR2=");
        hex(cr2);
    }
    text("\n");
    halt()
}

fn halt() -> ! {
    loop {
        unsafe { asm!("cli; hlt", options(nomem, nostack)) };
    }
}

fn state_pointer() -> *mut RunState {
    local_state().unwrap_or_else(|| {
        text("\nUnregistered trap CPU\n");
        halt()
    })
}
