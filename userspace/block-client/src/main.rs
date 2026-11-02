#![no_std]
#![no_main]
use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_interfaces::block::{self, Operation as Op, Status, role};
use kryo_libsys::{self as sys, Grant, Handle, ObjectKind};
use kryo_services::{Manifest, RoleRule, ServiceId};

const VIEW: u64 = 0x0200_0000;
const BLOCK: usize = 4096;
global_asm!(r#"
    .section .text.start,"ax"
    .global _start
_start:
    cld
    xor ebp, ebp
    call {main}
    ud2
"#,main=sym client_main);

extern "C" fn client_main(raw: u64) -> ! {
    let result = Handle::from_raw(raw).ok_or(200).and_then(run);
    sys::exit(result.err().unwrap_or(0))
}

fn run(record: Handle) -> Result<(), u64> {
    let startup = sys::startup(record).map_err(|_| 201u64)?;
    let mode = startup.argument;
    if !(41..=44).contains(&startup.service) || mode > 3 {
        return Err(202);
    }
    let rules = [
        RoleRule {
            role: role::READ_GATE,
            kind: ObjectKind::Endpoint,
            rights: sys::SEND,
            required: mode != 2,
        },
        RoleRule {
            role: role::WRITE_GATE,
            kind: ObjectKind::Endpoint,
            rights: sys::SEND,
            required: mode == 1,
        },
        RoleRule {
            role: role::CONTROL_GATE,
            kind: ObjectKind::Endpoint,
            rights: sys::SEND,
            required: mode == 2,
        },
        RoleRule {
            role: role::DEBUG,
            kind: ObjectKind::DebugConsole,
            rights: sys::WRITE,
            required: true,
        },
    ];
    Manifest {
        service: ServiceId::new(startup.service).map_err(|_| 203u64)?,
        roles: &rules,
    }
    .validate(&startup)
    .map_err(|_| 204u64)?;
    sys::close(record).map_err(|_| 205u64)?;
    let debug = startup.grant(role::DEBUG).unwrap().handle;
    if mode == 2 {
        let gate = startup.grant(role::CONTROL_GATE).unwrap().handle;
        request(
            gate,
            [block::header(Op::Info), 1, 0, 1],
            &[],
            Status::Denied,
        )?;
        request(gate, [block::header(Op::Stop), 1, 0, 2], &[], Status::Ok)?;
        sys::debug_log(debug, 93, mode).map_err(|_| 206u64)?;
        return Ok(());
    }
    let read = startup.grant(role::READ_GATE).unwrap().handle;
    let memory = sys::memory_create(1).map_err(|_| 207u64)?;
    unsafe { sys::memory_map(memory, VIEW, 0, true) }.map_err(|_| 208u64)?;
    let output = Grant {
        handle: memory,
        rights: sys::READ | sys::WRITE | sys::MAP,
    };
    let input = Grant {
        handle: memory,
        rights: sys::READ | sys::MAP,
    };
    if mode == 3 {
        let result = sys::call_caps_until(
            read,
            [block::header(Op::Read), 1, 0, 1],
            &[output],
            deadline()?,
        );
        if !matches!(result, Err(failure) if failure.error == sys::Error::Kernel(-8) && failure.accepted)
        {
            return Err(209);
        }
        // A later reply from this single-threaded service establishes that its
        // cancelled request has released the loan. Cancellation alone cannot.
        request(read, [block::header(Op::Info), 1, 0, 2], &[], Status::Ok)?;
    } else {
        let info = request(read, [block::header(Op::Info), 1, 0, 1], &[], Status::Ok)?;
        if info & !block::INFO_READ_ONLY < 4 {
            return Err(210);
        }
        request(read, [block::header(Op::Info), 2, 0, 2], &[], Status::Stale)?;
        request(read, [0, 1, 0, 3], &[], Status::Malformed)?;
        request(
            read,
            [block::header(Op::Read), 1, u64::MAX, 4],
            &[output],
            Status::Range,
        )?;
        request(
            read,
            [block::header(Op::Read), 1, 0, 5],
            &[input],
            Status::Denied,
        )?;
        request(
            read,
            [block::header(Op::Write), 1, 2, 6],
            &[input],
            Status::Denied,
        )?;
        request(
            read,
            [block::header(Op::Flush), 1, 0, 7],
            &[],
            Status::Denied,
        )?;
        request(
            read,
            [block::header(Op::Stop), 1, 0, 8],
            &[],
            Status::Denied,
        )?;
        for id in 10..42 {
            request(
                read,
                [block::header(Op::Read), 1, 0, id],
                &[input, input],
                Status::Malformed,
            )?;
            sys::send_caps_until(
                read,
                [block::header(Op::Info), 1, 0, id],
                &[input, input],
                deadline()?,
            )
            .map_err(|_| 211u64)?;
        }
        if request(
            read,
            [block::header(Op::Read), 1, 0, 42],
            &[output],
            Status::Ok,
        )? != BLOCK as u64
        {
            return Err(212);
        }
        for (index, byte) in b"CRYOLITE-BLOCK-PROBE-V1".iter().enumerate() {
            if get(index) != *byte {
                return Err(213);
            }
        }
        if mode == 1 {
            let write = startup.grant(role::WRITE_GATE).unwrap().handle;
            for index in 0..BLOCK {
                put(index, pattern(index));
            }
            let readonly = info & block::INFO_READ_ONLY != 0;
            let status = if readonly {
                Status::ReadOnly
            } else {
                Status::Ok
            };
            request(
                write,
                [block::header(Op::Write), 1, 2, 43],
                &[input],
                status,
            )?;
            request(write, [block::header(Op::Flush), 1, 0, 44], &[], status)?;
            request(
                write,
                [block::header(Op::Stop), 1, 0, 45],
                &[],
                Status::Denied,
            )?;
            if !readonly {
                for index in 0..BLOCK {
                    put(index, 0);
                }
                request(
                    read,
                    [block::header(Op::Read), 1, 2, 46],
                    &[output],
                    Status::Ok,
                )?;
                for index in 0..BLOCK {
                    if get(index) != pattern(index) {
                        return Err(214);
                    }
                }
            }
        }
    }
    unsafe { sys::memory_unmap(VIEW) }.map_err(|_| 215u64)?;
    sys::close(memory).map_err(|_| 216u64)?;
    sys::debug_log(debug, 93, mode).map_err(|_| 217u64)?;
    Ok(())
}

fn request(
    endpoint: Handle,
    message: [u64; 4],
    grants: &[Grant],
    status: Status,
) -> Result<u64, u64> {
    let result = if grants.is_empty() {
        sys::call_until(endpoint, message, deadline()?)
    } else {
        sys::call_caps_until(endpoint, message, grants, deadline()?)
    };
    let reply = result.map_err(|_| 218u64)?;
    if reply[0] != message[3] || reply[1] != status as u64 || reply[3] != 1 {
        return Err(219);
    }
    Ok(reply[2])
}

fn deadline() -> Result<u64, u64> {
    sys::clock()
        .map_err(|_| 220u64)?
        .checked_add(2_000_000_000)
        .ok_or(221)
}
fn pattern(index: usize) -> u8 {
    (index as u8).wrapping_mul(17).wrapping_add(0x53)
}
fn get(index: usize) -> u8 {
    unsafe { ((VIEW as *const u8).add(index)).read_volatile() }
}
fn put(index: usize, byte: u8) {
    unsafe { ((VIEW as *mut u8).add(index)).write_volatile(byte) }
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    sys::exit(255)
}
