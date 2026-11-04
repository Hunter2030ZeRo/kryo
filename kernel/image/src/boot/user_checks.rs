use super::memory::BootFramePool;
use super::vm_backend::{KernelMemory, UserSpace, check_preparation_failure, new_space};
use core::cell::RefCell;
use kryo_core::mm::frame::{FrameAllocError, OwnedFrame, PhysRange};
use kryo_core::mm::vm::Permissions;
use kryo_hal::{DirectMap, UserOutcome, UserReason, run_user};

const CODE: u64 = 0x40_0000;
const DATA: u64 = 0x60_0000;
const STACK: u64 = 0x80_0000;
const STACK_TOP: u64 = STACK + 4096;

pub(super) fn run(pool: &'static BootFramePool, direct: DirectMap, physical_end: u64) {
    let before = pool.stats();
    let heap_before = crate::heap::stats();
    let reserved = reserve_low_frames(pool);
    let mut region = super::platform::take_region();
    // This is the sole kernel alias manager, initialized before any child root.
    let kernel = RefCell::new(
        unsafe { KernelMemory::new(pool, direct, physical_end, &region) }
            .expect("Cannot initialize kernel alias policy"),
    );

    check_preparation_failure(pool, &kernel);
    check_distinct_spaces(pool, &kernel);
    check_fault(
        pool,
        &kernel,
        "kernel-read",
        load_exit(0xffff_ffff_8000_0000),
        ExpectedFault::page(5, 0xffff_ffff_8000_0000),
        Permissions::ReadWrite,
    );
    let alias = direct.frame_address(0x20_0000).expect("Invalid HHDM probe");
    check_fault(
        pool,
        &kernel,
        "hhdm-read",
        load_exit(alias),
        ExpectedFault::page(5, alias),
        Permissions::ReadWrite,
    );
    check_fault(
        pool,
        &kernel,
        "rx-write",
        write_byte(CODE),
        ExpectedFault::page(7, CODE),
        Permissions::ReadWrite,
    );
    check_fault(
        pool,
        &kernel,
        "ro-write",
        write_byte(DATA),
        ExpectedFault::page(7, DATA),
        Permissions::ReadOnly,
    );
    check_fault(
        pool,
        &kernel,
        "nx-execute",
        jump(DATA),
        ExpectedFault::page(21, DATA),
        Permissions::ReadWrite,
    );
    let mut guard = Program::new();
    guard.instruction(&[0x48, 0xbc], STACK); // mov rsp, immediate
    guard.bytes(&[0x50]); // push rax
    check_fault(
        pool,
        &kernel,
        "stack-guard",
        guard,
        ExpectedFault::page(6, STACK - 8),
        Permissions::ReadWrite,
    );
    check_fault(
        pool,
        &kernel,
        "privileged-cli",
        Program::from(&[0xfa]),
        ExpectedFault::exception(13),
        Permissions::ReadWrite,
    );
    check_fault(
        pool,
        &kernel,
        "invalid-opcode",
        Program::from(&[0x0f, 0x0b]),
        ExpectedFault::exception(6),
        Permissions::ReadWrite,
    );
    check_fault(
        pool,
        &kernel,
        "unsupported-fpu",
        Program::from(&[0xd9, 0xee]),
        ExpectedFault::exception(7),
        Permissions::ReadWrite,
    );

    check_fault(
        pool,
        &kernel,
        "port-io",
        Program::from(&[0xb0, 0x7a, 0xe6, 0xe9]),
        ExpectedFault::exception(13),
        Permissions::ReadWrite,
    );
    check_entry_barriers(pool, &kernel);
    let mut direction_flag = Program::from(&[0xfd]);
    direction_flag.bytes(exit(42).as_bytes());
    let mut flagged = make_space(pool, &kernel, &direction_flag, Permissions::ReadWrite, 0);
    assert_exit(execute(&mut flagged), 42);
    flagged.close().expect("Cannot close DF test space");
    let mut survivor = make_space(pool, &kernel, &exit(73), Permissions::ReadWrite, 0);
    assert_exit(execute(&mut survivor), 73);
    survivor.close().expect("Cannot close surviving user space");
    super::task_checks::run(pool, direct, &kernel, &mut region);
    drop((flagged, survivor));
    assert_eq!(
        kernel.borrow().transient_heap_usage(),
        (heap_before.allocations, heap_before.bytes)
    );
    let (retained, retained_pages) = kernel.into_inner().retain_tables();
    for frame in reserved.into_iter().flatten() {
        // Reserved frames were never exposed to user mappings or devices.
        unsafe { pool.release(frame) }.expect("Cannot release test reservation");
    }
    assert_eq!(
        pool.stats().free + retained_pages,
        before.free + heap_before.backing_pages
    );
    let _ = kryo_hal::print(format_args!(
        "USER: isolation OK cases=16 kernel_tables={} free={}\n",
        retained,
        pool.stats().free,
    ));
}

fn check_distinct_spaces(pool: &'static BootFramePool, kernel: &RefCell<KernelMemory>) {
    let mut first = make_space(pool, kernel, &load_exit(DATA), Permissions::ReadWrite, 42);
    let mut second = make_space(pool, kernel, &load_exit(DATA), Permissions::ReadWrite, 84);
    assert_ne!(first.root_frame().unwrap(), second.root_frame().unwrap());
    assert_exit(execute(&mut first), 42);
    assert_exit(execute(&mut second), 84);
    assert_exit(execute(&mut first), 42);
    assert!(first.map_page(CODE, Permissions::ReadOnly, &[]).is_err());
    assert!(first.map_page(0, Permissions::ReadWrite, &[]).is_err());
    assert!(
        first
            .map_page(0xffff_8000_0000_0000, Permissions::ReadWrite, &[])
            .is_err()
    );
    assert!(Permissions::new(true, true).is_none());
    let mut data = [0; 8];
    first
        .read_page(DATA, 0, &mut data)
        .expect("Cannot inspect private data");
    assert_eq!(u64::from_le_bytes(data), 42);
    first
        .unmap_page(DATA)
        .expect("Cannot unmap owned user data");
    assert!(first.read_page(DATA, 0, &mut data).is_err());
    first.close().expect("Cannot close first user space");
    second.close().expect("Cannot close second user space");
    kryo_hal::emergency_write_str("USER: separate address spaces OK\n");
}

struct ExpectedFault {
    vector: u64,
    error: u64,
    address: Option<u64>,
}

impl ExpectedFault {
    fn page(error: u64, address: u64) -> Self {
        Self {
            vector: 14,
            error,
            address: Some(address),
        }
    }
    fn exception(vector: u64) -> Self {
        Self {
            vector,
            error: 0,
            address: None,
        }
    }
}

fn check_entry_barriers(pool: &'static BootFramePool, kernel: &RefCell<KernelMemory>) {
    for (name, opcode, vectors) in [
        ("syscall", [0x0f, 0x05], &[6][..]),
        ("sysenter", [0x0f, 0x34], &[6, 13][..]),
    ] {
        let mut space = make_space(
            pool,
            kernel,
            &Program::from(&opcode),
            Permissions::ReadWrite,
            0,
        );
        let outcome = execute(&mut space);
        assert_eq!(outcome.reason, UserReason::Fault);
        assert!(vectors.contains(&outcome.vector));
        assert_eq!(outcome.error, 0);
        space.close().expect("Cannot close entry barrier test");
        let _ = kryo_hal::print(format_args!(
            "USER fault: {} vector={}\n",
            name, outcome.vector
        ));
    }
}

fn check_fault(
    pool: &'static BootFramePool,
    kernel: &RefCell<KernelMemory>,
    name: &str,
    program: Program,
    expected: ExpectedFault,
    data_access: Permissions,
) {
    let mut space = make_space(pool, kernel, &program, data_access, 0);
    let outcome = execute(&mut space);
    assert_eq!(outcome.reason, UserReason::Fault);
    assert_eq!(outcome.vector, expected.vector);
    assert_eq!(outcome.error & 0x17, expected.error);
    if let Some(address) = expected.address {
        assert_eq!(outcome.address, address);
    }
    space.close().expect("Cannot close faulted user space");
    let _ = kryo_hal::print(format_args!(
        "USER fault: {} vector={} error={:#x} address={:#x}\n",
        name, outcome.vector, outcome.error, outcome.address,
    ));
}

fn make_space<'kernel>(
    pool: &'static BootFramePool,
    kernel: &'kernel RefCell<KernelMemory>,
    program: &Program,
    data_access: Permissions,
    value: u64,
) -> UserSpace<'kernel> {
    let mut space = new_space(pool, kernel).expect("Cannot create user address space");
    space
        .map_page(CODE, Permissions::ReadExecute, program.as_bytes())
        .expect("Cannot map user code");
    space
        .map_page(DATA, data_access, &value.to_le_bytes())
        .expect("Cannot map user data");
    space
        .map_page(STACK, Permissions::ReadWrite, &[])
        .expect("Cannot map user stack");
    space
}

fn execute(space: &mut UserSpace<'_>) -> UserOutcome {
    // The exclusive run borrow pins every backing frame and table until the
    // trampoline restores the boot CR3. Code is scalar and has bounded paths.
    space
        .with_root(|root| unsafe { run_user(root, CODE, STACK_TOP) })
        .expect("User space was active outside run")
        .expect("Cannot enter user mode")
}

fn assert_exit(outcome: UserOutcome, status: u64) {
    assert_eq!(outcome.reason, UserReason::Exited);
    assert_eq!(outcome.status, status);
}

fn reserve_low_frames(pool: &'static BootFramePool) -> [Option<OwnedFrame<'static>>; 512] {
    let mut reserved = [const { None }; 512];
    let low = PhysRange::from_base_len(0, 0x20_0000).unwrap();
    // Allocation is cursor-based; high-frame checks may have moved that cursor.
    // Select this range explicitly so the VM failure fixture keeps its premise.
    for slot in &mut reserved {
        match pool.allocate_in(low) {
            Ok(frame) => *slot = Some(frame),
            Err(FrameAllocError::OutOfMemory) => return reserved,
            Err(error) => panic!("Cannot reserve low frames: {:?}", error),
        }
    }
    reserved
}

struct Program {
    bytes: [u8; 64],
    length: usize,
}

impl Program {
    fn new() -> Self {
        Self {
            bytes: [0; 64],
            length: 0,
        }
    }
    fn from(bytes: &[u8]) -> Self {
        let mut program = Self::new();
        program.bytes(bytes);
        program
    }
    fn bytes(&mut self, bytes: &[u8]) {
        let end = self.length + bytes.len();
        self.bytes[self.length..end].copy_from_slice(bytes);
        self.length = end;
    }
    fn instruction(&mut self, opcode: &[u8], immediate: u64) {
        self.bytes(opcode);
        self.bytes(&immediate.to_le_bytes());
    }
    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

fn exit(status: u32) -> Program {
    let mut program = Program::new();
    program.bytes(&[0xbf]); // mov edi, immediate
    program.bytes(&status.to_le_bytes());
    program.bytes(&[0xcd, 0x80]);
    program
}

fn load_exit(address: u64) -> Program {
    let mut program = Program::new();
    program.instruction(&[0x48, 0xb8], address); // mov rax, immediate
    program.bytes(&[0x48, 0x8b, 0x38, 0xcd, 0x80]); // mov rdi, [rax]; int 0x80
    program
}

fn write_byte(address: u64) -> Program {
    let mut program = Program::new();
    program.instruction(&[0x48, 0xb8], address);
    program.bytes(&[0xc6, 0x00, 0x00]); // mov byte ptr [rax], 0
    program.bytes(exit(0).as_bytes());
    program
}

fn jump(address: u64) -> Program {
    let mut program = Program::new();
    program.instruction(&[0x48, 0xb8], address);
    program.bytes(&[0xff, 0xe0]); // jmp rax
    program
}
