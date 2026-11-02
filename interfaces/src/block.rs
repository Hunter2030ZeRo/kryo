//! Version-one block service messages. Gate capabilities carry authority;
//! instance and request numbers only reject stale or malformed traffic.

pub const VERSION: u64 = 1;
pub const SERVICE: u64 = 40;
pub const BLOCK_SIZE: usize = 4096;
pub const INFO_READ_ONLY: u64 = 1 << 63;

pub mod role {
    pub const DEVICE: u64 = 1;
    pub const READ_GATE: u64 = 2;
    pub const WRITE_GATE: u64 = 3;
    pub const CONTROL_GATE: u64 = 4;
    pub const DEBUG: u64 = 5;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Read,
    Write,
    Control,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Operation {
    Info = 1,
    Read = 2,
    Write = 3,
    Flush = 4,
    Stop = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Status {
    Ok = 0,
    Malformed = 1,
    Denied = 2,
    Range = 3,
    ReadOnly = 4,
    Io = 5,
    Uncertain = 6,
    Unavailable = 7,
    Stale = 8,
}

pub const fn header(operation: Operation) -> u64 {
    (VERSION << 32) | operation as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub operation: Operation,
    pub block: u64,
    pub id: u64,
}

impl Request {
    pub fn decode(
        message: [u64; 4],
        gate: Gate,
        instance: u64,
        blocks: u64,
        read_only: bool,
        capabilities: usize,
        allow_stop: bool,
    ) -> Result<Self, Status> {
        if message[0] >> 32 != VERSION || message[3] == 0 {
            return Err(Status::Malformed);
        }
        if message[1] != instance {
            return Err(Status::Stale);
        }
        let operation = match message[0] as u32 {
            1 => Operation::Info,
            2 => Operation::Read,
            3 => Operation::Write,
            4 => Operation::Flush,
            5 => Operation::Stop,
            _ => return Err(Status::Malformed),
        };
        let allowed = match gate {
            Gate::Read => matches!(operation, Operation::Info | Operation::Read),
            Gate::Write => operation != Operation::Stop,
            Gate::Control => operation == Operation::Stop && allow_stop,
        };
        if !allowed {
            return Err(Status::Denied);
        }
        let data = matches!(operation, Operation::Read | Operation::Write);
        if capabilities != usize::from(data) || (!data && message[2] != 0) {
            return Err(Status::Malformed);
        }
        if data && message[2] >= blocks {
            return Err(Status::Range);
        }
        if read_only && matches!(operation, Operation::Write | Operation::Flush) {
            return Err(Status::ReadOnly);
        }
        Ok(Self {
            operation,
            block: message[2],
            id: message[3],
        })
    }
}

pub const fn response(id: u64, status: Status, value: u64, instance: u64) -> [u64; 4] {
    [id, status as u64, value, instance]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decode(message: [u64; 4], gate: Gate, count: usize) -> Result<Request, Status> {
        Request::decode(message, gate, 7, 16, false, count, true)
    }

    #[test]
    fn rejects_bad_versions_unknown_operations_zero_ids_and_stale_instances() {
        for message in [
            [1, 7, 0, 1],
            [(1 << 32) | 99, 7, 0, 1],
            [header(Operation::Info), 7, 0, 0],
        ] {
            assert_eq!(decode(message, Gate::Read, 0), Err(Status::Malformed));
        }
        assert_eq!(
            decode([header(Operation::Info), 6, 0, 1], Gate::Read, 0),
            Err(Status::Stale)
        );
    }
    #[test]
    fn each_gate_carries_its_own_authority() {
        assert_eq!(
            decode([header(Operation::Write), 7, 1, 1], Gate::Read, 1),
            Err(Status::Denied)
        );
        assert_eq!(
            decode([header(Operation::Flush), 7, 0, 1], Gate::Read, 0),
            Err(Status::Denied)
        );
        assert_eq!(
            decode([header(Operation::Stop), 7, 0, 1], Gate::Write, 0),
            Err(Status::Denied)
        );
        assert_eq!(
            decode([header(Operation::Read), 7, 1, 1], Gate::Control, 1),
            Err(Status::Denied)
        );
        assert!(decode([header(Operation::Stop), 7, 0, 1], Gate::Control, 0).is_ok());
        assert_eq!(
            Request::decode(
                [header(Operation::Stop), 7, 0, 1],
                Gate::Control,
                7,
                16,
                false,
                0,
                false
            ),
            Err(Status::Denied)
        );
    }
    #[test]
    fn validates_full_fixed_block_and_capability_count() {
        for count in [0, 2] {
            assert_eq!(
                decode([header(Operation::Read), 7, 0, 1], Gate::Read, count),
                Err(Status::Malformed)
            );
        }
        assert_eq!(
            decode([header(Operation::Read), 7, 16, 1], Gate::Read, 1),
            Err(Status::Range)
        );
        assert_eq!(
            decode([header(Operation::Write), 7, u64::MAX, 1], Gate::Write, 1),
            Err(Status::Range)
        );
        assert!(decode([header(Operation::Read), 7, 15, 1], Gate::Read, 1).is_ok());
        assert_eq!(
            decode([header(Operation::Info), 7, 1, 1], Gate::Read, 0),
            Err(Status::Malformed)
        );
        assert_eq!(
            decode([header(Operation::Info), 7, 0, 1], Gate::Read, 1),
            Err(Status::Malformed)
        );
    }
    #[test]
    fn readonly_device_rejects_mutation_after_gate_validation() {
        for operation in [Operation::Write, Operation::Flush] {
            assert_eq!(
                Request::decode(
                    [header(operation), 7, 0, 1],
                    Gate::Write,
                    7,
                    16,
                    true,
                    usize::from(operation == Operation::Write),
                    true
                ),
                Err(Status::ReadOnly)
            );
        }
        assert!(
            Request::decode(
                [header(Operation::Read), 7, 0, 1],
                Gate::Read,
                7,
                16,
                true,
                1,
                true
            )
            .is_ok()
        );
    }
}
