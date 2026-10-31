use super::programs::Program;
use kryo_interfaces::syscall::{Call, Error};

const REQUEST: [u64; 4] = [41, 0x1234_5678_9abc_def0, 7, 9];
const RESPONSE: [u64; 4] = [42, 0x1234_5678_9abc_def0, 7, 9];
const SENT: [u64; 4] = [71, 0x1234_5678_9abc_def0, 17, 19];

pub(super) fn rpc_client(endpoint: u64) -> Program {
    let mut code = Program::new();
    call_on(&mut code, Call::Receive, endpoint, 0);
    code.expect_status(Error::PermissionDenied.status());
    message(&mut code, REQUEST);
    call_on(&mut code, Call::Call, endpoint, 0);
    code.expect_status(0);
    expect_message(&mut code, RESPONSE);
    message(&mut code, SENT);
    call_on(&mut code, Call::Send, endpoint, 0);
    code.expect_status(0);
    code.call(Call::Exit, 11, 0);
    code.finish()
}

pub(super) fn rpc_server(endpoint: u64) -> Program {
    let mut code = Program::new();
    call_on(&mut code, Call::Receive, endpoint, 0);
    code.expect_status(0);
    expect_message(&mut code, REQUEST);
    code.emit(&[0x48, 0x85, 0xd2]); // test rdx,rdx
    code.failure_if_zero();
    code.emit(&[0x49, 0x89, 0xd7]); // mov r15,rdx

    message(&mut code, RESPONSE);
    reply(&mut code, endpoint);
    code.expect_status(0);
    reply(&mut code, endpoint);
    code.expect_status(Error::InvalidReply.status());

    call_on(&mut code, Call::Receive, endpoint, 0);
    code.expect_status(0);
    code.expect_value(0);
    expect_message(&mut code, SENT);
    code.call(Call::Exit, 12, 0);
    code.finish()
}

pub(super) fn waiter(notification: u64, status: u64) -> Program {
    let mut code = Program::new();
    call_on(&mut code, Call::Wait, notification, 0);
    code.expect_status(0);
    code.call(Call::Exit, status, 0);
    code.finish()
}

pub(super) fn double_waiter(notification: u64, status: u64) -> Program {
    let mut code = Program::new();
    for _ in 0..2 {
        code.call(Call::Yield, 0, 0);
        code.expect_status(0);
    }
    for _ in 0..2 {
        call_on(&mut code, Call::Wait, notification, 0);
        code.expect_status(0);
    }
    code.call(Call::Exit, status, 0);
    code.finish()
}

pub(super) fn signaller(notification: u64) -> Program {
    let mut code = Program::new();
    for _ in 0..2 {
        call_on(&mut code, Call::Signal, notification, 0);
        code.expect_status(0);
    }
    code.call(Call::Exit, 13, 0);
    code.finish()
}

pub(super) fn cancelled_client(endpoint: u64, delivery: u64) -> Program {
    let mut code = Program::new();
    message(&mut code, REQUEST);
    call_on(&mut code, Call::Call, endpoint, 0);
    code.expect_status(Error::Cancelled.status());
    expect_result(&mut code, delivery);
    code.call(Call::Exit, 14, 0);
    code.finish()
}

pub(super) fn closer_or_canceller(handle: u64, cancel: bool) -> Program {
    let mut code = Program::new();
    let call = if cancel {
        Call::CancelWait
    } else {
        Call::EndpointClose
    };
    call_on(&mut code, call, handle, 0);
    code.expect_status(0);
    code.call(Call::Exit, 15, 0);
    code.finish()
}

pub(super) fn peer_closed_client(endpoint: u64) -> Program {
    let mut code = Program::new();
    message(&mut code, REQUEST);
    call_on(&mut code, Call::Call, endpoint, 0);
    code.expect_status(Error::PeerClosed.status());
    code.call(Call::Exit, 16, 0);
    code.finish()
}

pub(super) fn exiting_server(endpoint: u64, accept: bool) -> Program {
    let mut code = Program::new();
    if accept {
        call_on(&mut code, Call::Receive, endpoint, 0);
        code.expect_status(0);
    }
    code.call(Call::Exit, 17, 0);
    code.finish()
}

pub(super) fn cancelling_server(endpoint: u64) -> Program {
    let mut code = Program::new();
    call_on(&mut code, Call::Receive, endpoint, 0);
    code.expect_status(0);
    code.emit(&[0x48, 0x85, 0xd2]); // test rdx,rdx
    code.failure_if_zero();
    code.emit(&[0x49, 0x89, 0xd7]); // mov r15,rdx
    code.emit(&[0xb8]);
    code.emit(&(Call::CancelWait as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xdf]); // mov rdi,rbx: client CONTROL handle
    code.immediate(&[0x48, 0xbe], 0);
    code.emit(&[0xcd, 0x80]);
    code.expect_status(0);
    code.expect_value(1);
    reply(&mut code, endpoint);
    code.expect_status(Error::InvalidReply.status());
    code.call(Call::Exit, 19, 0);
    code.finish()
}

fn message(code: &mut Program, words: [u64; 4]) {
    for (index, word) in words.into_iter().enumerate() {
        code.immediate(&[0x49, 0xb8 + index as u8], word); // mov r8..r11,imm64
    }
}

fn expect_message(code: &mut Program, words: [u64; 4]) {
    for (index, word) in words.into_iter().enumerate() {
        code.immediate(&[0x48, 0xb8], word); // mov rax,imm64
        code.emit(&[0x49, 0x39, 0xc0 + index as u8]); // cmp r8..r11,rax
        code.failure_jump();
    }
}

fn expect_result(code: &mut Program, expected: u64) {
    code.immediate(&[0x48, 0xb8], expected);
    code.emit(&[0x48, 0x39, 0xc2]); // cmp rdx,rax
    code.failure_jump();
}

fn reply(code: &mut Program, endpoint: u64) {
    code.emit(&[0xb8]);
    code.emit(&(Call::Reply as u32).to_le_bytes());
    if endpoint == 0 {
        code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp
    } else {
        code.immediate(&[0x48, 0xbf], endpoint);
    }
    code.emit(&[0x4c, 0x89, 0xfe, 0xcd, 0x80]); // mov rsi,r15; int80
}

fn call_on(code: &mut Program, call: Call, handle: u64, extra: u64) {
    if handle != 0 {
        code.call(call, handle, extra);
        return;
    }
    code.emit(&[0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp: bootstrap handle supplied after task creation
    code.immediate(&[0x48, 0xbe], extra);
    code.emit(&[0xcd, 0x80]);
}
