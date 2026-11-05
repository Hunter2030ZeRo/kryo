use crate::locks::{Grant as LockGrant, Locks};
use betafs_core::{Compression, CreateAttributes, Error, FileKind, FileSystem, ROOT_INODE};
use kryo_interfaces::file::{
    self, DirectoryEntry, FileType, Info, LockOptions, OpenOptions, Operation, Request, Status,
};
use kryo_libsys::{self as sys, Grant, Handle, ObjectKind, ReceiveEvent, ReplyToken};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const PAGE: usize = file::PAGE_SIZE as usize;
const LOAN: u64 = 0x0300_0000;

struct Entry {
    name: Vec<u8>,
    info: Info,
}

struct Description {
    label: u64,
    inode: u64,
    position: u64,
    options: OpenOptions,
    directory: Option<Vec<Entry>>,
    directory_position: usize,
    closed: bool,
}

pub struct Server {
    fs: FileSystem,
    descriptions: Vec<Option<Description>>,
    locks: Locks<ReplyToken>,
    next_label: u64,
    namespace_open: bool,
    poisoned: bool,
    block_poisoned: Arc<AtomicBool>,
    sync_trace: Option<Handle>,
    syncs: u64,
}

struct Completion {
    value: u64,
    attributes: u64,
    gate: Option<Handle>,
    opened: Option<u64>,
}

impl Completion {
    fn value(value: u64) -> Self {
        Self {
            value,
            attributes: 0,
            gate: None,
            opened: None,
        }
    }
    fn info(info: Info) -> Self {
        Self {
            attributes: info.attributes(),
            ..Self::value(info.size)
        }
    }
}

impl Server {
    pub fn new(
        fs: FileSystem,
        block_poisoned: Arc<AtomicBool>,
        sync_trace: Option<Handle>,
    ) -> Self {
        Self {
            fs,
            descriptions: Vec::new(),
            locks: Locks::new(),
            next_label: file::NAMESPACE_LABEL + 1,
            namespace_open: true,
            poisoned: false,
            block_poisoned,
            sync_trace,
            syncs: 0,
        }
    }

    fn ready(&self) -> Result<(), Status> {
        if self.poisoned || self.block_poisoned.load(Ordering::Acquire) {
            Err(Status::Unavailable)
        } else {
            Ok(())
        }
    }

    fn status(&mut self, error: Error) -> Status {
        if matches!(
            &error,
            Error::Tree(betafs_tree::Error::CommitUncertain(_) | betafs_tree::Error::NeedsReopen)
        ) || self.block_poisoned.load(Ordering::Acquire)
        {
            self.poisoned = true;
        }
        match error {
            Error::NotFound => Status::NotFound,
            Error::AlreadyExists => Status::Exists,
            Error::NotDirectory => Status::NotDirectory,
            Error::IsDirectory => Status::IsDirectory,
            Error::InvalidName | Error::InvalidMode => Status::Malformed,
            Error::TooLarge => Status::Overflow,
            Error::InvalidHandle => Status::BadFile,
            Error::Tree(betafs_tree::Error::NoSpace) => Status::NoSpace,
            _ => Status::Io,
        }
    }

    fn result<T>(&mut self, result: betafs_core::Result<T>) -> Result<T, Status> {
        result.map_err(|error| self.status(error))
    }

    fn info(&mut self, inode: u64) -> Result<Info, Status> {
        let result = self.fs.metadata(inode);
        let inode = self.result(result)?;
        let info = Info {
            size: inode.size,
            kind: if inode.kind == FileKind::Directory {
                FileType::Directory
            } else {
                FileType::Regular
            },
            readonly: inode.mode & 0o222 == 0,
        };
        Info::decode(info.size, info.attributes())
    }

    fn lookup(&mut self, path: &[u8]) -> Result<u64, Status> {
        validate_path(path)?;
        let mut inode = ROOT_INODE;
        for name in path[1..]
            .split(|byte| *byte == b'/')
            .filter(|name| !name.is_empty())
        {
            let result = self.fs.lookup(inode, name);
            inode = self.result(result)?;
        }
        if path.len() > 1 && path.ends_with(b"/") && self.info(inode)?.kind != FileType::Directory {
            return Err(Status::NotDirectory);
        }
        Ok(inode)
    }

    fn parent<'a>(&mut self, path: &'a [u8]) -> Result<(u64, &'a [u8]), Status> {
        validate_path(path)?;
        if path.ends_with(b"/") {
            return Err(Status::NotDirectory);
        }
        let slash = path
            .iter()
            .rposition(|byte| *byte == b'/')
            .ok_or(Status::Malformed)?;
        let parent = if slash == 0 {
            ROOT_INODE
        } else {
            self.lookup(&path[..slash])?
        };
        let info = self.info(parent)?;
        if info.kind != FileType::Directory {
            return Err(Status::NotDirectory);
        }
        if info.readonly {
            return Err(Status::Denied);
        }
        Ok((parent, &path[slash + 1..]))
    }

    fn snapshot(&mut self, inode: u64) -> Result<Vec<Entry>, Status> {
        let result = self.fs.read_dir(inode);
        let entries = self.result(result)?;
        let mut snapshot = Vec::new();
        snapshot
            .try_reserve(entries.len())
            .map_err(|_| Status::NoSpace)?;
        for entry in entries {
            snapshot.push(Entry {
                info: self.info(entry.inode)?,
                name: entry.name,
            });
        }
        Ok(snapshot)
    }

    fn slot(&self, label: u64) -> Option<usize> {
        self.descriptions.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.label == label && !entry.closed)
        })
    }

    fn reserve_description(&mut self) -> Result<usize, Status> {
        self.locks.reserve_owner()?;
        if let Some(slot) = self.descriptions.iter().position(Option::is_none) {
            return Ok(slot);
        }
        self.descriptions
            .try_reserve(1)
            .map_err(|_| Status::NoSpace)?;
        self.descriptions.push(None);
        Ok(self.descriptions.len() - 1)
    }

    fn open(
        &mut self,
        endpoint: Handle,
        request: Request,
        memory: Handle,
    ) -> Result<Completion, Status> {
        let directory_only = request.operation == Operation::OpenDir;
        let options = if directory_only {
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
        let existing = match self.lookup(path) {
            Ok(inode) => Some(inode),
            Err(Status::NotFound) if options.flags & file::CREATE != 0 => None,
            Err(status) => return Err(status),
        };
        let parent = if existing.is_none() {
            Some(self.parent(path)?)
        } else {
            None
        };
        let mut directory = None;
        if let Some(inode) = existing {
            if options.flags & (file::CREATE | file::EXCLUSIVE) == (file::CREATE | file::EXCLUSIVE)
            {
                return Err(Status::Exists);
            }
            let info = self.info(inode)?;
            if directory_only && info.kind != FileType::Directory {
                return Err(Status::NotDirectory);
            }
            if info.kind == FileType::Directory {
                if options.flags != file::READ_ONLY {
                    return Err(Status::IsDirectory);
                }
                directory = Some(self.snapshot(inode)?);
            } else if options.writable() && info.readonly {
                return Err(Status::Denied);
            }
        }
        let slot = self.reserve_description()?;
        let label = self.next_label;
        self.next_label = label.checked_add(1).ok_or(Status::TooManyOpen)?;
        let gate = sys::endpoint_gate_create(endpoint, label).map_err(|_| Status::NoSpace)?;
        let published = (|| {
            let inode = if let Some(inode) = existing {
                inode
            } else {
                let (parent, name) = parent.ok_or(Status::Malformed)?;
                let result = self.fs.create_file_with_attributes(
                    parent,
                    name,
                    &[],
                    Compression::default(),
                    CreateAttributes {
                        mode: options.mode as u16,
                        uid: 0,
                        gid: 0,
                    },
                );
                self.result(result)?
            };
            if options.flags & file::TRUNCATE != 0 {
                let result = self.fs.truncate(inode, 0);
                self.result(result)?;
            }
            let result = self.fs.pin_inode(inode);
            self.result(result)?;
            self.descriptions[slot] = Some(Description {
                label,
                inode,
                position: 0,
                options,
                directory,
                directory_position: 0,
                closed: false,
            });
            self.locks.add_owner(label, inode);
            Ok(Completion {
                gate: Some(gate),
                opened: Some(label),
                ..Completion::value(0)
            })
        })();
        if published.is_err() && sys::close(gate).is_err() {
            sys::exit(245);
        }
        published
    }

    fn execute(
        &mut self,
        endpoint: Handle,
        label: u64,
        request: Request,
        memory: Option<Handle>,
    ) -> Result<Completion, Status> {
        self.ready()?;
        if matches!(request.operation, Operation::Open | Operation::OpenDir) {
            return self.open(endpoint, request, memory.ok_or(Status::Malformed)?);
        }
        if request.operation == Operation::PathInfo {
            let mut path = [0; PAGE];
            let path = &mut path[..request.length as usize];
            copy_from(memory.ok_or(Status::Malformed)?, 0, path)?;
            let inode = self.lookup(path)?;
            return self.info(inode).map(Completion::info);
        }
        let slot = self.slot(label).ok_or(Status::BadFile)?;
        let description = self.descriptions[slot].as_ref().unwrap();
        let (inode, position, options, directory) = (
            description.inode,
            description.position,
            description.options,
            description.directory.is_some(),
        );
        match request.operation {
            Operation::Info => self.info(inode).map(Completion::info),
            Operation::Seek => {
                if directory {
                    return Err(Status::IsDirectory);
                }
                let info = self.info(inode)?;
                let position = file::seek_position(
                    position,
                    info.size,
                    request.argument as i64,
                    request.length,
                )?;
                self.descriptions[slot].as_mut().unwrap().position = position;
                Ok(Completion::value(position))
            }
            Operation::Read | Operation::ReadAt => {
                if directory {
                    return Err(Status::IsDirectory);
                }
                if !options.readable() {
                    return Err(Status::Denied);
                }
                let offset = if request.operation == Operation::ReadAt {
                    request.argument
                } else {
                    position
                };
                let result = self.fs.read_at(inode, offset, request.length as usize);
                let data = self.result(result)?;
                copy_to(
                    memory.ok_or(Status::Malformed)?,
                    if request.operation == Operation::ReadAt {
                        0
                    } else {
                        request.argument
                    },
                    &data,
                )?;
                if request.operation == Operation::Read {
                    self.descriptions[slot].as_mut().unwrap().position = offset + data.len() as u64;
                }
                Ok(Completion::value(data.len() as u64))
            }
            Operation::Write | Operation::WriteAt => {
                if directory {
                    return Err(Status::IsDirectory);
                }
                if !options.writable() || self.info(inode)?.readonly {
                    return Err(Status::Denied);
                }
                let mut data = [0; PAGE];
                let data = &mut data[..request.length as usize];
                copy_from(
                    memory.ok_or(Status::Malformed)?,
                    if request.operation == Operation::WriteAt {
                        0
                    } else {
                        request.argument
                    },
                    data,
                )?;
                let offset = if request.operation == Operation::WriteAt {
                    request.argument
                } else if options.flags & file::APPEND != 0 {
                    self.info(inode)?.size
                } else {
                    position
                };
                let result = self
                    .fs
                    .write_at(inode, offset, data, Compression::default());
                self.result(result)?;
                if request.operation == Operation::Write {
                    self.descriptions[slot].as_mut().unwrap().position = offset + data.len() as u64;
                }
                Ok(Completion::value(data.len() as u64))
            }
            Operation::SetLen => {
                if directory {
                    return Err(Status::IsDirectory);
                }
                if !options.writable() || self.info(inode)?.readonly {
                    return Err(Status::Denied);
                }
                let result = self.fs.truncate(inode, request.argument);
                self.result(result)?;
                Ok(Completion::value(0))
            }
            Operation::Sync => {
                self.sync()?;
                Ok(Completion::value(0))
            }
            Operation::NextDir => {
                let description = self.descriptions[slot].as_ref().unwrap();
                let entries = description.directory.as_ref().ok_or(Status::NotDirectory)?;
                let mut payload = [0; file::DIR_HEADER_SIZE + file::DIR_NAME_MAX];
                let length = if let Some(entry) = entries.get(description.directory_position) {
                    DirectoryEntry {
                        info: entry.info,
                        name: &entry.name,
                    }
                    .encode(&mut payload)?
                } else {
                    0
                };
                copy_to(memory.ok_or(Status::Malformed)?, 0, &payload[..length])?;
                if length != 0 {
                    self.descriptions[slot].as_mut().unwrap().directory_position += 1;
                }
                Ok(Completion::value(length as u64))
            }
            Operation::Open
            | Operation::OpenDir
            | Operation::PathInfo
            | Operation::Lock
            | Operation::Unlock => unreachable!(),
        }
    }

    fn sync(&mut self) -> Result<(), Status> {
        let next_sync = self.syncs.checked_add(1).ok_or(Status::Overflow)?;
        if let Some(debug) = self.sync_trace {
            sys::debug_log(debug, 312, next_sync).map_err(|_| Status::Io)?;
        }
        let result = self.fs.sync();
        self.result(result)?;
        self.syncs = next_sync;
        if let Some(debug) = self.sync_trace {
            sys::debug_log(debug, 311, self.syncs).map_err(|_| Status::Io)?;
        }
        Ok(())
    }

    fn retire(&mut self, endpoint: Handle, label: u64) -> Result<(), u64> {
        if label == file::NAMESPACE_LABEL {
            self.namespace_open = false;
            return Ok(());
        }
        if let Some(pending) = self.locks.remove_owner(label) {
            send_status(endpoint, pending.reply, pending.id, Status::Unavailable)?;
        }
        if let Some(slot) = self.slot(label) {
            let description = self.descriptions[slot].as_mut().unwrap();
            description.closed = true;
            let result = self.fs.unpin_inode(description.inode);
            // A failed final-unpin keeps its description and inode ownership record.
            self.result(result).map_err(|_| 263u64)?;
            self.descriptions[slot] = None;
        }
        self.progress_locks(endpoint)
    }

    fn deliver_lock(&mut self, endpoint: Handle, grant: LockGrant<ReplyToken>) -> Result<(), u64> {
        let pending = grant.pending;
        match sys::reply(
            endpoint,
            pending.reply,
            file::response(pending.id, Status::Ok, 0),
        ) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.locks.rollback(grant);
                if matches!(error, sys::Error::Kernel(-9 | -7)) {
                    return Ok(());
                }
                send_status(endpoint, pending.reply, pending.id, Status::Unavailable)?;
                Err(264)
            }
        }
    }

    fn progress_locks(&mut self, endpoint: Handle) -> Result<(), u64> {
        if self.reject_unavailable(|reply, id, status| send_status(endpoint, reply, id, status))? {
            return Ok(());
        }
        while let Some(grant) = self.locks.next_ready() {
            self.deliver_lock(endpoint, grant)?;
        }
        Ok(())
    }

    fn reject_unavailable(
        &mut self,
        mut publish: impl FnMut(ReplyToken, u64, Status) -> Result<(), u64>,
    ) -> Result<bool, u64> {
        if self.ready().is_ok() {
            return Ok(false);
        }
        while let Some(pending) = self.locks.pop_pending() {
            publish(pending.reply, pending.id, Status::Unavailable)?;
        }
        Ok(true)
    }

    fn lock(
        &mut self,
        endpoint: Handle,
        label: u64,
        request: Request,
        reply: ReplyToken,
    ) -> Result<(), u64> {
        if request.operation == Operation::Unlock {
            let status = match self.locks.unlock(label) {
                Ok(canceled) => {
                    if let Some(pending) = canceled {
                        send_status(endpoint, pending.reply, pending.id, Status::Unavailable)?;
                    }
                    Status::Ok
                }
                Err(status) => status,
            };
            send_status(endpoint, reply, request.id, status)?;
        } else if let Err(status) = self.ready() {
            send_status(endpoint, reply, request.id, status)?;
        } else {
            let options = LockOptions::decode(request.argument).map_err(|_| 264u64)?;
            match self.locks.request(label, options, request.id, reply) {
                Ok(Some(grant)) => self.deliver_lock(endpoint, grant)?,
                Ok(None) => {}
                Err(status) => send_status(endpoint, reply, request.id, status)?,
            }
        }
        self.progress_locks(endpoint)
    }

    pub fn serve(&mut self, endpoint: Handle) -> Result<(), u64> {
        let result = self.serve_inner(endpoint);
        while let Some(pending) = self.locks.pop_pending() {
            let _ = send_status(endpoint, pending.reply, pending.id, Status::Unavailable);
        }
        result
    }

    fn serve_inner(&mut self, endpoint: Handle) -> Result<(), u64> {
        loop {
            let incoming = sys::receive(endpoint, 2).map_err(|_| 260u64)?;
            if incoming.event == ReceiveEvent::GateClosed {
                if incoming.message != [0; 4]
                    || incoming.reply.is_some()
                    || incoming.handles != [None; 2]
                {
                    return Err(261);
                }
                self.retire(endpoint, incoming.label)?;
                if !self.namespace_open && self.descriptions.iter().all(Option::is_none) {
                    self.ready().map_err(|_| 262u64)?;
                    self.sync().map_err(|_| 262u64)?;
                    sys::close(endpoint).map_err(|_| 265u64)?;
                    return Ok(());
                }
                continue;
            }
            let handles = incoming.handles;
            let Some(reply) = incoming.reply else {
                close_all(handles)?;
                continue;
            };
            let request = Request::decode(
                incoming.message,
                incoming.label == file::NAMESPACE_LABEL,
                handles.iter().flatten().count(),
            );
            if let Ok(request) = request
                && matches!(request.operation, Operation::Lock | Operation::Unlock)
            {
                close_all(handles)?;
                self.lock(endpoint, incoming.label, request, reply)?;
                continue;
            }
            let result = request
                .and_then(|request| self.execute(endpoint, incoming.label, request, handles[0]));
            close_all(handles)?;
            let (status, completion) = match result {
                Ok(completion) => (Status::Ok, completion),
                Err(status) => (status, Completion::value(0)),
            };
            let response = [
                incoming.message[1],
                status as u64,
                completion.value,
                completion.attributes,
            ];
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
                if let Some(label) = completion.opened {
                    self.retire(endpoint, label)?;
                }
                if !matches!(error, sys::Error::Kernel(-9 | -7)) {
                    send_status(endpoint, reply, incoming.message[1], Status::NoSpace)?;
                }
            }
            if let Some(gate) = completion.gate {
                sys::close(gate).map_err(|_| 265u64)?;
            }
            self.reject_unavailable(|reply, id, status| send_status(endpoint, reply, id, status))?;
        }
    }
}

pub fn validate_path(path: &[u8]) -> Result<(), Status> {
    if path.is_empty() || path.len() > PAGE {
        return Err(Status::Range);
    }
    if path[0] != b'/' || path.contains(&0) {
        return Err(Status::Malformed);
    }
    for name in path[1..]
        .split(|byte| *byte == b'/')
        .filter(|name| !name.is_empty())
    {
        if name.len() > file::DIR_NAME_MAX {
            return Err(Status::Range);
        }
        if matches!(name, b"." | b"..") {
            return Err(Status::Malformed);
        }
    }
    Ok(())
}

fn send_status(endpoint: Handle, reply: ReplyToken, id: u64, status: Status) -> Result<(), u64> {
    match sys::reply(endpoint, reply, file::response(id, status, 0)) {
        Ok(()) | Err(sys::Error::Kernel(-9 | -7)) => Ok(()),
        Err(_) => Err(264),
    }
}

fn map(memory: Handle, writable: bool) -> Result<(), Status> {
    let info = sys::handle_info(memory).map_err(|_| Status::Malformed)?;
    if info.kind != ObjectKind::Memory {
        return Err(Status::Malformed);
    }
    let rights = sys::READ | sys::MAP | if writable { sys::WRITE } else { 0 };
    if info.rights & rights != rights {
        return Err(Status::Denied);
    }
    unsafe { sys::memory_map(memory, LOAN, 0, writable) }.map_err(|_| Status::Malformed)
}

fn unmap() {
    if unsafe { sys::memory_unmap(LOAN) }.is_err() {
        sys::exit(247);
    }
}

fn copy_from(memory: Handle, offset: u64, output: &mut [u8]) -> Result<(), Status> {
    map(memory, false)?;
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = unsafe { ((LOAN + offset) as *const u8).add(index).read_volatile() };
    }
    unmap();
    Ok(())
}

fn copy_to(memory: Handle, offset: u64, input: &[u8]) -> Result<(), Status> {
    map(memory, true)?;
    for (index, byte) in input.iter().enumerate() {
        unsafe {
            ((LOAN + offset) as *mut u8)
                .add(index)
                .write_volatile(*byte)
        };
    }
    unmap();
    Ok(())
}

fn close_all(handles: [Option<Handle>; 2]) -> Result<(), u64> {
    let mut failed = false;
    for handle in handles.into_iter().flatten() {
        failed |= sys::close(handle).is_err();
    }
    if failed { Err(266) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Exchange, Fault, Volume, tests::Model};
    use betafs_block::Block;
    use kryo_interfaces::block::Operation as BlockOperation;
    use std::sync::atomic::AtomicUsize;

    fn server() -> Server {
        let (volume, health) = Volume::connect(Model::new()).unwrap();
        Server::new(FileSystem::format(volume).unwrap(), health, None)
    }

    struct FlushObserver {
        model: Model,
        calls: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }

    impl Exchange for FlushObserver {
        fn exchange(
            &mut self,
            operation: BlockOperation,
            physical: u64,
            id: u64,
            data: Option<&mut Block>,
        ) -> Result<[u64; 4], Fault> {
            if operation == BlockOperation::Flush {
                self.calls.fetch_add(1, Ordering::Relaxed);
                if self.fail.load(Ordering::Relaxed) {
                    return Err(Fault { accepted: true });
                }
            }
            self.model.exchange(operation, physical, id, data)
        }
    }

    #[test]
    fn untraced_server_sync_executes_real_barrier_and_propagates_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let transport = FlushObserver {
            model: Model::new(),
            calls: calls.clone(),
            fail: fail.clone(),
        };
        let (volume, health) = Volume::connect(transport).unwrap();
        let fs = FileSystem::format(volume).unwrap();
        let mut server = Server::new(fs, health, None);
        let before = calls.load(Ordering::Relaxed);
        assert_eq!(server.sync(), Ok(()));
        assert!(calls.load(Ordering::Relaxed) > before);
        assert_eq!(server.syncs, 1);
        fail.store(true, Ordering::Relaxed);
        assert_eq!(server.sync(), Err(Status::Io));
        assert_eq!(server.syncs, 1);
        assert_eq!(server.ready(), Err(Status::Unavailable));
    }

    #[test]
    fn byte_component_lookup_is_rooted_and_does_not_require_utf8() {
        let mut server = server();
        let parent = server.fs.mkdir_at(ROOT_INODE, b"sub").unwrap();
        let inode = server
            .fs
            .create_file_at(parent, b"name\xff", b"value", Compression::default())
            .unwrap();
        assert_eq!(server.lookup(b"/sub/name\xff"), Ok(inode));
        assert_eq!(server.lookup(b"//sub//name\xff"), Ok(inode));
        assert_eq!(server.lookup(b"/sub/name\xff/"), Err(Status::NotDirectory));
        for path in [
            b"/sub/../sub".as_slice(),
            b"/./sub",
            b"relative",
            b"/sub\0suffix",
        ] {
            assert_eq!(server.lookup(path), Err(Status::Malformed));
        }
        assert_eq!(server.lookup(b"/missing"), Err(Status::NotFound));
        assert_eq!(server.lookup(b"/"), Ok(ROOT_INODE));
        assert_eq!(
            server.parent(b"/sub/new\xff"),
            Ok((parent, b"new\xff".as_slice()))
        );
    }

    #[test]
    fn actual_directory_snapshots_cache_inode_metadata_and_readonly_policy() {
        let mut server = server();
        let inode = server
            .fs
            .create_file_with_attributes(
                ROOT_INODE,
                b"readonly",
                b"immutable",
                Compression::default(),
                CreateAttributes {
                    mode: 0o444,
                    uid: 0,
                    gid: 0,
                },
            )
            .unwrap();
        let mutable = server
            .fs
            .create_file_at(ROOT_INODE, b"probe", b"before", Compression::default())
            .unwrap();
        let snapshot = server.snapshot(ROOT_INODE).unwrap();
        server.fs.truncate(mutable, 1).unwrap();
        let cached = snapshot
            .iter()
            .find(|entry| entry.name == b"probe")
            .unwrap();
        assert_eq!(cached.info.size, 6);
        assert_eq!(server.info(mutable).unwrap().size, 1);
        assert_eq!(
            server.info(inode).unwrap(),
            Info {
                size: 9,
                kind: FileType::Regular,
                readonly: true
            }
        );
        assert_eq!(server.info(ROOT_INODE).unwrap().kind, FileType::Directory);
        assert!(!server.info(ROOT_INODE).unwrap().readonly);
    }

    #[test]
    fn uncertain_engine_commit_or_block_session_poisons_future_work() {
        let mut server = server();
        assert_eq!(
            server.status(Error::Tree(betafs_tree::Error::NoSpace)),
            Status::NoSpace
        );
        assert_eq!(server.ready(), Ok(()));
        assert_eq!(
            server.status(Error::Tree(betafs_tree::Error::CommitUncertain(Box::new(
                betafs_tree::Error::NoSpace
            )))),
            Status::Io
        );
        assert_eq!(server.ready(), Err(Status::Unavailable));
        let mut server = super::tests::server();
        server.block_poisoned.store(true, Ordering::Release);
        assert_eq!(server.ready(), Err(Status::Unavailable));
        server.status(Error::NotFound);
        assert!(server.poisoned);
    }

    #[test]
    fn accepted_storage_failure_rejects_pending_locks_without_another_message() {
        let model = Model::new();
        let (volume, health) = Volume::connect(model.clone()).unwrap();
        let mut server = Server::new(FileSystem::format(volume).unwrap(), health, None);
        let inode = server
            .fs
            .create_file_at(ROOT_INODE, b"probe", b"before", Compression::default())
            .unwrap();
        for label in [2, 3] {
            server.locks.reserve_owner().unwrap();
            server.locks.add_owner(label, inode);
        }
        // The callback below observes this inert token; the host test never
        // submits it to a native syscall. ReplyToken contains a single u64.
        let reply: ReplyToken = unsafe { std::mem::zeroed() };
        let exclusive = LockOptions::decode(file::LockMode::Exclusive as u64).unwrap();
        assert!(
            server
                .locks
                .request(2, exclusive, 20, reply)
                .unwrap()
                .is_some()
        );
        assert!(
            server
                .locks
                .request(3, exclusive, 30, reply)
                .unwrap()
                .is_none()
        );
        let mut replies = Vec::new();
        assert!(
            !server
                .reject_unavailable(|_, id, status| {
                    replies.push((id, status));
                    Ok(())
                })
                .unwrap()
        );
        assert!(replies.is_empty());
        model.fail_next_write();
        let result = server
            .fs
            .write_at(inode, 0, b"after", Compression::default());
        assert_eq!(server.result(result), Err(Status::Io));
        assert!(
            server
                .reject_unavailable(|_, id, status| {
                    replies.push((id, status));
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(replies, [(30, Status::Unavailable)]);
        assert!(server.locks.pop_pending().is_none());
        let try_exclusive =
            LockOptions::decode(file::LockMode::Exclusive as u64 | file::LOCK_TRY).unwrap();
        assert!(matches!(
            server.locks.request(3, try_exclusive, 31, reply),
            Err(Status::WouldBlock)
        ));
        server.locks.unlock(2).unwrap();
        assert!(server.locks.next_ready().is_none());
    }

    #[test]
    fn failed_last_unpin_retains_closed_description_and_poisoned_owner_record() {
        let model = Model::new();
        let (volume, health) = Volume::connect(model.clone()).unwrap();
        let mut server = Server::new(FileSystem::format(volume).unwrap(), health, None);
        let inode = server
            .fs
            .create_file_at(ROOT_INODE, b"orphan", b"held", Compression::default())
            .unwrap();
        let slot = server.reserve_description().unwrap();
        server.fs.pin_inode(inode).unwrap();
        server.descriptions[slot] = Some(Description {
            label: 2,
            inode,
            position: 0,
            options: OpenOptions {
                flags: file::READ_ONLY,
                mode: 0,
            },
            directory: None,
            directory_position: 0,
            closed: false,
        });
        server.locks.add_owner(2, inode);
        server.fs.unlink_at(ROOT_INODE, b"orphan").unwrap();
        model.fail_next_write();
        assert_eq!(server.retire(Handle::from_raw(1).unwrap(), 2), Err(263));
        let retained = server.descriptions[slot].as_ref().unwrap();
        assert!(retained.closed);
        assert_eq!(retained.inode, inode);
        assert_eq!(server.slot(2), None);
        assert_eq!(server.ready(), Err(Status::Unavailable));
    }

    #[test]
    fn description_storage_grows_and_successful_final_close_balances_actual_pins() {
        let mut server = server();
        let inode = server
            .fs
            .create_file_at(ROOT_INODE, b"probe", &[], Compression::default())
            .unwrap();
        for label in 2..130 {
            let slot = server.reserve_description().unwrap();
            server.fs.pin_inode(inode).unwrap();
            server.descriptions[slot] = Some(Description {
                label,
                inode,
                position: 0,
                options: OpenOptions {
                    flags: file::READ_ONLY,
                    mode: 0,
                },
                directory: None,
                directory_position: 0,
                closed: false,
            });
            server.locks.add_owner(label, inode);
        }
        assert_eq!(server.descriptions.len(), 128);
        for label in 2..130 {
            server.retire(Handle::from_raw(1).unwrap(), label).unwrap();
        }
        assert!(server.descriptions.iter().all(Option::is_none));
        assert!(matches!(
            server.fs.unpin_inode(inode),
            Err(Error::InvalidHandle)
        ));
    }
}
