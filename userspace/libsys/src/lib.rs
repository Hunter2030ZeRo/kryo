#![no_std]

//! Small wrappers for Kryo's experimental version-zero native calls.

use core::arch::asm;
pub use kryo_interfaces::startup::ObjectKind;
use kryo_interfaces::syscall::Call;
pub use kryo_interfaces::syscall::TaskStatus;

pub const INSPECT: u64 = 1;
pub const CONTROL: u64 = 2;
pub const DUP: u64 = 4;
pub const SEND: u64 = 8;
pub const RECEIVE: u64 = 16;
pub const WAIT: u64 = 32;
pub const SIGNAL: u64 = 64;
pub const READ: u64 = 128;
pub const WRITE: u64 = 256;
pub const MAP: u64 = 512;
pub const TRANSFER: u64 = 1024;
pub const EXECUTE: u64 = 2048;
pub const CREATE: u64 = 4096;
pub const ASSIGN: u64 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Resource {
    SharedPages = 0,
    Handles = 1,
    Waits = 2,
    Threads = 3,
    Mappings = 4,
    Tables = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum BudgetQuery {
    Limit = 0,
    Used = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(u64);

impl Handle {
    pub const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An actual native-call failure, preserving the kernel's negative status.
    Kernel(i64),
    /// The wrapper rejected an argument before entering the kernel.
    InvalidArgument,
    InvalidResponse,
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant {
    pub handle: Handle,
    pub rights: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandleInfo {
    pub kind: ObjectKind,
    pub rights: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupGrant {
    pub role: u64,
    pub kind: ObjectKind,
    pub rights: u64,
    pub handle: Handle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    pub service: u64,
    pub instance: u64,
    pub argument: u64,
    entries: [Option<StartupGrant>; kryo_interfaces::startup::MAX_GRANTS],
    count: usize,
}

impl Startup {
    pub fn grant(&self, role: u64) -> Option<StartupGrant> {
        self.grants().find(|entry| entry.role == role)
    }

    pub fn grants(&self) -> impl Iterator<Item = StartupGrant> + '_ {
        self.entries[..self.count].iter().flatten().copied()
    }
}

pub type Message = [u64; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyToken(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Received {
    pub message: Message,
    pub reply: Option<ReplyToken>,
    pub handles: [Option<Handle>; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcError {
    pub error: Error,
    /// Once accepted, retrying a failed call can repeat its service-side work.
    pub accepted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitReason {
    Exited,
    Fault,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessExit {
    pub code: u64,
    pub reason: ExitReason,
}

pub fn clock() -> Result<u64, Error> {
    invoke(Call::ClockRead, 0, 0)
}

pub fn sleep_until(deadline: u64) -> Result<(), Error> {
    invoke(Call::SleepUntil, deadline, 0).map(|_| ())
}

pub fn sleep_for(duration_ns: u64) -> Result<(), Error> {
    let deadline = clock()?.checked_add(duration_ns).ok_or(Error::Overflow)?;
    sleep_until(deadline)
}

pub fn close(handle: Handle) -> Result<(), Error> {
    invoke(Call::Close, handle.raw(), 0).map(|_| ())
}

pub fn duplicate(handle: Handle, rights: u64) -> Result<Handle, Error> {
    handle_result(invoke(Call::Duplicate, handle.raw(), rights)?)
}

pub fn handle_info(handle: Handle) -> Result<HandleInfo, Error> {
    let status;
    let kind: u64;
    let rights;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::HandleInfo as u64 => status,
            in("rdi") handle.raw(),
            lateout("rdx") kind,
            lateout("r8") rights,
            lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status)?;
    let kind = ObjectKind::try_from(kind).map_err(|_| Error::InvalidResponse)?;
    if rights & !kind.allowed_rights() != 0 {
        return Err(Error::InvalidResponse);
    }
    Ok(HandleInfo { kind, rights })
}

pub fn manifest_create(service: u64, instance: u64, argument: u64) -> Result<Handle, Error> {
    let status;
    let result;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::ManifestCreate as u64 => status,
            in("rdi") kryo_interfaces::startup::VERSION,
            in("rsi") service,
            inlateout("rdx") instance => result,
            inlateout("r8") argument => _,
            lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status)?;
    handle_result(result)
}

pub fn manifest_grant(
    manifest: Handle,
    role: u64,
    grant: Grant,
    kind: ObjectKind,
) -> Result<(), Error> {
    let status;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::ManifestGrant as u64 => status,
            in("rdi") manifest.raw(),
            in("rsi") role,
            inlateout("rdx") grant.handle.raw() => _,
            inlateout("r8") kind as u64 => _,
            inlateout("r9") grant.rights => _,
            lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status)
}

pub fn spawn_manifest(image: Handle, manifest: Handle) -> Result<Handle, Error> {
    handle_result(invoke(Call::SpawnManifest, image.raw(), manifest.raw())?)
}

pub fn manifest_budget(manifest: Handle, budget: Handle) -> Result<(), Error> {
    invoke(Call::ManifestBudget, manifest.raw(), budget.raw()).map(|_| ())
}

pub fn budget_info(budget: Handle, resource: Resource, query: BudgetQuery) -> Result<u64, Error> {
    let status;
    let result;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::BudgetInfo as u64 => status,
            in("rdi") budget.raw(),
            in("rsi") resource as u64,
            inlateout("rdx") query as u64 => result,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status)?;
    Ok(result)
}

pub fn budget_self(resource: Resource, query: BudgetQuery) -> Result<u64, Error> {
    invoke(Call::BudgetSelf, resource as u64, query as u64)
}

pub fn memory_create(pages: usize) -> Result<Handle, Error> {
    handle_result(invoke(Call::MemoryCreate, pages as u64, 0)?)
}

/// # Safety
/// The caller must own this vacant address range and preserve every runtime
/// mapping/reference that can overlap it, including concurrent thread accesses.
pub unsafe fn memory_map(
    memory: Handle,
    address: u64,
    page: usize,
    writable: bool,
) -> Result<(), Error> {
    let status;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::MemoryMap as u64 => status,
            in("rdi") memory.raw(),
            in("rsi") address,
            inlateout("rdx") page as u64 => _,
            inlateout("r8") u64::from(writable) => _,
            lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status)
}

/// # Safety
/// No Rust reference or concurrent access may remain into this page. The caller
/// must own its mapping lifetime and must not remove runtime/code/stack storage.
pub unsafe fn memory_unmap(address: u64) -> Result<(), Error> {
    invoke(Call::MemoryUnmap, address, 0).map(|_| ())
}

pub fn startup_word(record: Handle, index: usize) -> Result<u64, Error> {
    invoke(Call::StartupRead, record.raw(), index as u64)
}

pub fn startup(record: Handle) -> Result<Startup, Error> {
    read_startup(|index| startup_word(record, index))
}

pub fn create_endpoint(factory: Handle) -> Result<Handle, Error> {
    handle_result(invoke(Call::EndpointCreate, factory.raw(), 0)?)
}

pub fn create_notification(factory: Handle) -> Result<Handle, Error> {
    handle_result(invoke(Call::NotificationCreate, factory.raw(), 0)?)
}

pub fn close_endpoint(endpoint: Handle) -> Result<(), Error> {
    invoke(Call::EndpointClose, endpoint.raw(), 0).map(|_| ())
}

pub fn signal(notification: Handle) -> Result<(), Error> {
    invoke(Call::Signal, notification.raw(), 0).map(|_| ())
}

pub fn send(endpoint: Handle, message: Message) -> Result<(), IpcError> {
    ipc(Call::Send, endpoint, 0, 0, message, &[]).map(|_| ())
}

pub fn send_until(endpoint: Handle, message: Message, deadline: u64) -> Result<(), IpcError> {
    ipc(Call::SendUntil, endpoint, 0, deadline, message, &[]).map(|_| ())
}

pub fn call(endpoint: Handle, message: Message) -> Result<Message, IpcError> {
    ipc(Call::Call, endpoint, 0, 0, message, &[]).map(|result| result.message)
}

pub fn call_until(endpoint: Handle, message: Message, deadline: u64) -> Result<Message, IpcError> {
    ipc(Call::CallUntil, endpoint, 0, deadline, message, &[]).map(|result| result.message)
}

pub fn send_caps(endpoint: Handle, message: Message, grants: &[Grant]) -> Result<(), IpcError> {
    send_grants(Call::SendCaps, endpoint, message, grants, 0).map(|_| ())
}

pub fn send_caps_until(
    endpoint: Handle,
    message: Message,
    grants: &[Grant],
    deadline: u64,
) -> Result<(), IpcError> {
    send_grants(Call::SendCapsUntil, endpoint, message, grants, deadline).map(|_| ())
}

pub fn call_caps(
    endpoint: Handle,
    message: Message,
    grants: &[Grant],
) -> Result<Message, IpcError> {
    send_grants(Call::CallCaps, endpoint, message, grants, 0).map(|result| result.message)
}

pub fn call_caps_until(
    endpoint: Handle,
    message: Message,
    grants: &[Grant],
    deadline: u64,
) -> Result<Message, IpcError> {
    send_grants(Call::CallCapsUntil, endpoint, message, grants, deadline)
        .map(|result| result.message)
}

pub fn receive(endpoint: Handle, capacity: usize) -> Result<Received, IpcError> {
    receive_message(Call::Receive, endpoint, capacity, 0)
}

pub fn receive_until(
    endpoint: Handle,
    capacity: usize,
    deadline: u64,
) -> Result<Received, IpcError> {
    receive_message(Call::ReceiveUntil, endpoint, capacity, deadline)
}

/// Starts a bounded receive interval when the kernel admits the operation.
pub fn receive_for(
    endpoint: Handle,
    capacity: usize,
    duration_ns: u64,
) -> Result<Received, IpcError> {
    receive_message(Call::ReceiveFor, endpoint, capacity, duration_ns)
}

pub fn reply(endpoint: Handle, token: ReplyToken, message: Message) -> Result<(), Error> {
    ipc(Call::Reply, endpoint, token.0, 0, message, &[])
        .map(|_| ())
        .map_err(|failure| failure.error)
}

pub fn yield_now() -> Result<(), Error> {
    invoke(Call::Yield, 0, 0).map(|_| ())
}

pub fn wait_notification(notification: Handle) -> Result<(), Error> {
    invoke(Call::Wait, notification.raw(), 0).map(|_| ())
}

pub fn wait_notification_until(notification: Handle, deadline: u64) -> Result<(), Error> {
    let status;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::WaitUntil as u64 => status,
            in("rdi") notification.raw(),
            inlateout("rdx") deadline => _,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status)
}

pub fn device_read(device: Handle, offset: u64) -> Result<u32, Error> {
    u32::try_from(invoke(Call::DeviceRead, device.raw(), offset)?)
        .map_err(|_| Error::InvalidResponse)
}

pub fn device_write(device: Handle, offset: u64, value: u32) -> Result<(), Error> {
    let status;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::DeviceWrite as u64 => status,
            in("rdi") device.raw(),
            in("rsi") offset,
            inlateout("rdx") u64::from(value) => _,
            lateout("r8") _, lateout("r9") _, lateout("r10") _, lateout("r11") _,
            options(nostack),
        );
    }
    check(status)
}

/// # Safety
/// Retain a valid user TLS area and preserve the calling runtime's TLS contract.
/// Kernel address validation alone does not establish a Rust object lifetime.
pub unsafe fn set_tls(base: u64) -> Result<(), Error> {
    invoke(Call::SetTls, base, 0).map(|_| ())
}

pub fn task_status(handle: Handle) -> Result<u64, Error> {
    invoke(Call::TaskStatus, handle.raw(), 0)
}

pub fn task_stop(handle: Handle, code: u64) -> Result<(), Error> {
    invoke(Call::TaskStop, handle.raw(), code).map(|_| ())
}

pub fn cancel_wait(handle: Handle) -> Result<bool, Error> {
    match invoke(Call::CancelWait, handle.raw(), 0)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::InvalidResponse),
    }
}

pub fn spawn(image: Handle, grants: &[Grant]) -> Result<Handle, Error> {
    if grants.len() > 2 {
        return Err(Error::InvalidArgument);
    }
    let first = grants.first().map_or(0, |grant| grant.handle.raw());
    let second = grants.get(1).map_or(0, |grant| grant.handle.raw());
    let first_rights = grants.first().map_or(0, |grant| grant.rights);
    let second_rights = grants.get(1).map_or(0, |grant| grant.rights);
    let status;
    let result;
    // Explicit high-register operands let the compiler retain its own callee-
    // saved values around this non-C register interface.
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::Spawn as u64 => status,
            in("rdi") image.raw(),
            in("rsi") grants.len() as u64,
            lateout("rdx") result,
            inlateout("r12") first => _,
            inlateout("r13") second => _,
            inlateout("r14") first_rights => _,
            inlateout("r15") second_rights => _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    check(status)?;
    Handle::from_raw(result).ok_or(Error::InvalidResponse)
}

pub fn process_wait(process: Handle) -> Result<ProcessExit, Error> {
    wait(Call::ProcessWait, process, 0)
}

pub fn process_wait_until(process: Handle, deadline: u64) -> Result<ProcessExit, Error> {
    wait(Call::ProcessWaitUntil, process, deadline)
}

/// # Safety
/// The entry must implement the native thread bootstrap: RDI is the argument,
/// RSI is the new TLS address, RDX contains CPU feature flags, and RSP is aligned
/// to 16 bytes without a return address. Enter Rust through an assembly CALL and
/// never return. Retain shared argument/code/data lifetimes through join, and
/// synchronize all concurrent access. This does not establish Rust thread or TLS
/// runtime support; the kernel supplies only the stack, TLS page, and CPU state.
pub unsafe fn thread_create(
    entry: unsafe extern "C" fn() -> !,
    argument: u64,
) -> Result<Handle, Error> {
    let result = invoke(Call::ThreadCreate, entry as *const () as u64, argument)?;
    Handle::from_raw(result).ok_or(Error::InvalidResponse)
}

/// # Safety
/// All resources requiring destructors must be released before calling. This
/// exits the current thread directly and does not unwind its Rust stack.
pub unsafe fn thread_exit(code: u64) -> ! {
    unsafe {
        asm!(
            "int 0x80",
            "ud2",
            in("rax") Call::ThreadExit as u64,
            in("rdi") code,
            options(noreturn),
        );
    }
}

pub fn thread_join(thread: Handle) -> Result<ProcessExit, Error> {
    wait(Call::ThreadJoin, thread, 0)
}

pub fn thread_join_until(thread: Handle, deadline: u64) -> Result<ProcessExit, Error> {
    wait(Call::ThreadJoinUntil, thread, deadline)
}

pub fn debug_log(debug: Handle, event: u64, value: u64) -> Result<(), Error> {
    let status;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") Call::DebugLog as u64 => status,
            in("rdi") debug.raw(),
            in("rsi") event,
            inlateout("r8") value => _,
            lateout("rdx") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    check(status)
}

pub fn exit(code: u64) -> ! {
    unsafe {
        asm!(
            "int 0x80",
            "ud2",
            in("rax") Call::Exit as u64,
            in("rdi") code,
            options(noreturn),
        );
    }
}

fn wait(call: Call, control: Handle, deadline: u64) -> Result<ProcessExit, Error> {
    let status;
    let code;
    let reason;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") call as u64 => status,
            in("rdi") control.raw(),
            inlateout("rdx") deadline => code,
            lateout("r8") reason,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    check(status)?;
    let reason = match reason {
        0u64 => ExitReason::Exited,
        1 => ExitReason::Fault,
        2 => ExitReason::Stopped,
        _ => return Err(Error::InvalidResponse),
    };
    Ok(ProcessExit { code, reason })
}

fn invoke(call: Call, first: u64, second: u64) -> Result<u64, Error> {
    let status;
    let result;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") call as u64 => status,
            in("rdi") first,
            in("rsi") second,
            lateout("rdx") result,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    check(status)?;
    Ok(result)
}

fn handle_result(raw: u64) -> Result<Handle, Error> {
    Handle::from_raw(raw).ok_or(Error::InvalidResponse)
}

struct IpcResult {
    result: u64,
    message: Message,
    handles: [u64; 2],
    count: u64,
}

fn invalid_ipc(error: Error) -> IpcError {
    IpcError {
        error,
        accepted: false,
    }
}

fn send_grants(
    call: Call,
    endpoint: Handle,
    message: Message,
    grants: &[Grant],
    deadline: u64,
) -> Result<IpcResult, IpcError> {
    if !(1..=2).contains(&grants.len()) {
        return Err(invalid_ipc(Error::InvalidArgument));
    }
    ipc(
        call,
        endpoint,
        grants.len() as u64,
        deadline,
        message,
        grants,
    )
}

fn receive_message(
    call: Call,
    endpoint: Handle,
    capacity: usize,
    deadline: u64,
) -> Result<Received, IpcError> {
    if capacity > 2 {
        return Err(invalid_ipc(Error::InvalidArgument));
    }
    let result = ipc(call, endpoint, capacity as u64, deadline, [0; 4], &[])?;
    if result.count > capacity as u64 {
        return Err(invalid_ipc(Error::InvalidResponse));
    }
    let mut handles = [None; 2];
    for (index, slot) in handles.iter_mut().enumerate().take(result.count as usize) {
        *slot = Some(handle_result(result.handles[index]).map_err(invalid_ipc)?);
    }
    Ok(Received {
        message: result.message,
        reply: (result.result != 0).then_some(ReplyToken(result.result)),
        handles,
    })
}

fn ipc(
    call: Call,
    endpoint: Handle,
    argument: u64,
    deadline: u64,
    message: Message,
    grants: &[Grant],
) -> Result<IpcResult, IpcError> {
    let status;
    let result;
    let [mut first, mut second, mut third, mut fourth] = message;
    let mut first_handle = grants.first().map_or(0, |grant| grant.handle.raw());
    let mut second_handle = grants.get(1).map_or(0, |grant| grant.handle.raw());
    let mut count = grants.first().map_or(0, |grant| grant.rights);
    let second_rights = grants.get(1).map_or(0, |grant| grant.rights);
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") call as u64 => status,
            in("rdi") endpoint.raw(),
            in("rsi") argument,
            inlateout("rdx") deadline => result,
            inlateout("r8") first,
            inlateout("r9") second,
            inlateout("r10") third,
            inlateout("r11") fourth,
            inlateout("r12") first_handle,
            inlateout("r13") second_handle,
            inlateout("r14") count,
            inlateout("r15") second_rights => _,
            options(nostack),
        );
    }
    check(status).map_err(|error| IpcError {
        error,
        accepted: result != 0,
    })?;
    Ok(IpcResult {
        result,
        message: [first, second, third, fourth],
        handles: [first_handle, second_handle],
        count,
    })
}

fn check(status: u64) -> Result<(), Error> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Kernel(status as i64))
    }
}

fn read_startup(mut read: impl FnMut(usize) -> Result<u64, Error>) -> Result<Startup, Error> {
    use kryo_interfaces::startup::{ENTRY_WORDS, HEADER_WORDS, MAGIC, MAX_GRANTS, VERSION};

    if read(0)? != MAGIC || read(1)? != VERSION {
        return Err(Error::InvalidResponse);
    }
    let length = usize::try_from(read(2)?).map_err(|_| Error::InvalidResponse)?;
    let service = read(3)?;
    let instance = read(4)?;
    let argument = read(5)?;
    let count = usize::try_from(read(6)?).map_err(|_| Error::InvalidResponse)?;
    if service == 0
        || instance == 0
        || count > MAX_GRANTS
        || length != HEADER_WORDS + count * ENTRY_WORDS
    {
        return Err(Error::InvalidResponse);
    }
    let mut entries: [Option<StartupGrant>; MAX_GRANTS] = [None; MAX_GRANTS];
    for index in 0..count {
        let offset = HEADER_WORDS + index * ENTRY_WORDS;
        let role = read(offset)?;
        let kind = ObjectKind::try_from(read(offset + 1)?).map_err(|_| Error::InvalidResponse)?;
        let rights = read(offset + 2)?;
        let handle = handle_result(read(offset + 3)?)?;
        if role == 0
            || entries[..index]
                .iter()
                .flatten()
                .any(|entry| entry.role == role)
            || rights == 0
            || rights & !kind.allowed_rights() != 0
        {
            return Err(Error::InvalidResponse);
        }
        entries[index] = Some(StartupGrant {
            role,
            kind,
            rights,
            handle,
        });
    }
    Ok(Startup {
        service,
        instance,
        argument,
        entries,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kryo_interfaces::startup::{ENTRY_WORDS, HEADER_WORDS, MAGIC, MAX_GRANTS, VERSION};

    fn words() -> [u64; HEADER_WORDS + 2 * ENTRY_WORDS] {
        [
            MAGIC,
            VERSION,
            15,
            10,
            2,
            7,
            2,
            1,
            3,
            SEND,
            0x100000001,
            2,
            10,
            CREATE,
            0x100000002,
        ]
    }

    fn decode(words: &[u64]) -> Result<Startup, Error> {
        read_startup(|index| words.get(index).copied().ok_or(Error::InvalidResponse))
    }

    #[test]
    fn startup_record_preserves_named_attenuated_grants() {
        let record = decode(&words()).unwrap();
        assert_eq!(
            (record.service, record.instance, record.argument),
            (10, 2, 7)
        );
        assert_eq!(record.grants().count(), 2);
        assert_eq!(record.grant(1).unwrap().kind, ObjectKind::Endpoint);
        assert_eq!(record.grant(2).unwrap().rights, CREATE);
        assert_eq!(record.grant(2).unwrap().handle.raw(), 0x100000002);
        assert_eq!(record.grant(99), None);
    }

    #[test]
    fn startup_rejects_unknown_headers_and_invalid_lengths() {
        for index in [0, 1, 2, 3, 4, 6] {
            let mut invalid = words();
            invalid[index] = if index == 6 { MAX_GRANTS as u64 + 1 } else { 0 };
            assert_eq!(decode(&invalid), Err(Error::InvalidResponse));
        }
        assert_eq!(decode(&words()[..14]), Err(Error::InvalidResponse));
    }

    #[test]
    fn startup_rejects_duplicate_roles_bad_kinds_and_rights() {
        for (index, value) in [(7, 0), (11, 1), (8, 99), (9, 0), (9, 1 << 63), (10, 0)] {
            let mut invalid = words();
            invalid[index] = value;
            assert_eq!(decode(&invalid), Err(Error::InvalidResponse));
        }
    }

    #[test]
    fn startup_budget_assignment_requires_the_budget_kind() {
        let mut record = words();
        record[12] = ObjectKind::ResourceBudget as u64;
        record[13] = READ | ASSIGN | TRANSFER;
        let budget = decode(&record).unwrap().grant(2).unwrap();
        assert_eq!(budget.kind, ObjectKind::ResourceBudget);
        assert_eq!(budget.rights, READ | ASSIGN | TRANSFER);
        record[12] = ObjectKind::ChannelFactory as u64;
        assert_eq!(decode(&record), Err(Error::InvalidResponse));
    }
}
