//! A bounded, volatile file backend for the optional libc validation fixture.
//! It implements neither BetAFS nor a persistent namespace.

use core::sync::atomic::{AtomicBool, Ordering};
use kryo_interfaces::file::{self, OpenOptions, Operation, Request, Status};
use kryo_libsys::{self as sys, Grant, Handle, ObjectKind, ReceiveEvent};

const LOAN: u64 = 0x0300_0000;
const PAGE: usize = file::PAGE_SIZE as usize;
const CAPACITY: usize = 16 * 1024;
const OPEN_LIMIT: usize = 8;
const READ_ONLY_BYTES: &[u8] = b"read-only fixture\n";

#[derive(Clone, Copy, PartialEq, Eq)]
enum File {
    Probe,
    ReadOnly,
}

#[derive(Clone, Copy)]
struct Description {
    label: u64,
    file: File,
    position: u64,
    options: OpenOptions,
}

struct State {
    bytes: [u8; CAPACITY],
    length: usize,
    exists: bool,
    descriptions: [Option<Description>; OPEN_LIMIT],
    next_label: u64,
    namespace_open: bool,
    opens: usize,
    reads: usize,
    writes: usize,
    seeks: usize,
    closures: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            bytes: [0; CAPACITY],
            length: 0,
            exists: false,
            descriptions: [None; OPEN_LIMIT],
            next_label: file::NAMESPACE_LABEL + 1,
            namespace_open: true,
            opens: 0,
            reads: 0,
            writes: 0,
            seeks: 0,
            closures: 0,
        }
    }

    fn description(&self, label: u64) -> Option<usize> {
        self.descriptions
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.label == label))
    }

    fn retire(&mut self, label: u64) {
        if label == file::NAMESPACE_LABEL {
            self.namespace_open = false;
        } else if let Some(slot) = self.description(label) {
            self.descriptions[slot] = None;
            self.closures += 1;
        }
    }

    fn complete(&self) -> bool {
        !self.namespace_open && self.descriptions.iter().all(Option::is_none)
    }

    fn checked(&self) -> bool {
        self.opens >= 6
            && self.reads > 0
            && self.writes > 0
            && self.seeks > 0
            && self.closures == self.opens
    }

    fn size(&self, file: File) -> usize {
        match file {
            File::Probe => self.length,
            File::ReadOnly => READ_ONLY_BYTES.len(),
        }
    }

    fn data(&self, file: File) -> &[u8] {
        match file {
            File::Probe => &self.bytes[..self.length],
            File::ReadOnly => READ_ONLY_BYTES,
        }
    }
}

struct Completion {
    value: u64,
    gate: Option<Handle>,
    opened: Option<usize>,
}

impl Completion {
    const fn value(value: u64) -> Self {
        Self {
            value,
            gate: None,
            opened: None,
        }
    }
}

pub(super) fn serve(endpoint: Handle) -> Result<(), u64> {
    static CLAIMED: AtomicBool = AtomicBool::new(false);
    static mut STATE: State = State::new();
    if CLAIMED.swap(true, Ordering::AcqRel) {
        return Err(239);
    }
    // The sole successful claim owns this fixture's storage for process lifetime.
    let state = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    loop {
        let incoming = sys::receive(endpoint, 2).map_err(|_| 240u64)?;
        if incoming.event == ReceiveEvent::GateClosed {
            if incoming.message != [0; 4]
                || incoming.reply.is_some()
                || incoming.handles != [None; 2]
            {
                return Err(241);
            }
            state.retire(incoming.label);
            if state.complete() {
                if !state.checked() {
                    return Err(242);
                }
                sys::close(endpoint).map_err(|_| 243u64)?;
                return Ok(());
            }
            continue;
        }
        let handles = incoming.handles;
        let Some(reply) = incoming.reply else {
            close_all(handles)?;
            continue;
        };
        let count = handles.iter().flatten().count();
        let result = Request::decode(
            incoming.message,
            incoming.label == file::NAMESPACE_LABEL,
            count,
        )
        .and_then(|request| execute(endpoint, state, incoming.label, request, handles[0]));
        // All accepted CPU accesses end before grants close and the response publishes.
        close_all(handles)?;
        let (status, completion) = match result {
            Ok(completion) => (Status::Ok, completion),
            Err(status) => (status, Completion::value(0)),
        };
        let response = file::response(incoming.message[1], status, completion.value);
        let delivered = match completion.gate {
            Some(gate) => sys::reply_caps(
                endpoint,
                reply,
                response,
                &[Grant {
                    handle: gate,
                    rights: sys::SEND | sys::DUP | sys::TRANSFER,
                }],
            ),
            None => sys::reply(endpoint, reply, response),
        };
        if let Err(error) = delivered {
            if let Some(slot) = completion.opened {
                state.descriptions[slot] = None;
            }
            if !matches!(error, sys::Error::Kernel(-9 | -7)) {
                // A capability reply can fail preflight while its token remains live.
                match sys::reply(
                    endpoint,
                    reply,
                    file::response(incoming.message[1], Status::NoSpace, 0),
                ) {
                    Ok(()) | Err(sys::Error::Kernel(-9 | -7)) => {}
                    Err(_) => return Err(244),
                }
            }
        } else if completion.opened.is_some() {
            state.opens += 1;
        }
        if let Some(gate) = completion.gate {
            // The client and accepted operations are now the only external owners.
            sys::close(gate).map_err(|_| 245u64)?;
        }
    }
}

fn execute(
    endpoint: Handle,
    state: &mut State,
    label: u64,
    request: Request,
    memory: Option<Handle>,
) -> Result<Completion, Status> {
    if request.operation == Operation::Open {
        return open(endpoint, state, request, memory.ok_or(Status::Malformed)?);
    }
    let slot = state.description(label).ok_or(Status::BadFile)?;
    let mut description = state.descriptions[slot].ok_or(Status::BadFile)?;
    let result = match request.operation {
        Operation::Seek => {
            let position = file::seek_position(
                description.position,
                state.size(description.file) as u64,
                request.argument as i64,
                request.length,
            )?;
            description.position = position;
            state.seeks += 1;
            Completion::value(position)
        }
        Operation::Read => {
            if !description.options.readable() {
                return Err(Status::Denied);
            }
            let memory = memory.ok_or(Status::Malformed)?;
            map(memory, true)?;
            let position = usize::try_from(description.position).unwrap_or(usize::MAX);
            let data = state.data(description.file);
            let bytes = if position >= data.len() {
                0
            } else {
                (request.length as usize).min(data.len() - position)
            };
            for index in 0..bytes {
                // The private mapping and received grant retain the loan. The client
                // may access it concurrently, so do not create a shared-memory slice.
                unsafe {
                    ((LOAN + request.argument) as *mut u8)
                        .add(index)
                        .write_volatile(data[position + index]);
                }
            }
            unmap()?;
            description.position += bytes as u64;
            state.reads += 1;
            Completion::value(bytes as u64)
        }
        Operation::Write => {
            if !description.options.writable() || description.file == File::ReadOnly {
                return Err(Status::Denied);
            }
            let mut snapshot = [0; PAGE];
            copy_from(
                memory.ok_or(Status::Malformed)?,
                request.argument,
                &mut snapshot[..request.length as usize],
            )?;
            if request.length == 0 {
                return Ok(Completion::value(0));
            }
            let position = if description.options.flags & file::APPEND != 0 {
                state.length as u64
            } else {
                description.position
            };
            let position = usize::try_from(position).map_err(|_| Status::NoSpace)?;
            if request.length != 0 && position >= CAPACITY {
                return Err(Status::NoSpace);
            }
            let bytes = (request.length as usize).min(CAPACITY.saturating_sub(position));
            if bytes != 0 {
                if position > state.length {
                    state.bytes[state.length..position].fill(0);
                }
                state.bytes[position..position + bytes].copy_from_slice(&snapshot[..bytes]);
                state.length = state.length.max(position + bytes);
            }
            description.position = position as u64 + bytes as u64;
            state.writes += 1;
            Completion::value(bytes as u64)
        }
        Operation::Open => unreachable!(),
    };
    state.descriptions[slot] = Some(description);
    Ok(result)
}

fn open(
    endpoint: Handle,
    state: &mut State,
    request: Request,
    memory: Handle,
) -> Result<Completion, Status> {
    let options = OpenOptions::decode(request.argument)?;
    let mut path = [0; PAGE];
    let path = &mut path[..request.length as usize];
    copy_from(memory, 0, path)?;
    if path.contains(&0) {
        return Err(Status::Malformed);
    }
    let file = match &*path {
        b"/probe" => File::Probe,
        b"/readonly" => File::ReadOnly,
        _ => return Err(Status::NotFound),
    };
    let exists = file == File::ReadOnly || state.exists;
    if !exists && options.flags & file::CREATE == 0 {
        return Err(Status::NotFound);
    }
    if exists
        && options.flags & (file::CREATE | file::EXCLUSIVE) == (file::CREATE | file::EXCLUSIVE)
    {
        return Err(Status::Exists);
    }
    if file == File::ReadOnly && options.writable() {
        return Err(Status::Denied);
    }
    let slot = state
        .descriptions
        .iter()
        .position(Option::is_none)
        .ok_or(Status::TooManyOpen)?;
    let label = state.next_label;
    let next_label = label.checked_add(1).ok_or(Status::TooManyOpen)?;
    let gate = sys::endpoint_gate_create(endpoint, label).map_err(|_| Status::NoSpace)?;
    state.next_label = next_label;
    if file == File::Probe {
        state.exists = true;
        if options.flags & file::TRUNCATE != 0 {
            state.length = 0;
        }
    }
    state.descriptions[slot] = Some(Description {
        label,
        file,
        position: 0,
        options,
    });
    Ok(Completion {
        value: 0,
        gate: Some(gate),
        opened: Some(slot),
    })
}

fn map(memory: Handle, writable: bool) -> Result<(), Status> {
    let info = sys::handle_info(memory).map_err(|_| Status::Malformed)?;
    let rights = sys::READ | sys::MAP | if writable { sys::WRITE } else { 0 };
    if info.kind != ObjectKind::Memory {
        return Err(Status::Malformed);
    }
    if info.rights & rights != rights {
        return Err(Status::Denied);
    }
    // LOAN is vacant on entry and private to this one receiving thread.
    unsafe { sys::memory_map(memory, LOAN, 0, writable) }.map_err(|_| Status::Malformed)
}

fn unmap() -> Result<(), Status> {
    // No references or further raw accesses survive this operation.
    if unsafe { sys::memory_unmap(LOAN) }.is_err() {
        // Failed retirement leaves a retained mapping; never reuse its window.
        sys::exit(247);
    }
    Ok(())
}

fn copy_from(memory: Handle, offset: u64, output: &mut [u8]) -> Result<(), Status> {
    map(memory, false)?;
    for (index, byte) in output.iter_mut().enumerate() {
        // Read each client-mutable byte once into owned local storage.
        *byte = unsafe { ((LOAN + offset) as *const u8).add(index).read_volatile() };
    }
    unmap()
}

fn close_all(handles: [Option<Handle>; 2]) -> Result<(), u64> {
    let mut failed = false;
    for handle in handles.into_iter().flatten() {
        failed |= sys::close(handle).is_err();
    }
    if failed { Err(246) } else { Ok(()) }
}
