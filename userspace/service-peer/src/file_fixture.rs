//! A bounded, volatile file backend for the optional libc validation fixture.
//! It implements neither BetAFS nor a persistent namespace.

#[cfg(not(test))]
use core::sync::atomic::{AtomicBool, Ordering};
use kryo_interfaces::file::{self, FileType, Info, OpenOptions, Status, SyncScope};
#[cfg(not(test))]
use kryo_interfaces::file::{Operation, Request};
#[cfg(not(test))]
use kryo_libsys::{self as sys, Grant, Handle, ObjectKind, ReceiveEvent};

#[cfg(not(test))]
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

    fn info(&self, file: File) -> Info {
        Info {
            size: self.size(file) as u64,
            kind: FileType::Regular,
            readonly: file == File::ReadOnly,
        }
    }

    fn read_at(&self, file: File, position: u64, bytes: usize) -> &[u8] {
        let data = self.data(file);
        let position = usize::try_from(position).unwrap_or(usize::MAX);
        if position >= data.len() {
            &[]
        } else {
            &data[position..position + bytes.min(data.len() - position)]
        }
    }

    fn write_at(&mut self, file: File, position: u64, input: &[u8]) -> Result<usize, Status> {
        if file == File::ReadOnly {
            return Err(Status::Denied);
        }
        if input.is_empty() {
            return Ok(0);
        }
        let position = usize::try_from(position).map_err(|_| Status::NoSpace)?;
        if position >= CAPACITY {
            return Err(Status::NoSpace);
        }
        let bytes = input.len().min(CAPACITY - position);
        if position > self.length {
            self.bytes[self.length..position].fill(0);
        }
        self.bytes[position..position + bytes].copy_from_slice(&input[..bytes]);
        self.length = self.length.max(position + bytes);
        Ok(bytes)
    }

    fn set_len(&mut self, file: File, length: u64) -> Result<(), Status> {
        if file == File::ReadOnly {
            return Err(Status::Denied);
        }
        let length = usize::try_from(length).map_err(|_| Status::NoSpace)?;
        if length > CAPACITY {
            return Err(Status::NoSpace);
        }
        if length > self.length {
            self.bytes[self.length..length].fill(0);
        }
        self.length = length;
        Ok(())
    }

    fn sync(&self, _file: File, _scope: SyncScope) -> Result<(), Status> {
        // This RAM backend has no storage barrier to complete.
        Err(Status::Unsupported)
    }
}

#[cfg(not(test))]
struct Completion {
    value: u64,
    attributes: u64,
    gate: Option<Handle>,
    opened: Option<usize>,
}

#[cfg(not(test))]
impl Completion {
    const fn value(value: u64) -> Self {
        Self {
            value,
            attributes: 0,
            gate: None,
            opened: None,
        }
    }

    fn info(info: Info) -> Self {
        Self {
            value: info.size,
            attributes: info.attributes(),
            gate: None,
            opened: None,
        }
    }
}

#[cfg(not(test))]
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
        let mut response = file::response(incoming.message[1], status, completion.value);
        response[3] = completion.attributes;
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

#[cfg(not(test))]
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
        Operation::Read | Operation::ReadAt => {
            if !description.options.readable() {
                return Err(Status::Denied);
            }
            let memory = memory.ok_or(Status::Malformed)?;
            map(memory, true)?;
            let (position, loan_offset) = if request.operation == Operation::ReadAt {
                (request.argument, 0)
            } else {
                (description.position, request.argument)
            };
            let data = state.read_at(description.file, position, request.length as usize);
            let bytes = data.len();
            for (index, byte) in data.iter().enumerate() {
                // The private mapping and received grant retain the loan. The client
                // may access it concurrently, so do not create a shared-memory slice.
                unsafe {
                    ((LOAN + loan_offset) as *mut u8)
                        .add(index)
                        .write_volatile(*byte);
                }
            }
            unmap()?;
            if request.operation == Operation::Read {
                description.position += bytes as u64;
            }
            state.reads += 1;
            Completion::value(bytes as u64)
        }
        Operation::Write | Operation::WriteAt => {
            if !description.options.writable() || description.file == File::ReadOnly {
                return Err(Status::Denied);
            }
            let mut snapshot = [0; PAGE];
            copy_from(
                memory.ok_or(Status::Malformed)?,
                if request.operation == Operation::WriteAt {
                    0
                } else {
                    request.argument
                },
                &mut snapshot[..request.length as usize],
            )?;
            if request.length == 0 {
                return Ok(Completion::value(0));
            }
            let position = if request.operation == Operation::WriteAt {
                request.argument
            } else if description.options.flags & file::APPEND != 0 {
                state.length as u64
            } else {
                description.position
            };
            let bytes = state.write_at(
                description.file,
                position,
                &snapshot[..request.length as usize],
            )?;
            if request.operation == Operation::Write {
                description.position = position + bytes as u64;
            }
            state.writes += 1;
            Completion::value(bytes as u64)
        }
        Operation::Info => Completion::info(state.info(description.file)),
        Operation::SetLen => {
            if !description.options.writable() {
                return Err(Status::Denied);
            }
            state.set_len(description.file, request.argument)?;
            Completion::value(0)
        }
        Operation::Sync => {
            state.sync(description.file, SyncScope::try_from(request.argument)?)?;
            Completion::value(0)
        }
        Operation::Open => unreachable!(),
    };
    state.descriptions[slot] = Some(description);
    Ok(result)
}

#[cfg(not(test))]
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
        attributes: 0,
        gate: Some(gate),
        opened: Some(slot),
    })
}

#[cfg(not(test))]
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

#[cfg(not(test))]
fn unmap() -> Result<(), Status> {
    // No references or further raw accesses survive this operation.
    if unsafe { sys::memory_unmap(LOAN) }.is_err() {
        // Failed retirement leaves a retained mapping; never reuse its window.
        sys::exit(247);
    }
    Ok(())
}

#[cfg(not(test))]
fn copy_from(memory: Handle, offset: u64, output: &mut [u8]) -> Result<(), Status> {
    map(memory, false)?;
    for (index, byte) in output.iter_mut().enumerate() {
        // Read each client-mutable byte once into owned local storage.
        *byte = unsafe { ((LOAN + offset) as *const u8).add(index).read_volatile() };
    }
    unmap()
}

#[cfg(not(test))]
fn close_all(handles: [Option<Handle>; 2]) -> Result<(), u64> {
    let mut failed = false;
    for handle in handles.into_iter().flatten() {
        failed |= sys::close(handle).is_err();
    }
    if failed { Err(246) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut state = State::new();
        state.exists = true;
        state
    }

    #[test]
    fn metadata_reports_backend_size_and_readonly_property() {
        let mut state = state();
        state.write_at(File::Probe, 0, b"mutable").unwrap();
        assert_eq!(
            state.info(File::Probe),
            Info {
                size: 7,
                kind: FileType::Regular,
                readonly: false
            }
        );
        assert_eq!(
            state.info(File::ReadOnly),
            Info {
                size: READ_ONLY_BYTES.len() as u64,
                kind: FileType::Regular,
                readonly: true,
            }
        );
    }

    #[test]
    fn positioned_storage_writes_honor_offsets_and_zero_gaps() {
        let mut state = state();
        assert_eq!(state.write_at(File::Probe, 5, b"AB"), Ok(2));
        assert_eq!(state.data(File::Probe), b"\0\0\0\0\0AB");
        assert_eq!(state.write_at(File::Probe, 0, b"Z"), Ok(1));
        assert_eq!(state.data(File::Probe), b"Z\0\0\0\0AB");
        assert_eq!(state.read_at(File::Probe, 5, 1), b"A");
    }

    #[test]
    fn positioned_reads_clip_eof_without_offset_overflow() {
        let mut state = state();
        state.write_at(File::Probe, 0, b"abc").unwrap();
        assert_eq!(state.read_at(File::Probe, 1, usize::MAX), b"bc");
        assert_eq!(state.read_at(File::Probe, 3, PAGE), b"");
        assert_eq!(state.read_at(File::Probe, i64::MAX as u64, PAGE), b"");
        assert_eq!(state.read_at(File::ReadOnly, u64::MAX, PAGE), b"");
    }

    #[test]
    fn partial_capacity_writes_report_progress_and_failed_writes_preserve_state() {
        let mut state = state();
        assert_eq!(
            state.write_at(File::Probe, (CAPACITY - 2) as u64, b"ABC"),
            Ok(2)
        );
        assert_eq!(&state.data(File::Probe)[CAPACITY - 2..], b"AB");
        assert!(
            state.data(File::Probe)[..CAPACITY - 2]
                .iter()
                .all(|byte| *byte == 0)
        );
        let before = state.bytes;
        assert_eq!(
            state.write_at(File::Probe, CAPACITY as u64, b"X"),
            Err(Status::NoSpace)
        );
        assert_eq!(
            state.write_at(File::Probe, u64::MAX, b"X"),
            Err(Status::NoSpace)
        );
        assert_eq!(state.write_at(File::Probe, u64::MAX, b""), Ok(0));
        assert_eq!(state.bytes, before);
        assert_eq!(state.length, CAPACITY);
    }

    #[test]
    fn resize_zeroes_sparse_growth_and_discarded_bytes_on_regrowth() {
        let mut state = state();
        state.write_at(File::Probe, 0, b"sensitive").unwrap();
        state.set_len(File::Probe, 3).unwrap();
        state.set_len(File::Probe, 12).unwrap();
        assert_eq!(state.data(File::Probe), b"sen\0\0\0\0\0\0\0\0\0");
        state.set_len(File::Probe, 0).unwrap();
        state.set_len(File::Probe, 3).unwrap();
        assert_eq!(state.data(File::Probe), b"\0\0\0");
    }

    #[test]
    fn invalid_resize_and_readonly_mutation_have_no_effect() {
        let mut state = state();
        state.write_at(File::Probe, 0, b"keep").unwrap();
        let before = state.bytes;
        assert_eq!(
            state.set_len(File::Probe, CAPACITY as u64 + 1),
            Err(Status::NoSpace)
        );
        assert_eq!(state.set_len(File::Probe, u64::MAX), Err(Status::NoSpace));
        assert_eq!(state.set_len(File::ReadOnly, 0), Err(Status::Denied));
        assert_eq!(state.write_at(File::ReadOnly, 0, b"X"), Err(Status::Denied));
        assert_eq!(state.write_at(File::ReadOnly, 0, b""), Err(Status::Denied));
        assert_eq!(state.bytes, before);
        assert_eq!(state.length, 4);
        assert_eq!(state.data(File::ReadOnly), READ_ONLY_BYTES);
    }

    #[test]
    fn volatile_backend_never_reports_a_completed_storage_barrier() {
        let mut state = state();
        state.write_at(File::Probe, 0, b"live").unwrap();
        for scope in [SyncScope::All, SyncScope::Data] {
            assert_eq!(state.sync(File::Probe, scope), Err(Status::Unsupported));
            assert_eq!(state.sync(File::ReadOnly, scope), Err(Status::Unsupported));
        }
        assert_eq!(state.data(File::Probe), b"live");
    }
}
