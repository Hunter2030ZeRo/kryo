use betafs_block::{Block, BlockDevice};
use kryo_interfaces::block::{self, Operation, Status};
use kryo_libsys::{self as sys, Grant, Handle};
use std::cell::RefCell;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Fault {
    pub accepted: bool,
}

pub trait Exchange: Send {
    fn exchange(
        &mut self,
        operation: Operation,
        physical: u64,
        id: u64,
        data: Option<&mut Block>,
    ) -> Result<[u64; 4], Fault>;
}

pub struct Volume<T: Exchange> {
    session: RefCell<Session<T>>,
    blocks: u64,
}

struct Session<T: Exchange> {
    transport: T,
    next_id: u64,
    poisoned: Arc<AtomicBool>,
}

impl<T: Exchange> Session<T> {
    fn request(
        &mut self,
        operation: Operation,
        physical: u64,
        data: Option<&mut Block>,
    ) -> io::Result<u64> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(io::Error::other("block session requires reopen"));
        }
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("request ID exhausted"))?;
        let response = match self.transport.exchange(operation, physical, id, data) {
            Ok(response) => response,
            Err(fault) => {
                if fault.accepted {
                    self.poisoned.store(true, Ordering::Release);
                }
                return Err(io::Error::other("block call failed"));
            }
        };
        let data_operation = matches!(operation, Operation::Read | Operation::Write);
        if response[0] != id
            || response[3] != 1
            || response[1] != Status::Ok as u64
            || (data_operation && response[2] != block::BLOCK_SIZE as u64)
            || (operation == Operation::Flush && response[2] != 0)
        {
            // A malformed or failed accepted exchange cannot establish completion.
            self.poisoned.store(true, Ordering::Release);
            return Err(io::Error::other("invalid or failed block completion"));
        }
        Ok(response[2])
    }
}

impl<T: Exchange> Volume<T> {
    pub fn connect(transport: T) -> io::Result<(Self, Arc<AtomicBool>)> {
        let poisoned = Arc::new(AtomicBool::new(false));
        let mut session = Session {
            transport,
            next_id: 1,
            poisoned: poisoned.clone(),
        };
        let info = session.request(Operation::Info, 0, None)?;
        if info & block::INFO_READ_ONLY != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "read-only block device",
            ));
        }
        let blocks = info
            .checked_sub(1)
            .filter(|blocks| *blocks >= 4)
            .ok_or_else(|| io::Error::other("volume too small"))?;
        Ok((
            Self {
                session: RefCell::new(session),
                blocks,
            },
            poisoned,
        ))
    }

    fn physical(&self, logical: u64) -> io::Result<u64> {
        if logical >= self.blocks {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        logical
            .checked_add(1)
            .ok_or_else(|| io::Error::other("block offset overflow"))
    }
}

impl<T: Exchange> BlockDevice for Volume<T> {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&self, id: u64, output: &mut Block) -> io::Result<()> {
        let physical = self.physical(id)?;
        let mut snapshot = [0; block::BLOCK_SIZE];
        self.session
            .borrow_mut()
            .request(Operation::Read, physical, Some(&mut snapshot))?;
        output.copy_from_slice(&snapshot);
        Ok(())
    }

    fn write_block(&mut self, id: u64, input: &Block) -> io::Result<()> {
        let physical = self.physical(id)?;
        let mut snapshot = *input;
        self.session
            .get_mut()
            .request(Operation::Write, physical, Some(&mut snapshot))?;
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        self.session.get_mut().request(Operation::Flush, 0, None)?;
        Ok(())
    }
}

pub struct Native {
    read: Handle,
    write: Handle,
}

impl Native {
    pub fn new(read: Handle, write: Handle) -> Self {
        Self { read, write }
    }
}

impl Exchange for Native {
    fn exchange(
        &mut self,
        operation: Operation,
        physical: u64,
        id: u64,
        data: Option<&mut Block>,
    ) -> Result<[u64; 4], Fault> {
        let endpoint = if matches!(operation, Operation::Info | Operation::Read) {
            self.read
        } else {
            self.write
        };
        let request = [block::header(operation), 1, physical, id];
        let Some(data) = data else {
            return sys::call(endpoint, request).map_err(|error| Fault {
                accepted: error.accepted,
            });
        };
        // RefCell serializes the sole session. This window is unique among active
        // requests and becomes vacant before a successfully completed call returns.
        let address = 0x0400_0000;
        let memory = sys::memory_create(1).map_err(|_| Fault { accepted: false })?;
        if unsafe { sys::memory_map(memory, address, 0, true) }.is_err() {
            let _ = sys::close(memory);
            return Err(Fault { accepted: false });
        }
        if operation == Operation::Write {
            for (index, byte) in data.iter().enumerate() {
                unsafe { (address as *mut u8).add(index).write_volatile(*byte) };
            }
        }
        let rights = sys::READ
            | sys::MAP
            | if operation == Operation::Read {
                sys::WRITE
            } else {
                0
            };
        let response = sys::call_caps(
            endpoint,
            request,
            &[Grant {
                handle: memory,
                rights,
            }],
        );
        if let Ok(message) = response
            && operation == Operation::Read
            && message == block::response(id, Status::Ok, block::BLOCK_SIZE as u64, 1)
        {
            for (index, byte) in data.iter_mut().enumerate() {
                *byte = unsafe { (address as *const u8).add(index).read_volatile() };
            }
        }
        // Every request owns fresh backing. Accepted failures poison the session;
        // a retained server loan can never expose backing to a later call.
        if unsafe { sys::memory_unmap(address) }.is_err() {
            sys::exit(247);
        }
        if sys::close(memory).is_err() {
            sys::exit(246);
        }
        response.map_err(|error| Fault {
            accepted: error.accepted,
        })
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        let _ = sys::close(self.read);
        let _ = sys::close(self.write);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use betafs_core::{Compression, FileSystem, ROOT_INODE};
    use std::sync::Mutex;

    #[derive(Clone, Copy)]
    enum Injection {
        None,
        WrongId,
        AcceptedWrite,
        RejectedCall,
    }

    struct Disk {
        blocks: Vec<Block>,
        durable: Vec<Block>,
        calls: Vec<(Operation, u64)>,
        injection: Injection,
    }

    #[derive(Clone)]
    pub(crate) struct Model(Arc<Mutex<Disk>>);

    impl Model {
        pub(crate) fn fail_next_write(&self) {
            self.0.lock().unwrap().injection = Injection::AcceptedWrite;
        }

        pub(crate) fn new() -> Self {
            let mut blocks = vec![[0; 4096]; 256];
            let marker = b"CRYOLITE-BLOCK-PROBE-V1";
            blocks[0][..marker.len()].copy_from_slice(marker);
            Self(Arc::new(Mutex::new(Disk {
                durable: blocks.clone(),
                blocks,
                calls: Vec::new(),
                injection: Injection::None,
            })))
        }
    }

    impl Exchange for Model {
        fn exchange(
            &mut self,
            operation: Operation,
            physical: u64,
            id: u64,
            data: Option<&mut Block>,
        ) -> Result<[u64; 4], Fault> {
            let mut disk = self.0.lock().unwrap();
            disk.calls.push((operation, physical));
            let injection = if matches!(disk.injection, Injection::AcceptedWrite)
                && operation != Operation::Write
            {
                Injection::None
            } else {
                std::mem::replace(&mut disk.injection, Injection::None)
            };
            if matches!(injection, Injection::RejectedCall) {
                return Err(Fault { accepted: false });
            }
            let count = match operation {
                Operation::Info => disk.blocks.len() as u64,
                Operation::Read => {
                    data.unwrap()
                        .copy_from_slice(&disk.blocks[physical as usize]);
                    4096
                }
                Operation::Write => {
                    disk.blocks[physical as usize].copy_from_slice(data.unwrap());
                    if matches!(injection, Injection::AcceptedWrite) {
                        return Err(Fault { accepted: true });
                    }
                    4096
                }
                Operation::Flush => {
                    disk.durable = disk.blocks.clone();
                    0
                }
                Operation::Stop => unreachable!(),
            };
            Ok(block::response(
                if matches!(injection, Injection::WrongId) {
                    id + 1
                } else {
                    id
                },
                Status::Ok,
                count,
                1,
            ))
        }
    }

    #[test]
    fn actual_engine_reopens_after_barrier_and_never_addresses_marker_block() {
        let model = Model::new();
        let marker = model.0.lock().unwrap().blocks[0];
        let (volume, _) = Volume::connect(model.clone()).unwrap();
        assert_eq!(volume.block_count(), 255);
        let mut fs = FileSystem::format(volume).unwrap();
        let inode = fs
            .create_file_at(
                ROOT_INODE,
                b"native\xff",
                b"persisted",
                Compression::default(),
            )
            .unwrap();
        fs.write_at(inode, 32, b"tail", Compression::default())
            .unwrap();
        fs.sync().unwrap();
        drop(fs);
        {
            let mut disk = model.0.lock().unwrap();
            disk.blocks = disk.durable.clone();
            assert_eq!(disk.blocks[0], marker);
            assert!(
                disk.calls
                    .iter()
                    .filter(|(op, _)| matches!(op, Operation::Read | Operation::Write))
                    .all(|(_, physical)| *physical >= 1)
            );
        }
        let (volume, _) = Volume::connect(model).unwrap();
        let fs = FileSystem::open(volume).unwrap();
        let inode = fs.lookup(ROOT_INODE, b"native\xff").unwrap();
        let data = fs.read_at(inode, 0, 36).unwrap();
        assert_eq!(data.len(), 36);
        assert_eq!(&data[..9], b"persisted");
        assert!(data[9..32].iter().all(|byte| *byte == 0));
        assert_eq!(&data[32..], b"tail");
    }

    #[test]
    fn malformed_read_keeps_output_private_and_poisoned_session_never_retries() {
        let model = Model::new();
        let (volume, health) = Volume::connect(model.clone()).unwrap();
        model.0.lock().unwrap().injection = Injection::WrongId;
        let mut output = [0x5a; 4096];
        assert!(volume.read_block(0, &mut output).is_err());
        assert_eq!(output, [0x5a; 4096]);
        assert!(health.load(Ordering::Acquire));
        let count = model.0.lock().unwrap().calls.len();
        assert!(volume.read_block(0, &mut output).is_err());
        assert_eq!(model.0.lock().unwrap().calls.len(), count);
    }

    #[test]
    fn accepted_write_failure_retains_uncertain_media_and_blocks_further_calls() {
        let model = Model::new();
        let (mut volume, health) = Volume::connect(model.clone()).unwrap();
        model.0.lock().unwrap().injection = Injection::AcceptedWrite;
        assert!(volume.write_block(3, &[0x42; 4096]).is_err());
        assert_eq!(model.0.lock().unwrap().blocks[4], [0x42; 4096]);
        assert!(health.load(Ordering::Acquire));
        let count = model.0.lock().unwrap().calls.len();
        assert!(volume.sync().is_err());
        assert_eq!(model.0.lock().unwrap().calls.len(), count);
    }

    #[test]
    fn rejected_call_and_range_error_do_not_touch_media_or_poison_session() {
        let model = Model::new();
        let (mut volume, health) = Volume::connect(model.clone()).unwrap();
        let count = model.0.lock().unwrap().calls.len();
        assert!(volume.write_block(255, &[1; 4096]).is_err());
        assert!(volume.write_block(u64::MAX, &[1; 4096]).is_err());
        assert_eq!(model.0.lock().unwrap().calls.len(), count);
        model.0.lock().unwrap().injection = Injection::RejectedCall;
        assert!(volume.write_block(0, &[1; 4096]).is_err());
        assert!(!health.load(Ordering::Acquire));
        volume.write_block(0, &[2; 4096]).unwrap();
        assert_eq!(model.0.lock().unwrap().blocks[1], [2; 4096]);
    }

    #[test]
    fn actual_engine_format_rejects_existing_superblocks_without_reformatting() {
        let model = Model::new();
        let (volume, _) = Volume::connect(model.clone()).unwrap();
        let fs = FileSystem::format(volume).unwrap();
        drop(fs);
        let before = model.0.lock().unwrap().blocks.clone();
        let (volume, _) = Volume::connect(model.clone()).unwrap();
        assert!(FileSystem::format(volume).is_err());
        assert_eq!(model.0.lock().unwrap().blocks, before);
    }

    #[test]
    fn actual_engine_mutation_failure_does_not_retry_accepted_partial_storage_work() {
        let model = Model::new();
        let (volume, health) = Volume::connect(model.clone()).unwrap();
        let mut fs = FileSystem::format(volume).unwrap();
        let inode = fs
            .create_file_at(ROOT_INODE, b"probe", b"before", Compression::default())
            .unwrap();
        model.fail_next_write();
        assert!(
            fs.write_at(inode, 0, b"after", Compression::default())
                .is_err()
        );
        assert!(health.load(Ordering::Acquire));
        let calls = model.0.lock().unwrap().calls.len();
        assert!(fs.sync().is_err());
        assert_eq!(model.0.lock().unwrap().calls.len(), calls);
    }
}
