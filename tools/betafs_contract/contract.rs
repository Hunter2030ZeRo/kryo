//! Host-only contract checks against the independent BetAFS engine.
//! The transport below is a fault-injected wire model, not native IPC or DMA.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use betafs_block::{BLOCK_SIZE, Block, BlockDevice};
    use betafs_core::{Compression, CreateAttributes, Error, FileSystem, ROOT_INODE};
    use kryo_interfaces::{block as wire, file};
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::sync::{Arc, Mutex};

    const INSTANCE: u64 = 7;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Fault {
        None,
        BadId,
        BadInstance,
        ShortRead,
        ShortWrite,
        BadFlushLength,
        WriteIo,
        FlushIo,
        Accepted,
        Undelivered,
    }

    struct Disk {
        visible: Vec<Block>,
        durable: Vec<Block>,
        read_only: bool,
        fault: Fault,
        calls: usize,
        last_block: Option<u64>,
        loan_ids: Vec<u64>,
        retained_loans: Vec<Block>,
    }

    impl Disk {
        fn new(blocks: usize) -> Arc<Mutex<Self>> {
            Arc::new(Mutex::new(Self {
                visible: vec![[0; BLOCK_SIZE]; blocks],
                durable: vec![[0; BLOCK_SIZE]; blocks],
                read_only: false,
                fault: Fault::None,
                calls: 0,
                last_block: None,
                loan_ids: Vec::new(),
                retained_loans: Vec::new(),
            }))
        }

        fn crash(&mut self) {
            self.visible.clone_from(&self.durable);
            self.fault = Fault::None;
            self.retained_loans.clear();
        }
    }

    struct TransportFailure {
        accepted: bool,
    }

    struct Port {
        disk: Arc<Mutex<Disk>>,
    }

    impl Port {
        fn call(
            &mut self,
            gate: wire::Gate,
            message: [u64; 4],
            loan: Option<(u64, &mut Block)>,
        ) -> Result<[u64; 4], TransportFailure> {
            let mut disk = self.disk.lock().unwrap();
            disk.calls += 1;
            let fault = disk.fault;
            if fault == Fault::Undelivered {
                disk.fault = Fault::None;
                return Err(TransportFailure { accepted: false });
            }
            let request = match wire::Request::decode(
                message,
                gate,
                INSTANCE,
                disk.visible.len() as u64,
                disk.read_only,
                usize::from(loan.is_some()),
                false,
            ) {
                Ok(request) => request,
                Err(status) => return Ok(wire::response(message[3], status, 0, INSTANCE)),
            };
            if let Some((id, _)) = loan.as_ref() {
                disk.loan_ids.push(*id);
            }
            let mut status = wire::Status::Ok;
            let mut value = 0;
            match request.operation {
                wire::Operation::Info => {
                    value = disk.visible.len() as u64
                        | if disk.read_only {
                            wire::INFO_READ_ONLY
                        } else {
                            0
                        };
                }
                wire::Operation::Read => {
                    disk.last_block = Some(request.block);
                    let (_, bytes) = loan.unwrap();
                    *bytes = disk.visible[request.block as usize];
                    value = if fault == Fault::ShortRead {
                        4095
                    } else {
                        4096
                    };
                }
                wire::Operation::Write => {
                    disk.last_block = Some(request.block);
                    let (_, bytes) = loan.unwrap();
                    if fault == Fault::WriteIo {
                        // A failed write can have changed an arbitrary prefix.
                        disk.visible[request.block as usize][..31].copy_from_slice(&bytes[..31]);
                        status = wire::Status::Io;
                    } else {
                        disk.visible[request.block as usize] = *bytes;
                        value = if fault == Fault::ShortWrite {
                            4095
                        } else {
                            4096
                        };
                    }
                }
                wire::Operation::Flush => {
                    if fault == Fault::FlushIo {
                        status = wire::Status::Uncertain;
                    } else {
                        let visible = disk.visible.clone();
                        disk.durable = visible;
                        if fault == Fault::BadFlushLength {
                            value = 1;
                        }
                    }
                }
                wire::Operation::Stop => unreachable!(),
            }
            if fault == Fault::Accepted {
                // Retained owned bytes model a service owner outliving cancellation.
                let retained = disk.visible[0];
                disk.retained_loans.push(retained);
                disk.fault = Fault::None;
                return Err(TransportFailure { accepted: true });
            }
            if matches!(
                fault,
                Fault::BadId
                    | Fault::BadInstance
                    | Fault::ShortRead
                    | Fault::ShortWrite
                    | Fault::BadFlushLength
            ) {
                disk.fault = Fault::None;
            }
            Ok(wire::response(
                request.id + u64::from(fault == Fault::BadId),
                status,
                value,
                INSTANCE + u64::from(fault == Fault::BadInstance),
            ))
        }
    }

    struct WireDevice {
        port: RefCell<Port>,
        blocks: u64,
        read_only: bool,
        next_id: Cell<u64>,
        next_loan: Cell<u64>,
        poisoned: Cell<bool>,
    }

    fn invalid(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }

    impl WireDevice {
        fn open(disk: Arc<Mutex<Disk>>) -> Self {
            let mut port = Port { disk };
            let reply = port
                .call(
                    wire::Gate::Read,
                    [wire::header(wire::Operation::Info), INSTANCE, 0, 1],
                    None,
                )
                .unwrap_or_else(|_| panic!("fixture Info failed"));
            assert_eq!(reply[0], 1);
            assert_eq!(reply[1], wire::Status::Ok as u64);
            assert_eq!(reply[3], INSTANCE);
            let blocks = reply[2] & !wire::INFO_READ_ONLY;
            assert!(blocks > 0);
            Self {
                port: RefCell::new(port),
                blocks,
                read_only: reply[2] & wire::INFO_READ_ONLY != 0,
                next_id: Cell::new(2),
                next_loan: Cell::new(1),
                poisoned: Cell::new(false),
            }
        }

        fn request(
            &self,
            operation: wire::Operation,
            block: u64,
            loan: Option<&mut Block>,
        ) -> io::Result<()> {
            if self.poisoned.get() {
                return Err(io::Error::other("reopen required"));
            }
            let mutating = matches!(operation, wire::Operation::Write | wire::Operation::Flush);
            if self.read_only && mutating {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "read-only device",
                ));
            }
            if operation != wire::Operation::Flush && block >= self.blocks {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "block outside device",
                ));
            }
            let id = self.next_id.get();
            if id == u64::MAX {
                return Err(invalid("request id exhausted"));
            }
            self.next_id.set(id + 1);
            let loan = loan.map(|bytes| {
                let id = self.next_loan.get();
                self.next_loan
                    .set(id.checked_add(1).expect("fixture loan id exhausted"));
                (id, bytes)
            });
            let reply = match self.port.borrow_mut().call(
                if mutating {
                    wire::Gate::Write
                } else {
                    wire::Gate::Read
                },
                [wire::header(operation), INSTANCE, block, id],
                loan,
            ) {
                Ok(reply) => reply,
                Err(failure) => {
                    if failure.accepted {
                        self.poisoned.set(true);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "modeled transport failure",
                    ));
                }
            };
            let expected = if operation == wire::Operation::Flush {
                0
            } else {
                BLOCK_SIZE as u64
            };
            if reply[0] != id || reply[3] != INSTANCE || (reply[1] == 0 && reply[2] != expected) {
                self.poisoned.set(true);
                return Err(invalid("malformed block response"));
            }
            if reply[1] != wire::Status::Ok as u64 {
                if mutating {
                    self.poisoned.set(true);
                }
                return Err(io::Error::other("block service rejected request"));
            }
            Ok(())
        }
    }

    impl BlockDevice for WireDevice {
        fn block_count(&self) -> u64 {
            self.blocks
        }

        fn read_block(&self, id: u64, block: &mut Block) -> io::Result<()> {
            let mut scratch = [0; BLOCK_SIZE];
            self.request(wire::Operation::Read, id, Some(&mut scratch))?;
            *block = scratch;
            Ok(())
        }

        fn write_block(&mut self, id: u64, block: &Block) -> io::Result<()> {
            let mut scratch = *block;
            self.request(wire::Operation::Write, id, Some(&mut scratch))
        }

        fn sync(&mut self) -> io::Result<()> {
            self.request(wire::Operation::Flush, 0, None)
        }
    }

    fn filesystem() -> (FileSystem, Arc<Mutex<Disk>>) {
        let disk = Disk::new(2048);
        let fs = FileSystem::format(WireDevice::open(disk.clone())).unwrap();
        (fs, disk)
    }

    fn bytes(size: usize) -> Vec<u8> {
        (0..size).map(|index| (index * 37 + 11) as u8).collect()
    }

    #[test]
    fn block_indices_stay_in_4096_byte_units() {
        assert_eq!(BLOCK_SIZE, wire::BLOCK_SIZE);
        let disk = Disk::new(64);
        let mut device = WireDevice::open(disk.clone());
        device.write_block(37, &[0x53; BLOCK_SIZE]).unwrap();
        assert_eq!(disk.lock().unwrap().last_block, Some(37));
        assert_eq!(disk.lock().unwrap().visible[37], [0x53; BLOCK_SIZE]);
        assert_eq!(disk.lock().unwrap().visible[8], [0; BLOCK_SIZE]);
    }

    #[test]
    fn read_your_writes_and_flush_have_distinct_durability() {
        let disk = Disk::new(8);
        let mut device = WireDevice::open(disk.clone());
        device.write_block(2, &[0x53; BLOCK_SIZE]).unwrap();
        let mut block = [0; BLOCK_SIZE];
        device.read_block(2, &mut block).unwrap();
        assert_eq!(block, [0x53; BLOCK_SIZE]);
        disk.lock().unwrap().crash();
        device.read_block(2, &mut block).unwrap();
        assert_eq!(block, [0; BLOCK_SIZE]);
        device.write_block(2, &[0x71; BLOCK_SIZE]).unwrap();
        device.sync().unwrap();
        disk.lock().unwrap().crash();
        device.read_block(2, &mut block).unwrap();
        assert_eq!(block, [0x71; BLOCK_SIZE]);
    }

    #[test]
    fn malformed_reply_identity_never_publishes_read_bytes() {
        for fault in [Fault::BadId, Fault::BadInstance] {
            let disk = Disk::new(8);
            let device = WireDevice::open(disk.clone());
            disk.lock().unwrap().fault = fault;
            let mut block = [0xa5; BLOCK_SIZE];
            assert!(device.read_block(0, &mut block).is_err());
            assert_eq!(block, [0xa5; BLOCK_SIZE]);
        }
    }

    #[test]
    fn short_read_is_not_success_and_preserves_the_output() {
        let disk = Disk::new(8);
        let device = WireDevice::open(disk.clone());
        disk.lock().unwrap().fault = Fault::ShortRead;
        let mut block = [0xa5; BLOCK_SIZE];
        assert!(device.read_block(0, &mut block).is_err());
        assert_eq!(block, [0xa5; BLOCK_SIZE]);
    }

    #[test]
    fn short_write_and_nonzero_flush_completion_require_reopen() {
        for (fault, operation) in [
            (Fault::ShortWrite, wire::Operation::Write),
            (Fault::BadFlushLength, wire::Operation::Flush),
        ] {
            let disk = Disk::new(8);
            let mut device = WireDevice::open(disk.clone());
            disk.lock().unwrap().fault = fault;
            let result = if operation == wire::Operation::Write {
                device.write_block(0, &[1; BLOCK_SIZE])
            } else {
                device.sync()
            };
            assert!(result.is_err());
            let calls = disk.lock().unwrap().calls;
            assert!(device.sync().is_err());
            assert_eq!(disk.lock().unwrap().calls, calls);
        }
    }

    #[test]
    fn accepted_transport_failure_does_not_replay_or_reuse_a_loan() {
        let disk = Disk::new(8);
        let mut device = WireDevice::open(disk.clone());
        disk.lock().unwrap().fault = Fault::Accepted;
        assert!(device.write_block(0, &[1; BLOCK_SIZE]).is_err());
        let calls = disk.lock().unwrap().calls;
        assert!(device.write_block(0, &[1; BLOCK_SIZE]).is_err());
        assert_eq!(disk.lock().unwrap().calls, calls);
        assert_eq!(disk.lock().unwrap().loan_ids, [1]);
        assert_eq!(disk.lock().unwrap().retained_loans.len(), 1);
    }

    #[test]
    fn undelivered_failure_can_retry_with_fresh_backing() {
        let disk = Disk::new(8);
        let mut device = WireDevice::open(disk.clone());
        disk.lock().unwrap().fault = Fault::Undelivered;
        assert!(device.write_block(0, &[1; BLOCK_SIZE]).is_err());
        device.write_block(0, &[2; BLOCK_SIZE]).unwrap();
        assert_eq!(disk.lock().unwrap().loan_ids, [2]);
    }

    #[test]
    fn read_only_and_exhausted_ids_do_not_issue_mutating_requests() {
        let disk = Disk::new(8);
        disk.lock().unwrap().read_only = true;
        let mut device = WireDevice::open(disk.clone());
        let calls = disk.lock().unwrap().calls;
        assert!(device.write_block(0, &[1; BLOCK_SIZE]).is_err());
        assert!(device.sync().is_err());
        assert_eq!(disk.lock().unwrap().calls, calls);
        device.next_id.set(u64::MAX);
        assert!(device.read_block(0, &mut [0; BLOCK_SIZE]).is_err());
        assert_eq!(disk.lock().unwrap().calls, calls);
    }

    #[test]
    fn server_wire_validation_rejects_wrong_authority_and_stale_requests() {
        let disk = Disk::new(8);
        let mut port = Port { disk };
        let denied = port
            .call(
                wire::Gate::Read,
                [wire::header(wire::Operation::Write), INSTANCE, 0, 1],
                Some((1, &mut [0; BLOCK_SIZE])),
            )
            .unwrap_or_else(|_| panic!());
        assert_eq!(denied[1], wire::Status::Denied as u64);
        let stale = port
            .call(
                wire::Gate::Read,
                [wire::header(wire::Operation::Info), INSTANCE + 1, 0, 2],
                None,
            )
            .unwrap_or_else(|_| panic!());
        assert_eq!(stale[1], wire::Status::Stale as u64);
        assert!(
            wire::Request::decode(
                [wire::header(wire::Operation::Read), INSTANCE, 8, 3],
                wire::Gate::Read,
                INSTANCE,
                8,
                false,
                1,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn real_engine_roundtrips_multiple_rpc_sized_writes_and_reopens() {
        let (mut fs, disk) = filesystem();
        assert!(!disk.lock().unwrap().visible[0].starts_with(b"CRYOLITE-BLOCK-PROBE-V1"));
        let inode = fs
            .create_file_at(ROOT_INODE, b"probe", &[], Compression::Raw)
            .unwrap();
        let data = bytes(12_345);
        assert!(BLOCK_SIZE <= betafs_core::MAX_WRITE_SIZE);
        for (index, part) in data.chunks(BLOCK_SIZE).enumerate() {
            fs.write_at(inode, (index * BLOCK_SIZE) as u64, part, Compression::Raw)
                .unwrap();
        }
        assert_eq!(fs.read_at(inode, 0, data.len() + 17).unwrap(), data);
        assert!(fs.read_at(inode, data.len() as u64, 1).unwrap().is_empty());
        fs.truncate(inode, 6001).unwrap();
        fs.sync().unwrap();
        drop(fs);
        disk.lock().unwrap().crash();
        let fs = FileSystem::open(WireDevice::open(disk)).unwrap();
        assert_eq!(
            fs.read_at(fs.lookup(ROOT_INODE, b"probe").unwrap(), 0, 7000)
                .unwrap(),
            data[..6001]
        );
        fs.check().unwrap();
    }

    #[test]
    fn byte_names_and_creation_metadata_do_not_require_utf8_paths() {
        let (mut fs, _) = filesystem();
        let name = b"file-\xff";
        let inode = fs
            .create_file_with_attributes(
                ROOT_INODE,
                name,
                b"payload",
                Compression::Raw,
                CreateAttributes {
                    mode: 0o640,
                    uid: 1000,
                    gid: 1001,
                },
            )
            .unwrap();
        assert_eq!(fs.lookup(ROOT_INODE, name).unwrap(), inode);
        let metadata = fs.metadata(inode).unwrap();
        assert_eq!(
            (metadata.mode, metadata.uid, metadata.gid),
            (0o640, 1000, 1001)
        );
    }

    #[test]
    fn open_description_pins_keep_unlinked_data_until_the_final_release() {
        let (mut fs, _) = filesystem();
        let inode = fs
            .create_file_at(ROOT_INODE, b"open", b"retained", Compression::Raw)
            .unwrap();
        fs.pin_inode(inode).unwrap();
        fs.pin_inode(inode).unwrap();
        fs.unlink_at(ROOT_INODE, b"open").unwrap();
        assert!(matches!(
            fs.lookup(ROOT_INODE, b"open"),
            Err(Error::NotFound)
        ));
        fs.unpin_inode(inode).unwrap();
        assert_eq!(fs.read_at(inode, 0, 20).unwrap(), b"retained");
        fs.unpin_inode(inode).unwrap();
        assert!(matches!(fs.metadata(inode), Err(Error::NotFound)));
    }

    #[test]
    fn approved_cursor_and_append_contract_map_to_real_engine_calls() {
        let (mut fs, _) = filesystem();
        let inode = fs
            .create_file_at(ROOT_INODE, b"append", b"first", Compression::Raw)
            .unwrap();
        let options =
            file::OpenOptions::decode(u64::from(file::READ_WRITE | file::APPEND)).unwrap();
        assert!(options.readable() && options.writable());
        let cursor = file::seek_position(5, 5, 0, 0).unwrap();
        assert_eq!(cursor, 0);
        let position = if options.flags & file::APPEND != 0 {
            fs.metadata(inode).unwrap().size
        } else {
            cursor
        };
        fs.write_at(inode, position, b"second", Compression::Raw)
            .unwrap();
        assert_eq!(fs.read_at(inode, 0, 20).unwrap(), b"firstsecond");
        assert_eq!(file::seek_position(position + 6, 11, -6, 1).unwrap(), 5);
    }

    #[test]
    fn failed_publication_write_preserves_engine_uncertainty_and_stops_retry() {
        let (mut fs, disk) = filesystem();
        let inode = fs
            .create_file_at(ROOT_INODE, b"fault", b"old", Compression::Raw)
            .unwrap();
        disk.lock().unwrap().fault = Fault::WriteIo;
        assert!(matches!(
            fs.write_at(inode, 0, b"new", Compression::Raw),
            Err(Error::Tree(betafs_tree::Error::CommitUncertain(_)))
        ));
        let calls = disk.lock().unwrap().calls;
        assert!(matches!(
            fs.sync(),
            Err(Error::Tree(betafs_tree::Error::NeedsReopen))
        ));
        assert_eq!(disk.lock().unwrap().calls, calls);
    }

    #[test]
    fn failed_flush_preserves_engine_uncertainty_and_stops_retry() {
        let (mut fs, disk) = filesystem();
        disk.lock().unwrap().fault = Fault::FlushIo;
        assert!(matches!(
            fs.sync(),
            Err(Error::Tree(betafs_tree::Error::CommitUncertain(_)))
        ));
        let calls = disk.lock().unwrap().calls;
        assert!(matches!(
            fs.sync(),
            Err(Error::Tree(betafs_tree::Error::NeedsReopen))
        ));
        assert_eq!(disk.lock().unwrap().calls, calls);
    }
}
