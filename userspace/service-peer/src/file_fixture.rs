//! A bounded, volatile file backend for the optional libc validation fixture.
//! It implements neither BetAFS nor a persistent namespace.

#[cfg(not(test))]
use core::sync::atomic::{AtomicBool, Ordering};
use kryo_interfaces::file::{
    self, DirectoryEntry, FileType, Info, LockMode, LockOptions, LookupPolicy, OpenOptions, Status,
    SyncScope,
};
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
const LINK_LIMIT: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum File {
    Probe,
    ReadOnly,
    Root,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    File(File),
    Directory,
    Link(&'static [u8]),
}

fn lookup(path: &[u8]) -> Result<Node, Status> {
    match path {
        b"/" => Ok(Node::Directory),
        b"/probe" => Ok(Node::File(File::Probe)),
        b"/readonly" => Ok(Node::File(File::ReadOnly)),
        b"/probe-link" => Ok(Node::Link(b"/probe")),
        b"/dangling" => Ok(Node::Link(b"/missing")),
        b"/cycle" => Ok(Node::Link(b"/cycle")),
        b"/denied" => Err(Status::Denied),
        _ => Err(Status::NotFound),
    }
}

fn resolve(mut path: &[u8], policy: LookupPolicy) -> Result<Node, Status> {
    if path.is_empty() || path.len() > PAGE {
        return Err(Status::Range);
    }
    if path.contains(&0) {
        return Err(Status::Malformed);
    }
    let mut links = 0;
    loop {
        let node = lookup(path)?;
        let Node::Link(target) = node else {
            return Ok(node);
        };
        if policy == LookupPolicy::NoFollow {
            return Ok(node);
        }
        if links == LINK_LIMIT {
            return Err(Status::SymlinkLoop);
        }
        links += 1;
        path = target;
    }
}

#[derive(Clone, Copy)]
struct Description {
    label: u64,
    file: File,
    position: u64,
    options: OpenOptions,
    held: Option<LockMode>,
}

#[derive(Clone, Copy)]
struct DirectorySnapshot {
    entries: [Option<DirectoryEntry<'static>>; 5],
    position: usize,
}

#[cfg(not(test))]
type ReplyToken = sys::ReplyToken;
#[cfg(test)]
type ReplyToken = u64;

#[derive(Clone, Copy)]
struct PendingLock {
    label: u64,
    file: File,
    mode: LockMode,
    id: u64,
    reply: ReplyToken,
}

#[derive(Clone, Copy)]
struct LockGrant {
    pending: PendingLock,
    previous: Option<LockMode>,
}

struct State {
    bytes: [u8; CAPACITY],
    length: usize,
    exists: bool,
    descriptions: [Option<Description>; OPEN_LIMIT],
    directories: [Option<DirectorySnapshot>; OPEN_LIMIT],
    pending: [Option<PendingLock>; OPEN_LIMIT],
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
            directories: [None; OPEN_LIMIT],
            pending: [None; OPEN_LIMIT],
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

    fn retire(&mut self, label: u64) -> Option<PendingLock> {
        if label == file::NAMESPACE_LABEL {
            self.namespace_open = false;
        } else if let Some(slot) = self.description(label) {
            self.descriptions[slot] = None;
            self.directories[slot] = None;
            self.closures += 1;
        }
        self.cancel_pending(label)
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
            File::Root => 0,
        }
    }

    fn data(&self, file: File) -> &[u8] {
        match file {
            File::Probe => &self.bytes[..self.length],
            File::ReadOnly => READ_ONLY_BYTES,
            File::Root => &[],
        }
    }

    fn info(&self, file: File) -> Info {
        Info {
            size: self.size(file) as u64,
            kind: if file == File::Root {
                FileType::Directory
            } else {
                FileType::Regular
            },
            readonly: file == File::ReadOnly,
        }
    }

    fn path_info(&self, path: &[u8], policy: LookupPolicy) -> Result<Info, Status> {
        match resolve(path, policy)? {
            Node::File(File::Probe) if !self.exists => Err(Status::NotFound),
            Node::File(file) => Ok(self.info(file)),
            Node::Directory => Ok(Info {
                size: 0,
                kind: FileType::Directory,
                readonly: false,
            }),
            Node::Link(target) => Ok(Info {
                size: target.len() as u64,
                kind: FileType::Symlink,
                readonly: true,
            }),
        }
    }

    fn open_file(&self, path: &[u8], options: OpenOptions) -> Result<File, Status> {
        if options.flags & (file::CREATE | file::EXCLUSIVE) == (file::CREATE | file::EXCLUSIVE)
            && matches!(lookup(path), Ok(Node::Link(_)))
        {
            return Err(Status::Exists);
        }
        let file = match resolve(path, LookupPolicy::Follow)? {
            Node::File(file) => file,
            Node::Directory => {
                if options.flags != file::READ_ONLY {
                    return Err(Status::IsDirectory);
                }
                return Ok(File::Root);
            }
            Node::Link(_) => unreachable!(),
        };
        let exists = file == File::ReadOnly || self.exists;
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
        Ok(file)
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
        if file == File::Root {
            return Err(Status::IsDirectory);
        }
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
        if file == File::Root {
            return Err(Status::IsDirectory);
        }
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

    fn directory_snapshot(&self) -> DirectorySnapshot {
        let mut entries = [None; 5];
        let mut count = 0;
        for (name, path) in [
            (b"probe".as_slice(), b"/probe".as_slice()),
            (b"readonly", b"/readonly"),
            (b"probe-link", b"/probe-link"),
            (b"dangling", b"/dangling"),
            (b"cycle", b"/cycle"),
        ] {
            if let Ok(info) = self.path_info(path, LookupPolicy::NoFollow) {
                entries[count] = Some(DirectoryEntry { info, name });
                count += 1;
            }
        }
        DirectorySnapshot {
            entries,
            position: 0,
        }
    }

    fn directory_entry(&self, slot: usize) -> Result<Option<DirectoryEntry<'static>>, Status> {
        let snapshot = self.directories[slot]
            .as_ref()
            .ok_or(Status::NotDirectory)?;
        Ok(snapshot.entries.get(snapshot.position).copied().flatten())
    }

    fn advance_directory(&mut self, slot: usize) {
        if let Some(snapshot) = self.directories[slot].as_mut()
            && snapshot.position < snapshot.entries.len()
            && snapshot.entries[snapshot.position].is_some()
        {
            snapshot.position += 1;
        }
    }

    fn conflicts(&self, label: u64, file: File, mode: LockMode) -> bool {
        self.descriptions.iter().flatten().any(|other| {
            other.label != label
                && other.file == file
                && other
                    .held
                    .is_some_and(|held| held == LockMode::Exclusive || mode == LockMode::Exclusive)
        })
    }

    fn remove_pending(&mut self, index: usize) -> PendingLock {
        let pending = self.pending[index].take().unwrap();
        self.pending[index..].rotate_left(1);
        self.pending[OPEN_LIMIT - 1] = None;
        pending
    }

    fn cancel_pending(&mut self, label: u64) -> Option<PendingLock> {
        self.pending
            .iter()
            .position(|pending| pending.is_some_and(|p| p.label == label))
            .map(|index| self.remove_pending(index))
    }

    fn request_lock(
        &mut self,
        label: u64,
        options: LockOptions,
        id: u64,
        reply: ReplyToken,
    ) -> Result<Option<LockGrant>, Status> {
        let slot = self.description(label).ok_or(Status::BadFile)?;
        let description = self.descriptions[slot].unwrap();
        if self
            .pending
            .iter()
            .flatten()
            .any(|pending| pending.label == label)
        {
            return Err(if options.nonblocking {
                Status::WouldBlock
            } else {
                Status::Deadlock
            });
        }
        let pending = PendingLock {
            label,
            file: description.file,
            mode: options.mode,
            id,
            reply,
        };
        let queued = self
            .pending
            .iter()
            .flatten()
            .any(|waiter| waiter.file == description.file);
        let compatible = !self.conflicts(label, description.file, options.mode);
        let existing = description.held;
        if existing == Some(options.mode)
            || (existing == Some(LockMode::Exclusive) && options.mode == LockMode::Shared)
            || (compatible && !queued)
        {
            self.descriptions[slot].as_mut().unwrap().held = Some(options.mode);
            return Ok(Some(LockGrant {
                pending,
                previous: existing,
            }));
        }
        if options.nonblocking {
            return Err(Status::WouldBlock);
        }
        // An upgrade may not sleep while retaining a lock needed by an earlier waiter.
        if existing == Some(LockMode::Shared) {
            return Err(Status::Deadlock);
        }
        let free = self
            .pending
            .iter()
            .position(Option::is_none)
            .ok_or(Status::NoSpace)?;
        self.pending[free] = Some(pending);
        Ok(None)
    }

    fn next_lock(&mut self) -> Option<LockGrant> {
        for index in 0..OPEN_LIMIT {
            let Some(pending) = self.pending[index] else {
                break;
            };
            if self.pending[..index]
                .iter()
                .flatten()
                .any(|earlier| earlier.file == pending.file)
                || self.conflicts(pending.label, pending.file, pending.mode)
            {
                continue;
            }
            let Some(slot) = self.description(pending.label) else {
                self.remove_pending(index);
                return self.next_lock();
            };
            self.remove_pending(index);
            let previous = self.descriptions[slot]
                .as_mut()
                .unwrap()
                .held
                .replace(pending.mode);
            return Some(LockGrant { pending, previous });
        }
        None
    }

    fn rollback_lock(&mut self, grant: LockGrant) {
        if let Some(slot) = self.description(grant.pending.label) {
            self.descriptions[slot].as_mut().unwrap().held = grant.previous;
        }
    }

    fn unlock(&mut self, label: u64) -> Result<Option<PendingLock>, Status> {
        let slot = self.description(label).ok_or(Status::BadFile)?;
        self.descriptions[slot].as_mut().unwrap().held = None;
        Ok(self.cancel_pending(label))
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
    let result = serve_inner(endpoint, state);
    if result.is_err() {
        while let Some(pending) = state.pending[0] {
            state.remove_pending(0);
            let _ = send_status(endpoint, pending.reply, pending.id, Status::Unavailable);
        }
    }
    result
}

#[cfg(not(test))]
fn serve_inner(endpoint: Handle, state: &mut State) -> Result<(), u64> {
    loop {
        let incoming = sys::receive(endpoint, 2).map_err(|_| 240u64)?;
        if incoming.event == ReceiveEvent::GateClosed {
            if incoming.message != [0; 4]
                || incoming.reply.is_some()
                || incoming.handles != [None; 2]
            {
                return Err(241);
            }
            if let Some(pending) = state.retire(incoming.label) {
                send_status(endpoint, pending.reply, pending.id, Status::Unavailable)?;
            }
            progress_locks(endpoint, state)?;
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
        let request = Request::decode(
            incoming.message,
            incoming.label == file::NAMESPACE_LABEL,
            count,
        );
        if let Ok(request) = request
            && matches!(request.operation, Operation::Lock | Operation::Unlock)
        {
            close_all(handles)?;
            handle_lock(endpoint, state, incoming.label, request, reply)?;
            progress_locks(endpoint, state)?;
            continue;
        }
        let result = request
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
                state.directories[slot] = None;
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
fn send_status(endpoint: Handle, reply: ReplyToken, id: u64, status: Status) -> Result<(), u64> {
    match sys::reply(endpoint, reply, file::response(id, status, 0)) {
        Ok(()) | Err(sys::Error::Kernel(-9 | -7)) => Ok(()),
        Err(_) => Err(244),
    }
}

#[cfg(not(test))]
fn deliver_lock(endpoint: Handle, state: &mut State, grant: LockGrant) -> Result<(), u64> {
    let pending = grant.pending;
    match sys::reply(
        endpoint,
        pending.reply,
        file::response(pending.id, Status::Ok, 0),
    ) {
        Ok(()) => Ok(()),
        Err(error) => {
            state.rollback_lock(grant);
            if matches!(error, sys::Error::Kernel(-9 | -7)) {
                return Ok(());
            }
            // Unexpected preflight failures may leave a live token; reject rather than
            // retaining an advisory lock whose acquisition was never acknowledged.
            send_status(endpoint, pending.reply, pending.id, Status::Unavailable)?;
            Err(244)
        }
    }
}

#[cfg(not(test))]
fn progress_locks(endpoint: Handle, state: &mut State) -> Result<(), u64> {
    while let Some(grant) = state.next_lock() {
        deliver_lock(endpoint, state, grant)?;
    }
    Ok(())
}

#[cfg(not(test))]
fn handle_lock(
    endpoint: Handle,
    state: &mut State,
    label: u64,
    request: Request,
    reply: ReplyToken,
) -> Result<(), u64> {
    if request.operation == Operation::Unlock {
        let status = match state.unlock(label) {
            Ok(canceled) => {
                if let Some(pending) = canceled {
                    send_status(endpoint, pending.reply, pending.id, Status::Unavailable)?;
                }
                Status::Ok
            }
            Err(status) => status,
        };
        return send_status(endpoint, reply, request.id, status);
    }
    let options = LockOptions::decode(request.argument).map_err(|_| 244u64)?;
    match state.request_lock(label, options, request.id, reply) {
        Ok(Some(grant)) => deliver_lock(endpoint, state, grant),
        Ok(None) => Ok(()),
        Err(status) => send_status(endpoint, reply, request.id, status),
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
    if matches!(request.operation, Operation::Open | Operation::OpenDir) {
        return open(endpoint, state, request, memory.ok_or(Status::Malformed)?);
    }
    if request.operation == Operation::PathInfo {
        return path_info(state, request, memory.ok_or(Status::Malformed)?);
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
            if description.file == File::Root {
                return Err(Status::IsDirectory);
            }
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
            if description.file == File::Root {
                return Err(Status::IsDirectory);
            }
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
            if description.file == File::Root {
                return Err(Status::IsDirectory);
            }
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
        Operation::NextDir => {
            let entry = state.directory_entry(slot)?;
            let mut snapshot = [0; file::DIR_HEADER_SIZE + file::DIR_NAME_MAX];
            let length = match entry {
                Some(entry) => entry.encode(&mut snapshot)?,
                None => 0,
            };
            copy_to(memory.ok_or(Status::Malformed)?, &snapshot[..length])?;
            state.advance_directory(slot);
            Completion::value(length as u64)
        }
        Operation::Open
        | Operation::OpenDir
        | Operation::PathInfo
        | Operation::Lock
        | Operation::Unlock => unreachable!(),
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
    let options = if request.operation == Operation::OpenDir {
        OpenOptions {
            flags: file::READ_ONLY,
            mode: 0,
        }
    } else {
        OpenOptions::decode(request.argument)?
    };
    let mut path = [0; PAGE];
    let path = &mut path[..request.length as usize];
    copy_from(memory, 0, path)?;
    let file = state.open_file(path, options)?;
    if request.operation == Operation::OpenDir && file != File::Root {
        return Err(Status::NotDirectory);
    }
    let directory = (file == File::Root).then(|| state.directory_snapshot());
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
        held: None,
    });
    state.directories[slot] = directory;
    Ok(Completion {
        value: 0,
        attributes: 0,
        gate: Some(gate),
        opened: Some(slot),
    })
}

#[cfg(not(test))]
fn path_info(state: &State, request: Request, memory: Handle) -> Result<Completion, Status> {
    let mut path = [0; PAGE];
    let path = &mut path[..request.length as usize];
    copy_from(memory, 0, path)?;
    state
        .path_info(path, LookupPolicy::try_from(request.argument)?)
        .map(Completion::info)
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
fn copy_to(memory: Handle, input: &[u8]) -> Result<(), Status> {
    map(memory, true)?;
    for (index, byte) in input.iter().enumerate() {
        unsafe { (LOAN as *mut u8).add(index).write_volatile(*byte) };
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

    fn description(state: &mut State, slot: usize, file: File) -> u64 {
        let label = slot as u64 + 2;
        state.descriptions[slot] = Some(Description {
            label,
            file,
            position: 0,
            options: OpenOptions {
                flags: file::READ_WRITE,
                mode: 0,
            },
            held: None,
        });
        label
    }

    fn lock(
        state: &mut State,
        label: u64,
        mode: LockMode,
        nonblocking: bool,
    ) -> Result<Option<LockGrant>, Status> {
        state.request_lock(
            label,
            LockOptions { mode, nonblocking },
            label + 100,
            label + 1000,
        )
    }

    fn held(state: &State, label: u64) -> Option<LockMode> {
        state.descriptions[state.description(label).unwrap()]
            .unwrap()
            .held
    }

    #[test]
    fn directory_snapshots_cache_nofollow_entries_without_creating_missing_files() {
        let mut state = State::new();
        state.directories[0] = Some(state.directory_snapshot());
        let first = state.directory_entry(0).unwrap().unwrap();
        assert_eq!(first.name, b"readonly");
        assert!(first.info.readonly);
        assert!(!state.exists);
        state.exists = true;
        state.write_at(File::Probe, 0, b"before").unwrap();
        state.directories[1] = Some(state.directory_snapshot());
        state.set_len(File::Probe, 1).unwrap();
        let probe = state.directory_entry(1).unwrap().unwrap();
        assert_eq!(probe.name, b"probe");
        assert_eq!(probe.info.size, 6);
        assert_eq!(probe.info.kind, FileType::Regular);
        state.advance_directory(1);
        assert_eq!(state.directory_entry(0).unwrap().unwrap().name, b"readonly");
        let mut names = [b"".as_slice(); 4];
        for name in &mut names {
            let entry = state.directory_entry(0).unwrap().unwrap();
            *name = entry.name;
            if entry.name != b"readonly" {
                assert_eq!(entry.info.kind, FileType::Symlink);
                assert!(entry.info.readonly);
            }
            state.advance_directory(0);
        }
        assert_eq!(
            names,
            [b"readonly".as_slice(), b"probe-link", b"dangling", b"cycle"]
        );
        assert_eq!(state.directory_entry(0), Ok(None));
        state.advance_directory(0);
        assert_eq!(state.directory_entry(0), Ok(None));
        assert_eq!(state.directory_entry(2), Err(Status::NotDirectory));
        assert!(state.descriptions.iter().all(Option::is_none));
    }

    #[test]
    fn directory_open_and_mutation_have_distinct_type_errors() {
        let state = State::new();
        let read = OpenOptions {
            flags: file::READ_ONLY,
            mode: 0,
        };
        assert_eq!(state.open_file(b"/", read), Ok(File::Root));
        for flags in [
            file::WRITE_ONLY,
            file::READ_WRITE,
            file::APPEND,
            file::CREATE,
        ] {
            assert_eq!(
                state.open_file(b"/", OpenOptions { flags, mode: 0 }),
                Err(Status::IsDirectory)
            );
        }
        assert_eq!(state.info(File::Root).kind, FileType::Directory);
        let mut state = state;
        assert_eq!(state.write_at(File::Root, 0, b""), Err(Status::IsDirectory));
        assert_eq!(state.set_len(File::Root, 0), Err(Status::IsDirectory));
    }

    #[test]
    fn lock_queue_preserves_writer_fairness_and_independent_file_progress() {
        let mut state = state();
        let a = description(&mut state, 0, File::Probe);
        let b = description(&mut state, 1, File::Probe);
        let c = description(&mut state, 2, File::Probe);
        let d = description(&mut state, 3, File::ReadOnly);
        assert!(
            lock(&mut state, a, LockMode::Shared, false)
                .unwrap()
                .is_some()
        );
        assert!(
            lock(&mut state, b, LockMode::Exclusive, false)
                .unwrap()
                .is_none()
        );
        assert!(
            lock(&mut state, c, LockMode::Shared, false)
                .unwrap()
                .is_none()
        );
        assert!(
            lock(&mut state, d, LockMode::Exclusive, false)
                .unwrap()
                .is_some()
        );
        assert!(state.next_lock().is_none());
        state.unlock(a).unwrap();
        let writer = state.next_lock().unwrap();
        assert_eq!(writer.pending.label, b);
        assert!(state.next_lock().is_none());
        state.unlock(b).unwrap();
        assert_eq!(state.next_lock().unwrap().pending.label, c);
        assert_eq!(held(&state, c), Some(LockMode::Shared));
        assert_eq!(held(&state, d), Some(LockMode::Exclusive));
    }

    #[test]
    fn lock_upgrades_reject_deadlock_without_releasing_shared_ownership() {
        let mut state = state();
        let a = description(&mut state, 0, File::Probe);
        let b = description(&mut state, 1, File::Probe);
        lock(&mut state, a, LockMode::Shared, false).unwrap();
        lock(&mut state, b, LockMode::Shared, false).unwrap();
        assert!(matches!(
            lock(&mut state, a, LockMode::Exclusive, false),
            Err(Status::Deadlock)
        ));
        assert!(matches!(
            lock(&mut state, a, LockMode::Exclusive, true),
            Err(Status::WouldBlock)
        ));
        assert_eq!(held(&state, a), Some(LockMode::Shared));
        assert!(state.pending.iter().all(Option::is_none));
        state.unlock(b).unwrap();
        let upgrade = lock(&mut state, a, LockMode::Exclusive, false)
            .unwrap()
            .unwrap();
        assert_eq!(held(&state, a), Some(LockMode::Exclusive));
        state.rollback_lock(upgrade);
        assert_eq!(held(&state, a), Some(LockMode::Shared));
        lock(&mut state, a, LockMode::Exclusive, false).unwrap();
        let downgrade = lock(&mut state, a, LockMode::Shared, false)
            .unwrap()
            .unwrap();
        state.rollback_lock(downgrade);
        assert_eq!(held(&state, a), Some(LockMode::Exclusive));
        let repeat = lock(&mut state, a, LockMode::Exclusive, false)
            .unwrap()
            .unwrap();
        state.rollback_lock(repeat);
        assert_eq!(held(&state, a), Some(LockMode::Exclusive));
    }

    #[test]
    fn queued_shared_batch_and_canceled_grants_do_not_strand_locks() {
        let mut state = state();
        let a = description(&mut state, 0, File::Probe);
        let b = description(&mut state, 1, File::Probe);
        let c = description(&mut state, 2, File::Probe);
        let d = description(&mut state, 3, File::Probe);
        lock(&mut state, a, LockMode::Exclusive, false).unwrap();
        for (label, mode) in [
            (b, LockMode::Shared),
            (c, LockMode::Shared),
            (d, LockMode::Exclusive),
        ] {
            assert!(lock(&mut state, label, mode, false).unwrap().is_none());
        }
        assert!(matches!(
            lock(&mut state, b, LockMode::Shared, true),
            Err(Status::WouldBlock)
        ));
        assert!(matches!(
            lock(&mut state, b, LockMode::Shared, false),
            Err(Status::Deadlock)
        ));
        state.retire(a);
        let first = state.next_lock().unwrap();
        assert_eq!(first.pending.label, b);
        state.rollback_lock(first);
        assert_eq!(held(&state, b), None);
        assert_eq!(state.next_lock().unwrap().pending.label, c);
        assert!(state.next_lock().is_none());
        state.retire(c);
        let writer = state.next_lock().unwrap();
        assert_eq!(writer.pending.label, d);
        state.rollback_lock(writer);
        assert!(state.next_lock().is_none());
        assert_eq!(held(&state, d), None);
    }

    #[test]
    fn closing_or_unlocking_pending_owner_returns_its_exact_reply_token() {
        let mut state = state();
        let a = description(&mut state, 0, File::Probe);
        let b = description(&mut state, 1, File::Probe);
        let c = description(&mut state, 2, File::Probe);
        lock(&mut state, a, LockMode::Exclusive, false).unwrap();
        lock(&mut state, b, LockMode::Exclusive, false).unwrap();
        lock(&mut state, c, LockMode::Shared, false).unwrap();
        let canceled = state.unlock(b).unwrap().unwrap();
        assert_eq!(
            (canceled.label, canceled.id, canceled.reply),
            (b, b + 100, b + 1000)
        );
        let closed = state.retire(c).unwrap();
        assert_eq!(closed.reply, c + 1000);
        assert!(state.pending.iter().all(Option::is_none));
        state.retire(a);
        assert!(
            lock(&mut state, b, LockMode::Exclusive, true)
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            lock(&mut state, c, LockMode::Shared, true),
            Err(Status::BadFile)
        ));
    }

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

    #[test]
    fn pathname_metadata_observes_absence_without_creating_file_or_description() {
        let state = State::new();
        for policy in [LookupPolicy::Follow, LookupPolicy::NoFollow] {
            assert_eq!(state.path_info(b"/probe", policy), Err(Status::NotFound));
        }
        assert_eq!(
            state.path_info(b"/probe-link", LookupPolicy::Follow),
            Err(Status::NotFound)
        );
        assert_eq!(
            state.path_info(b"/probe-link", LookupPolicy::NoFollow),
            Ok(Info {
                size: 6,
                kind: FileType::Symlink,
                readonly: true,
            })
        );
        assert_eq!(
            state.path_info(b"/", LookupPolicy::Follow),
            Ok(Info {
                size: 0,
                kind: FileType::Directory,
                readonly: false,
            })
        );
        assert_eq!(
            state.path_info(b"/readonly", LookupPolicy::Follow),
            Ok(state.info(File::ReadOnly))
        );
        assert!(!state.exists);
        assert!(state.descriptions.iter().all(Option::is_none));
        assert_eq!(state.opens, 0);
        assert_eq!(state.closures, 0);
        assert_eq!(state.next_label, file::NAMESPACE_LABEL + 1);
    }

    #[test]
    fn pathname_follow_observes_backend_file_changes() {
        let mut state = state();
        state.write_at(File::Probe, 0, b"mutable").unwrap();
        for path in [b"/probe".as_slice(), b"/probe-link"] {
            assert_eq!(
                state.path_info(path, LookupPolicy::Follow),
                Ok(state.info(File::Probe))
            );
        }
        state.set_len(File::Probe, 3).unwrap();
        assert_eq!(
            state
                .path_info(b"/probe-link", LookupPolicy::Follow)
                .unwrap()
                .size,
            3
        );
        assert_eq!(
            state
                .path_info(b"/probe-link", LookupPolicy::NoFollow)
                .unwrap()
                .size,
            6
        );
    }

    #[test]
    fn dangling_links_and_cycles_distinguish_final_link_policy() {
        let state = state();
        assert_eq!(
            state.path_info(b"/dangling", LookupPolicy::Follow),
            Err(Status::NotFound)
        );
        assert_eq!(
            state.path_info(b"/dangling", LookupPolicy::NoFollow),
            Ok(Info {
                size: 8,
                kind: FileType::Symlink,
                readonly: true,
            })
        );
        assert_eq!(
            state.path_info(b"/cycle", LookupPolicy::Follow),
            Err(Status::SymlinkLoop)
        );
        assert_eq!(
            state.path_info(b"/cycle", LookupPolicy::NoFollow),
            Ok(Info {
                size: 6,
                kind: FileType::Symlink,
                readonly: true,
            })
        );
    }

    #[test]
    fn pathname_resolution_propagates_denial_and_rejects_unrecognized_names() {
        let state = state();
        for policy in [LookupPolicy::Follow, LookupPolicy::NoFollow] {
            assert_eq!(state.path_info(b"/denied", policy), Err(Status::Denied));
            for path in [
                b"/unknown".as_slice(),
                b"../probe",
                b"/probe/..",
                b"/unknown\xff",
            ] {
                assert_eq!(state.path_info(path, policy), Err(Status::NotFound));
            }
            assert_eq!(
                state.path_info(b"/probe\0suffix", policy),
                Err(Status::Malformed)
            );
            assert_eq!(state.path_info(b"", policy), Err(Status::Range));
            assert_eq!(
                state.path_info(&[b'x'; PAGE + 1], policy),
                Err(Status::Range)
            );
        }
    }

    #[test]
    fn open_follows_aliases_but_exclusive_creation_does_not_follow_links() {
        let mut state = State::new();
        let read = OpenOptions {
            flags: file::READ_ONLY,
            mode: 0,
        };
        let create = OpenOptions {
            flags: file::READ_WRITE | file::CREATE,
            mode: 0o666,
        };
        let exclusive = OpenOptions {
            flags: create.flags | file::EXCLUSIVE,
            mode: 0o666,
        };
        assert_eq!(state.open_file(b"/probe-link", read), Err(Status::NotFound));
        assert_eq!(state.open_file(b"/probe-link", create), Ok(File::Probe));
        for path in [b"/probe-link".as_slice(), b"/dangling", b"/cycle"] {
            assert_eq!(state.open_file(path, exclusive), Err(Status::Exists));
        }
        assert_eq!(state.open_file(b"/probe", exclusive), Ok(File::Probe));
        assert!(!state.exists);
        assert_eq!(state.open_file(b"/dangling", create), Err(Status::NotFound));
        assert_eq!(state.open_file(b"/cycle", read), Err(Status::SymlinkLoop));
        assert_eq!(state.open_file(b"/", read), Ok(File::Root));
        assert_eq!(state.open_file(b"/denied", read), Err(Status::Denied));
        state.exists = true;
        assert_eq!(state.open_file(b"/probe-link", read), Ok(File::Probe));
        assert_eq!(state.open_file(b"/readonly", read), Ok(File::ReadOnly));
        assert_eq!(state.open_file(b"/readonly", create), Err(Status::Denied));
        assert!(state.descriptions.iter().all(Option::is_none));
        assert_eq!(state.opens, 0);
    }
}
