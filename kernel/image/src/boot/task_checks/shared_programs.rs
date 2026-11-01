use super::programs::Program;
use kryo_interfaces::syscall::{Call, Error};

const PRODUCER_FIRST: u64 = 0xa0_0000;
const PRODUCER_SECOND: u64 = 0xb0_0000;
const RECEIVER_FIRST: u64 = 0xc0_0000;
const RECEIVER_SECOND: u64 = 0xd0_0000;
const PAGE_SIZE: u64 = 4096;
const TAIL: u64 = 0x1234_5678_9abc_def0;
const READ_MAP: u64 = 128 | 512;
const WRITE_MAP: u64 = READ_MAP | 256;
const MESSAGE: [u64; 4] = [41, TAIL, 42, 71];

pub(super) fn producer() -> Program {
    let mut code = Program::new();
    code.call(Call::MemoryCreate, 2, 0);
    code.expect_status(0);
    code.emit(&[0x49, 0x89, 0xd4]); // mov r12,rdx
    code.call(Call::MemoryCreate, 1, 0);
    code.expect_status(0);
    code.emit(&[0x49, 0x89, 0xd5]); // mov r13,rdx

    map(&mut code, false, PRODUCER_FIRST, 0, true);
    code.expect_status(0);
    map(&mut code, false, PRODUCER_FIRST + PAGE_SIZE, 1, true);
    code.expect_status(0);
    map(&mut code, true, PRODUCER_SECOND, 0, true);
    code.expect_status(0);
    for address in [
        PRODUCER_FIRST,
        PRODUCER_FIRST + PAGE_SIZE - 8,
        PRODUCER_FIRST + PAGE_SIZE,
    ] {
        expect_memory(&mut code, address, 0);
    }
    write_memory(&mut code, PRODUCER_FIRST, 41);
    write_memory(&mut code, PRODUCER_FIRST + PAGE_SIZE - 8, TAIL);
    write_memory(&mut code, PRODUCER_FIRST + PAGE_SIZE, 42);
    write_memory(&mut code, PRODUCER_SECOND, 71);

    message(&mut code);
    code.immediate(&[0x49, 0xbe], READ_MAP); // mov r14,rights
    code.immediate(&[0x49, 0xbf], WRITE_MAP); // mov r15,rights
    code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp: endpoint
    code.immediate(&[0x48, 0xbe], 2);
    invoke(&mut code, Call::SendCaps);
    code.expect_status(0);
    close(&mut code, false);
    close(&mut code, true);
    for address in [PRODUCER_FIRST, PRODUCER_FIRST + PAGE_SIZE, PRODUCER_SECOND] {
        unmap(&mut code, address);
    }
    notification(&mut code, Call::Signal);
    code.call(Call::Exit, 31, 0);
    code.finish()
}

pub(super) fn receiver() -> Program {
    let mut code = Program::new();
    for _ in 0..6 {
        code.call(Call::Yield, 0, 0);
        code.expect_status(0);
    }
    code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp: endpoint
    code.immediate(&[0x48, 0xbe], 2);
    invoke(&mut code, Call::Receive);
    code.expect_status(0);
    code.expect_value(0);
    code.emit(&[0x49, 0x83, 0xfe, 2]); // cmp r14,2: received capability count
    code.failure_jump();
    expect_message(&mut code);

    code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp: endpoint also grants SEND
    code.immediate(&[0x48, 0xbe], 1);
    code.immediate(&[0x49, 0xbe], READ_MAP);
    invoke(&mut code, Call::SendCaps);
    code.expect_status(Error::PermissionDenied.status());

    map(&mut code, false, RECEIVER_FIRST, 0, true);
    code.expect_status(Error::PermissionDenied.status());
    map(&mut code, false, RECEIVER_FIRST, 0, false);
    code.expect_status(0);
    map(&mut code, false, RECEIVER_FIRST + PAGE_SIZE, 1, false);
    code.expect_status(0);
    map(&mut code, true, RECEIVER_SECOND, 0, true);
    code.expect_status(0);
    close(&mut code, false);
    close(&mut code, true);
    notification(&mut code, Call::Wait);

    expect_memory(&mut code, RECEIVER_FIRST, 41);
    expect_memory(&mut code, RECEIVER_FIRST + PAGE_SIZE - 8, TAIL);
    expect_memory(&mut code, RECEIVER_FIRST + PAGE_SIZE, 42);
    expect_memory(&mut code, RECEIVER_SECOND, 71);
    write_memory(&mut code, RECEIVER_SECOND, 99);
    expect_memory(&mut code, RECEIVER_SECOND, 99);
    for address in [RECEIVER_FIRST, RECEIVER_FIRST + PAGE_SIZE, RECEIVER_SECOND] {
        unmap(&mut code, address);
    }
    code.call(Call::Exit, 32, 0);
    code.finish()
}

pub(super) fn capacity_sender() -> Program {
    let mut code = Program::new();
    code.call(Call::MemoryCreate, 1, 0);
    code.expect_status(0);
    code.emit(&[0x49, 0x89, 0xd4]); // mov r12,rdx
    code.emit(&[0x4d, 0x89, 0xe5]); // mov r13,r12: delegate the same object twice
    message(&mut code);
    code.immediate(&[0x49, 0xbe], READ_MAP);
    code.immediate(&[0x49, 0xbf], READ_MAP);
    code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp
    code.immediate(&[0x48, 0xbe], 2);
    invoke(&mut code, Call::SendCaps);
    code.expect_status(Error::OutOfResources.status());
    close(&mut code, false);

    message(&mut code);
    code.emit(&[0x48, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], 0);
    invoke(&mut code, Call::Send);
    code.expect_status(0);
    code.call(Call::Exit, 33, 0);
    code.finish()
}

pub(super) fn capacity_receiver() -> Program {
    let mut code = Program::new();
    code.emit(&[0x48, 0x89, 0xef]);
    code.immediate(&[0x48, 0xbe], 0);
    invoke(&mut code, Call::Receive);
    code.expect_status(0);
    code.expect_value(0);
    for register in [0xe4, 0xed, 0xf6] {
        code.emit(&[0x4d, 0x85, register]); // test r12/r13/r14 against itself
        code.failure_jump();
    }
    expect_message(&mut code);
    code.call(Call::Exit, 34, 0);
    code.finish()
}

pub(super) fn fault_sender() -> Program {
    let mut code = Program::new();
    code.call(Call::MemoryCreate, 1, 0);
    code.expect_status(0);
    code.emit(&[0x49, 0x89, 0xd4]); // mov r12,rdx
    code.immediate(&[0x49, 0xbe], READ_MAP);
    code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp: endpoint
    code.immediate(&[0x48, 0xbe], 1);
    invoke(&mut code, Call::SendCaps);
    code.expect_status(0);
    close(&mut code, false);
    code.call(Call::Exit, 41, 0);
    code.finish()
}

pub(super) fn fault_receiver() -> Program {
    let mut code = Program::new();
    code.emit(&[0x48, 0x89, 0xef]); // mov rdi,rbp: endpoint
    code.immediate(&[0x48, 0xbe], 1);
    invoke(&mut code, Call::Receive);
    code.expect_status(0);
    code.expect_value(0);
    code.emit(&[0x49, 0x83, 0xfe, 1]); // cmp r14,1: received capability count
    code.failure_jump();
    map(&mut code, false, RECEIVER_FIRST, 0, false);
    code.expect_status(0);
    close(&mut code, false);
    write_memory(&mut code, RECEIVER_FIRST, 1);
    code.emit(&[0x0f, 0x0b]); // ud2: a writable mapping must fail the expected #PF check
    code.finish()
}

fn invoke(code: &mut Program, call: Call) {
    code.emit(&[0xb8]);
    code.emit(&(call as u32).to_le_bytes());
    code.emit(&[0xcd, 0x80]);
}

fn memory_handle(code: &mut Program, second: bool) {
    code.emit(&[0x4c, 0x89, if second { 0xef } else { 0xe7 }]); // mov rdi,r13/r12
}

fn map(code: &mut Program, second: bool, address: u64, page: u32, writable: bool) {
    memory_handle(code, second);
    code.immediate(&[0x48, 0xbe], address);
    code.emit(&[0xba]); // mov edx,page
    code.emit(&page.to_le_bytes());
    code.emit(&[0x41, 0xb8]); // mov r8d,flags
    code.emit(&u32::from(writable).to_le_bytes());
    invoke(code, Call::MemoryMap);
}

fn close(code: &mut Program, second: bool) {
    memory_handle(code, second);
    code.immediate(&[0x48, 0xbe], 0);
    invoke(code, Call::Close);
    code.expect_status(0);
}

fn unmap(code: &mut Program, address: u64) {
    code.call(Call::MemoryUnmap, address, 0);
    code.expect_status(0);
}

fn notification(code: &mut Program, call: Call) {
    code.emit(&[0x48, 0x89, 0xdf]); // mov rdi,rbx: notification
    code.immediate(&[0x48, 0xbe], 0);
    invoke(code, call);
    code.expect_status(0);
}

fn message(code: &mut Program) {
    for (index, word) in MESSAGE.into_iter().enumerate() {
        code.immediate(&[0x49, 0xb8 + index as u8], word); // mov r8..r11,imm64
    }
}

fn expect_message(code: &mut Program) {
    for (index, word) in MESSAGE.into_iter().enumerate() {
        code.immediate(&[0x48, 0xb8], word);
        code.emit(&[0x49, 0x39, 0xc0 + index as u8]); // cmp r8..r11,rax
        code.failure_jump();
    }
}

fn write_memory(code: &mut Program, address: u64, value: u64) {
    code.immediate(&[0x48, 0xb9], address);
    code.immediate(&[0x48, 0xb8], value);
    code.emit(&[0x48, 0x89, 0x01]); // mov [rcx],rax
}

fn expect_memory(code: &mut Program, address: u64, value: u64) {
    code.immediate(&[0x48, 0xb9], address);
    code.immediate(&[0x48, 0xb8], value);
    code.emit(&[0x48, 0x39, 0x01]); // cmp [rcx],rax
    code.failure_jump();
}
