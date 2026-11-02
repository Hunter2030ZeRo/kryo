#![no_std]
#![no_main]

mod transport;

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_interfaces::block::{self, Gate, Operation, Request, Status, role};
use kryo_libsys::{self as sys, Handle, ObjectKind};
use kryo_services::{Manifest, RoleRule, ServiceId};
use transport::{Device, IoFailure};

const LOAN: u64 = 0x0300_0000;
const POLL_NS: u64 = 1_000_000;
const ROLES: [RoleRule; 5] = [
    RoleRule {
        role: role::DEVICE,
        kind: ObjectKind::Device,
        rights: sys::CONTROL | sys::READ | sys::WRITE,
        required: true,
    },
    RoleRule {
        role: role::READ_GATE,
        kind: ObjectKind::Endpoint,
        rights: sys::RECEIVE,
        required: true,
    },
    RoleRule {
        role: role::WRITE_GATE,
        kind: ObjectKind::Endpoint,
        rights: sys::RECEIVE,
        required: true,
    },
    RoleRule {
        role: role::CONTROL_GATE,
        kind: ObjectKind::Endpoint,
        rights: sys::RECEIVE,
        required: true,
    },
    RoleRule {
        role: role::DEBUG,
        kind: ObjectKind::DebugConsole,
        rights: sys::WRITE,
        required: true,
    },
];

global_asm!(r#"
    .section .text.start,"ax"
    .global _start
    .type _start,@function
_start:
    cld
    xor ebp, ebp
    call {main}
    ud2
    .size _start, .-_start
"#, main = sym service_main);

#[derive(Clone, Copy)]
#[repr(u64)]
enum Failure {
    Startup = 200,
    Transport,
    Ipc,
    Cleanup,
}

extern "C" fn service_main(raw: u64) -> ! {
    let record = Handle::from_raw(raw).unwrap_or_else(|| sys::exit(Failure::Startup as u64));
    match serve(record) {
        Ok(()) => sys::exit(0),
        Err(error) => sys::exit(error as u64),
    }
}

fn serve(record: Handle) -> Result<(), Failure> {
    let startup = sys::startup(record).map_err(|_| Failure::Startup)?;
    if startup.service != block::SERVICE || startup.argument > 1 {
        return Err(Failure::Startup);
    }
    Manifest {
        service: ServiceId::new(block::SERVICE).map_err(|_| Failure::Startup)?,
        roles: &ROLES,
    }
    .validate(&startup)
    .map_err(|_| Failure::Startup)?;
    let grant = |role| {
        startup
            .grant(role)
            .map(|entry| entry.handle)
            .ok_or(Failure::Startup)
    };
    let hardware = grant(role::DEVICE)?;
    let debug = grant(role::DEBUG)?;
    let gates = [
        (grant(role::READ_GATE)?, Gate::Read),
        (grant(role::WRITE_GATE)?, Gate::Write),
        (grant(role::CONTROL_GATE)?, Gate::Control),
    ];
    sys::close(record).map_err(|_| Failure::Cleanup)?;
    let mut device = Device::open(hardware).map_err(|_| Failure::Transport)?;
    sys::debug_log(debug, 91, startup.instance).map_err(|_| Failure::Ipc)?;
    loop {
        for &(endpoint, gate) in &gates {
            let incoming = match sys::receive_for(endpoint, 2, POLL_NS) {
                Ok(incoming) => incoming,
                Err(error) if error.error == sys::Error::Kernel(-10) => continue,
                Err(_) => return Err(Failure::Ipc),
            };
            // Keep every delivered grant reachable before checking any field.
            // Rejected and fire-and-forget requests release the same full set.
            let handles = incoming.handles;
            let Some(reply) = incoming.reply else {
                close_all(handles)?;
                continue;
            };
            let count = handles.iter().flatten().count();
            let request = Request::decode(
                incoming.message,
                gate,
                startup.instance,
                device.blocks(),
                device.read_only(),
                count,
                startup.argument == 1,
            );
            let mut stop = false;
            let mut fatal = false;
            let (status, value) = match request {
                Err(status) => (status, 0),
                Ok(request) => match execute(&mut device, request, handles[0]) {
                    Ok(value) => {
                        stop = request.operation == Operation::Stop;
                        (Status::Ok, value)
                    }
                    Err(error) => {
                        fatal = error.fatal;
                        (error.status, 0)
                    }
                },
            };
            // execute retires its CPU loan before these handles and the reply.
            close_all(handles)?;
            match sys::reply(
                endpoint,
                reply,
                block::response(incoming.message[3], status, value, startup.instance),
            ) {
                Ok(()) => {}
                Err(sys::Error::Kernel(-9)) => {
                    sys::debug_log(debug, 92, startup.instance).map_err(|_| Failure::Ipc)?;
                }
                Err(sys::Error::Kernel(-7)) => {}
                Err(_) => return Err(Failure::Ipc),
            }
            if fatal {
                return Err(Failure::Transport);
            }
            if stop {
                device.close().map_err(|_| Failure::Transport)?;
                for &(endpoint, _) in &gates {
                    sys::close(endpoint).map_err(|_| Failure::Cleanup)?;
                }
                sys::close(debug).map_err(|_| Failure::Cleanup)?;
                return Ok(());
            }
        }
    }
}

fn execute(
    device: &mut Device,
    request: Request,
    memory: Option<Handle>,
) -> Result<u64, IoFailure> {
    match request.operation {
        Operation::Info => Ok(device.blocks()
            | if device.read_only() {
                block::INFO_READ_ONLY
            } else {
                0
            }),
        Operation::Stop => Ok(0),
        Operation::Flush => device.flush().map(|()| 0),
        Operation::Read | Operation::Write => {
            let memory = memory.ok_or(IoFailure::ordinary(Status::Malformed))?;
            let info =
                sys::handle_info(memory).map_err(|_| IoFailure::ordinary(Status::Malformed))?;
            if info.kind != ObjectKind::Memory {
                return Err(IoFailure::ordinary(Status::Malformed));
            }
            let output = request.operation == Operation::Read;
            let rights = sys::READ | sys::MAP | if output { sys::WRITE } else { 0 };
            if info.rights & rights != rights {
                return Err(IoFailure::ordinary(Status::Denied));
            }
            // The fixed loan address is private to this single-threaded service.
            // Raw accesses avoid creating references to client-mutable storage.
            unsafe { sys::memory_map(memory, LOAN, 0, output) }.map_err(|error| {
                IoFailure::ordinary(if error == sys::Error::Kernel(-5) {
                    Status::Unavailable
                } else {
                    Status::Malformed
                })
            })?;
            let result = if output {
                device.read(request.block).map(|()| device.copy_to(LOAN))
            } else {
                device.copy_from(LOAN);
                device.write(request.block)
            };
            unsafe { sys::memory_unmap(LOAN) }.map_err(|_| IoFailure {
                status: Status::Unavailable,
                fatal: true,
            })?;
            result.map(|()| block::BLOCK_SIZE as u64)
        }
    }
}

fn close_all(handles: [Option<Handle>; 2]) -> Result<(), Failure> {
    let mut failed = false;
    for handle in handles.into_iter().flatten() {
        failed |= sys::close(handle).is_err();
    }
    if failed {
        Err(Failure::Cleanup)
    } else {
        Ok(())
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    sys::exit(255)
}
