use kryo_interfaces::syscall::{Call, Error, TaskStatus};

pub(super) const CODE: u64 = 0x40_0000;
pub(super) const DATA: u64 = 0x60_0000;
pub(super) const STACK: u64 = 0x80_0000;
pub(super) const SEED_A: u64 = 0x1234_5678;
pub(super) const SEED_B: u64 = 0x1357_2468;

pub(super) struct Program {
    bytes: [u8; 2048],
    length: usize,
    failures: [usize; 64],
    failure_count: usize,
}

impl Program {
    pub(super) fn new() -> Self {
        Self {
            bytes: [0; 2048],
            length: 0,
            failures: [0; 64],
            failure_count: 0,
        }
    }
    pub(super) fn emit(&mut self, bytes: &[u8]) {
        let end = self.length + bytes.len();
        self.bytes[self.length..end].copy_from_slice(bytes);
        self.length = end;
    }
    pub(super) fn immediate(&mut self, opcode: &[u8], value: u64) {
        self.emit(opcode);
        self.emit(&value.to_le_bytes());
    }
    pub(super) fn call(&mut self, call: Call, argument: u64, extra: u64) {
        self.emit(&[0xb8]);
        self.emit(&(call as u32).to_le_bytes());
        self.immediate(&[0x48, 0xbf], argument);
        self.immediate(&[0x48, 0xbe], extra);
        self.emit(&[0xcd, 0x80]);
    }
    pub(super) fn call_saved_handle(&mut self, call: Call) {
        self.emit(&[0xb8]);
        self.emit(&(call as u32).to_le_bytes());
        self.emit(&[0x4c, 0x89, 0xff, 0xcd, 0x80]); // mov rdi,r15; int80
    }
    pub(super) fn expect_status(&mut self, status: u64) {
        self.emit(&[0x48, 0x3d]); // cmp rax, sign-extended imm32
        self.emit(&(status as u32).to_le_bytes());
        self.failure_jump();
    }
    pub(super) fn expect_value(&mut self, value: u32) {
        self.emit(&[0x48, 0x81, 0xfa]); // cmp rdx, imm32
        self.emit(&value.to_le_bytes());
        self.failure_jump();
    }
    pub(super) fn failure_jump(&mut self) {
        self.failure_branch(0x85);
    }
    pub(super) fn failure_if_zero(&mut self) {
        self.failure_branch(0x84);
    }
    fn failure_branch(&mut self, opcode: u8) {
        self.emit(&[0x0f, opcode]);
        self.failures[self.failure_count] = self.length;
        self.failure_count += 1;
        self.emit(&[0; 4]);
    }
    pub(super) fn finish(mut self) -> Self {
        let failure = self.length;
        self.call(Call::Exit, 255, 0);
        self.emit(&[0x0f, 0x0b]);
        for index in 0..self.failure_count {
            let patch = self.failures[index];
            let distance = i32::try_from(failure as isize - (patch + 4) as isize).unwrap();
            self.bytes[patch..patch + 4].copy_from_slice(&distance.to_le_bytes());
        }
        self
    }
    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

pub(super) fn counter() -> Program {
    let mut code = Program::new();
    let start = code.length;
    code.emit(&[0x49, 0xff, 0xc2, 0x49, 0xff, 0xc6]); // inc r10; inc r14
    code.emit(&[0x48, 0xff, 0x03]); // inc qword [rbx]
    code.emit(&[0x49, 0x81, 0xfc]); // cmp r12, seed
    code.emit(&(SEED_A as u32).to_le_bytes());
    code.failure_jump();
    code.emit(&[0x49, 0x81, 0xfd]); // cmp r13, seed
    code.emit(&(SEED_B as u32).to_le_bytes());
    code.failure_jump();
    code.emit(&[0xe9]);
    let displacement = i32::try_from(start as isize - (code.length + 4) as isize).unwrap();
    code.emit(&displacement.to_le_bytes());
    code.finish()
}

pub(super) fn controller(
    inspect: u64,
    control: u64,
    inspect_right: u64,
    control_right: u64,
) -> Program {
    let mut code = Program::new();
    code.call(Call::TaskStop, inspect, 99);
    code.expect_status(Error::PermissionDenied.status());
    code.call(Call::Duplicate, inspect, control_right);
    code.expect_status(Error::PermissionDenied.status());
    code.call(Call::Duplicate, control, inspect_right);
    code.expect_status(0);
    code.emit(&[0x49, 0x89, 0xd7]); // mov r15,rdx: retain the returned opaque handle
    code.call_saved_handle(Call::TaskStatus);
    code.expect_status(0);
    code.expect_value(TaskStatus::Runnable as u32);
    code.call_saved_handle(Call::TaskStop);
    code.expect_status(Error::PermissionDenied.status());
    code.call_saved_handle(Call::Close);
    code.expect_status(0);
    code.call_saved_handle(Call::TaskStatus);
    code.expect_status(Error::InvalidHandle.status());
    code.call(Call::Yield, 0, 0);
    code.expect_status(0);
    code.call(Call::TaskStop, control, 33);
    code.expect_status(0);
    code.call(Call::TaskStatus, inspect, 0);
    code.expect_status(0);
    code.expect_value(TaskStatus::Exited as u32);
    code.call(Call::Close, inspect, 0);
    code.expect_status(0);
    code.call(Call::Close, control, 0);
    code.expect_status(0);
    code.call(Call::Exit, 7, 0);
    code.finish()
}

pub(super) fn bad_stack() -> Program {
    let mut code = Program::new();
    code.emit(&[0x31, 0xe4, 0xeb, 0xfe]); // xor esp,esp; jmp $
    code
}
