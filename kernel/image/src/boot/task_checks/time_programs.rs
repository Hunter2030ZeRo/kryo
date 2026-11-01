use super::programs::Program;
use kryo_core::capability::Rights;
use kryo_interfaces::syscall::{Call, Error, TaskStatus};

const SHORT_WAIT: u32 = 20_000_000;
const LONG_WAIT: u32 = 100_000_000;
const SHARED_PAGE: u64 = 0xc0_0000;
const SHARED_VALUE: u64 = 0x1234_5678_9abc_def0;

pub(super) fn exit(status: u64) -> Program {
    let mut code = Program::new();
    code.call(Call::Exit, status, 0);
    code.finish()
}

pub(super) fn sleep_checks() -> Program {
    let mut code = Program::new();
    code.call(Call::SleepUntil, 0, 0);
    code.expect_status(0);
    deadline(&mut code, SHORT_WAIT);
    sleep_saved(&mut code);
    code.expect_status(0);
    expect_deadline_reached(&mut code);
    code.call(Call::Exit, 61, 0);
    code.finish()
}

pub(super) fn timed_client(accepted: bool) -> Program {
    let mut code = Program::new();
    if accepted {
        wait_until_blocked(&mut code);
    }
    deadline(&mut code, if accepted { LONG_WAIT } else { SHORT_WAIT });
    timed_call(&mut code, Call::CallUntil, 0);
    code.expect_status(Error::TimedOut.status());
    code.expect_value(u32::from(accepted));
    expect_deadline_reached(&mut code);
    notification_call(&mut code, Call::Signal);
    code.expect_status(0);
    code.call(Call::Exit, if accepted { 65 } else { 62 }, 0);
    code.finish()
}

pub(super) fn waiting_owner(status: u64) -> Program {
    let mut code = Program::new();
    // Retain the endpoint until its caller has observed the timeout.
    notification_call(&mut code, Call::Wait);
    code.expect_status(0);
    code.call(Call::Exit, status, 0);
    code.finish()
}

pub(super) fn late_reply() -> Program {
    let mut code = Program::new();
    call_handle(&mut code, Call::Receive, 0);
    code.expect_status(0);
    code.emit(&[0x48, 0x85, 0xd2]);
    code.failure_if_zero();
    code.emit(&[0x49, 0x89, 0xd7]); // mov r15,rdx: reply token
    // The caller signals only after the accepted request has timed out.
    notification_call(&mut code, Call::Wait);
    code.expect_status(0);
    code.emit(&[0xb8]);
    code.emit(&(Call::Reply as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef, 0x4c, 0x89, 0xfe, 0xcd, 0x80]);
    code.expect_status(Error::InvalidReply.status());
    code.call(Call::Exit, 64, 0);
    code.finish()
}

pub(super) fn notification_timeout() -> Program {
    let mut code = Program::new();
    deadline(&mut code, SHORT_WAIT);
    timed_call(&mut code, Call::WaitUntil, 0);
    code.expect_status(Error::TimedOut.status());
    code.expect_value(0);
    expect_deadline_reached(&mut code);
    call_handle(&mut code, Call::Signal, 0);
    code.expect_status(0);
    call_handle(&mut code, Call::Wait, 0);
    code.expect_status(0);
    code.call(Call::Exit, 66, 0);
    code.finish()
}

pub(super) fn notification_event() -> Program {
    let mut code = Program::new();
    deadline(&mut code, LONG_WAIT);
    timed_call(&mut code, Call::WaitUntil, 0);
    code.expect_status(0);
    // Reusing the wait slot must not let the notification's old deadline wake
    // this later sleep. The clock check catches any premature completion.
    code.emit(&[0x49, 0x81, 0xc4]);
    code.emit(&LONG_WAIT.to_le_bytes());
    sleep_saved(&mut code);
    code.expect_status(0);
    expect_deadline_reached(&mut code);
    code.call(Call::Exit, 67, 0);
    code.finish()
}

pub(super) fn signaller() -> Program {
    let mut code = Program::new();
    code.call(Call::Yield, 0, 0);
    code.expect_status(0);
    call_handle(&mut code, Call::Signal, 0);
    code.expect_status(0);
    code.call(Call::Exit, 68, 0);
    code.finish()
}

pub(super) fn cancelled_sleep() -> Program {
    let mut code = Program::new();
    deadline(&mut code, LONG_WAIT);
    sleep_saved(&mut code);
    code.expect_status(Error::Cancelled.status());
    code.expect_value(0);
    code.emit(&[0x49, 0x81, 0xc4]);
    code.emit(&LONG_WAIT.to_le_bytes());
    sleep_saved(&mut code);
    code.expect_status(0);
    expect_deadline_reached(&mut code);
    code.call(Call::Exit, 69, 0);
    code.finish()
}

pub(super) fn canceller() -> Program {
    let mut code = Program::new();
    wait_until_blocked(&mut code);
    call_handle(&mut code, Call::CancelWait, 0);
    code.expect_status(0);
    code.expect_value(0);
    code.call(Call::Exit, 70, 0);
    code.finish()
}

pub(super) fn expired_wait() -> Program {
    let mut code = Program::new();
    call_handle(&mut code, Call::Signal, 0);
    code.expect_status(0);
    code.immediate(&[0x49, 0xbc], 0);
    timed_call(&mut code, Call::WaitUntil, 0);
    code.expect_status(Error::TimedOut.status());
    code.expect_value(0);
    // An already expired request must leave the early signal pending.
    call_handle(&mut code, Call::Wait, 0);
    code.expect_status(0);
    code.call(Call::SleepUntil, 0, 0);
    code.expect_status(0);
    code.call(Call::Exit, 71, 0);
    code.finish()
}

pub(super) fn receive_retry() -> Program {
    let mut code = Program::new();
    deadline(&mut code, SHORT_WAIT);
    timed_call(&mut code, Call::ReceiveUntil, 0);
    code.expect_status(Error::TimedOut.status());
    code.expect_value(0);
    expect_deadline_reached(&mut code);
    notification_call(&mut code, Call::Signal);
    code.expect_status(0);
    call_handle(&mut code, Call::Receive, 0);
    code.expect_status(0);
    code.expect_value(0);
    code.immediate(&[0x48, 0xb8], 0x51_ee_9a);
    code.emit(&[0x49, 0x39, 0xc0]);
    code.failure_jump();
    code.call(Call::Exit, 72, 0);
    code.finish()
}

pub(super) fn notified_sender() -> Program {
    let mut code = Program::new();
    notification_call(&mut code, Call::Wait);
    code.expect_status(0);
    code.immediate(&[0x49, 0xb8], 0x51_ee_9a);
    call_handle(&mut code, Call::Send, 0);
    code.expect_status(0);
    code.call(Call::Exit, 73, 0);
    code.finish()
}

pub(super) fn timed_capability() -> Program {
    let mut code = Program::new();
    code.call(Call::MemoryCreate, 1, 0);
    code.expect_status(0);
    code.emit(&[0x49, 0x89, 0xd5]); // mov r13,rdx: retain source handle
    deadline(&mut code, SHORT_WAIT);
    code.emit(&[0x4c, 0x89, 0xe2]); // mov rdx,r12: deadline argument
    code.emit(&[0x4d, 0x89, 0xec]); // mov r12,r13: first transferred handle
    code.immediate(&[0x49, 0xbe], (Rights::READ | Rights::MAP).bits());
    code.emit(&[0xb8]);
    code.emit(&(Call::CallCapsUntil as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], 1);
    code.emit(&[0xcd, 0x80]);
    code.expect_status(Error::TimedOut.status());
    code.expect_value(0);
    code.emit(&[0xb8]);
    code.emit(&(Call::Close as u32).to_le_bytes());
    code.emit(&[0x4c, 0x89, 0xef, 0xcd, 0x80]); // close the original source handle
    code.expect_status(0);
    notification_call(&mut code, Call::Signal);
    code.expect_status(0);
    code.call(Call::Exit, 74, 0);
    code.finish()
}

pub(super) fn accepted_capability_sender() -> Program {
    let mut code = Program::new();
    code.call(Call::MemoryCreate, 1, 0);
    code.expect_status(0);
    code.emit(&[0x49, 0x89, 0xd5]);
    map_memory(&mut code, true);
    code.immediate(&[0x48, 0xb8], SHARED_VALUE);
    code.immediate(&[0x48, 0xbf], SHARED_PAGE);
    code.emit(&[0x48, 0x89, 0x07]);
    code.call(Call::MemoryUnmap, SHARED_PAGE, 0);
    code.expect_status(0);
    deadline(&mut code, LONG_WAIT);
    code.emit(&[0x4c, 0x89, 0xe2, 0x4d, 0x89, 0xec]);
    code.immediate(&[0x49, 0xbe], (Rights::READ | Rights::MAP).bits());
    code.emit(&[0xb8]);
    code.emit(&(Call::CallCapsUntil as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], 1);
    code.emit(&[0xcd, 0x80]);
    code.expect_status(Error::TimedOut.status());
    code.expect_value(1);
    close_memory(&mut code);
    // The notification establishes that the source handle is already closed
    // when the receiver examines the accepted capability.
    notification_call(&mut code, Call::Signal);
    code.expect_status(0);
    code.call(Call::Exit, 76, 0);
    code.finish()
}

pub(super) fn accepted_capability_receiver() -> Program {
    let mut code = Program::new();
    call_handle(&mut code, Call::Receive, 1);
    code.expect_status(0);
    code.emit(&[0x49, 0x83, 0xfe, 1]);
    code.failure_jump();
    code.emit(&[0x49, 0x89, 0xd7, 0x4d, 0x89, 0xe5]);
    notification_call(&mut code, Call::Wait);
    code.expect_status(0);
    code.emit(&[0xb8]);
    code.emit(&(Call::Reply as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef, 0x4c, 0x89, 0xfe, 0xcd, 0x80]);
    code.expect_status(Error::InvalidReply.status());
    map_memory(&mut code, false);
    code.immediate(&[0x48, 0xbf], SHARED_PAGE);
    code.emit(&[0x48, 0x8b, 0x07]);
    code.immediate(&[0x48, 0xba], SHARED_VALUE);
    code.emit(&[0x48, 0x39, 0xd0]);
    code.failure_jump();
    close_memory(&mut code);
    code.call(Call::MemoryUnmap, SHARED_PAGE, 0);
    code.expect_status(0);
    code.call(Call::Exit, 77, 0);
    code.finish()
}

pub(super) fn early_interrupt_sleeper() -> Program {
    let mut code = Program::new();
    deadline(&mut code, 1_000_000_000);
    sleep_saved(&mut code);
    code.expect_status(0);
    expect_deadline_reached(&mut code);
    code.call(Call::Exit, 78, 0);
    code.finish()
}

fn notification_call(code: &mut Program, call: Call) {
    code.emit(&[0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xdf, 0xcd, 0x80]);
}

fn wait_until_blocked(code: &mut Program) {
    let inspect = code.as_bytes().len();
    code.emit(&[0xb8]);
    code.emit(&(Call::TaskStatus as u32).to_le_bytes());
    code.emit(&[0x4c, 0x89, 0xff, 0xcd, 0x80]); // mov rdi,r15: fixture INSPECT handle
    code.expect_status(0);
    code.emit(&[0x48, 0x81, 0xfa]); // cmp rdx,Blocked
    code.emit(&(TaskStatus::Blocked as u32).to_le_bytes());
    code.emit(&[0x0f, 0x85]);
    let displacement =
        i32::try_from(inspect as isize - (code.as_bytes().len() + 4) as isize).unwrap();
    code.emit(&displacement.to_le_bytes());
}

fn map_memory(code: &mut Program, writable: bool) {
    code.emit(&[0xb8]);
    code.emit(&(Call::MemoryMap as u32).to_le_bytes());
    code.emit(&[0x4c, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], SHARED_PAGE);
    code.emit(&[0x31, 0xd2]);
    code.immediate(&[0x49, 0xb8], u64::from(writable));
    code.emit(&[0xcd, 0x80]);
    code.expect_status(0);
}

fn close_memory(code: &mut Program) {
    code.emit(&[0xb8]);
    code.emit(&(Call::Close as u32).to_le_bytes());
    code.emit(&[0x4c, 0x89, 0xef, 0xcd, 0x80]);
    code.expect_status(0);
}

fn deadline(code: &mut Program, duration: u32) {
    code.call(Call::ClockRead, 0, 0);
    code.expect_status(0);
    code.emit(&[0x48, 0x81, 0xc2]); // add rdx,imm32
    code.emit(&duration.to_le_bytes());
    code.emit(&[0x49, 0x89, 0xd4]); // mov r12,rdx
}

fn sleep_saved(code: &mut Program) {
    code.emit(&[0xb8]);
    code.emit(&(Call::SleepUntil as u32).to_le_bytes());
    code.emit(&[0x4c, 0x89, 0xe7, 0xcd, 0x80]);
}

fn expect_deadline_reached(code: &mut Program) {
    code.call(Call::ClockRead, 0, 0);
    code.expect_status(0);
    code.emit(&[0x4c, 0x39, 0xe2, 0x0f, 0x93, 0xc0, 0x84, 0xc0]);
    code.failure_if_zero();
}

fn timed_call(code: &mut Program, call: Call, extra: u64) {
    code.emit(&[0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], extra);
    code.emit(&[0x4c, 0x89, 0xe2, 0xcd, 0x80]);
}

fn call_handle(code: &mut Program, call: Call, extra: u64) {
    code.emit(&[0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], extra);
    code.emit(&[0xcd, 0x80]);
}
