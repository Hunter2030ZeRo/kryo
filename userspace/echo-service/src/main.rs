#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_libsys::{self as sys, Grant, Handle, ObjectKind};
use kryo_services::{Manifest, RoleRule, ServiceId, protocol, role};

const SERVICE: u64 = 10;
const DIAGNOSTIC_SERVICE: u64 = 11;
const WAIT_NS: u64 = 1_000_000_000;
const POLL_NS: u64 = 100_000_000;
const ROLES: [RoleRule; 4] = [
    RoleRule {
        role: role::REGISTRATION,
        kind: ObjectKind::Endpoint,
        rights: sys::SEND,
        required: true,
    },
    RoleRule {
        role: role::CHECKPOINT,
        kind: ObjectKind::Endpoint,
        rights: sys::SEND,
        required: true,
    },
    RoleRule {
        role: role::FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights: sys::CREATE,
        required: true,
    },
    RoleRule {
        role: role::DEBUG,
        kind: ObjectKind::DebugConsole,
        rights: sys::WRITE,
        required: true,
    },
];

global_asm!(
    r#"
    .section .text.start,"ax"
    .global _start
    .type _start,@function
_start:
    cld
    xor ebp, ebp
    call {main}
    ud2
    .size _start, .-_start
"#,
    main = sym service_main,
);

#[repr(u64)]
enum Failure {
    Startup = 200,
    Authority,
    Ipc,
    Clock,
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
    let diagnostic = startup.service == DIAGNOSTIC_SERVICE && startup.argument == 1;
    if !diagnostic && (startup.service != SERVICE || startup.argument != 0) {
        return Err(Failure::Startup);
    }
    Manifest {
        service: ServiceId::new(startup.service).map_err(|_| Failure::Startup)?,
        roles: &ROLES,
    }
    .validate(&startup)
    .map_err(|_| Failure::Startup)?;
    let registration = startup
        .grant(role::REGISTRATION)
        .ok_or(Failure::Startup)?
        .handle;
    let checkpoint = startup
        .grant(role::CHECKPOINT)
        .ok_or(Failure::Startup)?
        .handle;
    let factory = startup.grant(role::FACTORY).ok_or(Failure::Startup)?.handle;
    let debug = startup.grant(role::DEBUG).ok_or(Failure::Startup)?.handle;
    let instance = startup.instance;
    sys::close(record).map_err(|_| Failure::Cleanup)?;
    check_authority(registration, factory)?;
    let endpoint = sys::create_endpoint(factory).map_err(|_| Failure::Authority)?;
    if diagnostic {
        sys::call_caps_until(
            registration,
            [
                protocol::header(protocol::REGISTER),
                DIAGNOSTIC_SERVICE,
                instance,
                0,
            ],
            &[Grant {
                handle: endpoint,
                rights: sys::SEND | sys::TRANSFER,
            }],
            deadline(WAIT_NS)?,
        )
        .map_err(|_| Failure::Ipc)?;
        return Err(Failure::Authority);
    }
    sys::send_caps_until(
        registration,
        [protocol::header(protocol::REGISTER), SERVICE, instance, 0],
        &[Grant {
            handle: endpoint,
            rights: sys::SEND | sys::TRANSFER,
        }],
        deadline(WAIT_NS)?,
    )
    .map_err(|_| Failure::Ipc)?;
    sys::close(registration).map_err(|_| Failure::Cleanup)?;
    sys::close(factory).map_err(|_| Failure::Cleanup)?;
    sys::debug_log(debug, 7, instance).map_err(|_| Failure::Ipc)?;
    sys::close(debug).map_err(|_| Failure::Cleanup)?;
    let mut late_reply_used = false;
    loop {
        let request = match sys::receive_until(endpoint, 0, deadline(POLL_NS)?) {
            Ok(request) => request,
            Err(failure) if failure.error == sys::Error::Kernel(-10) => continue,
            Err(_) => return Err(Failure::Ipc),
        };
        let Some(reply) = request.reply else { continue };
        let operation = protocol::opcode(request.message);
        let valid = matches!(operation, Ok(protocol::ECHO | protocol::ECHO_LATE_REPLY))
            && request.message[1] == instance
            && request.message[3] == 0;
        let late = valid && operation == Ok(protocol::ECHO_LATE_REPLY);
        let ready = if late && !late_reply_used {
            late_reply_used = true;
            let acknowledged = sys::call_until(
                checkpoint,
                [protocol::header(protocol::LATE_ACK), SERVICE, instance, 0],
                deadline(WAIT_NS)?,
            );
            acknowledged
                == Ok([
                    protocol::header(protocol::LATE_RESUME),
                    SERVICE,
                    instance,
                    0,
                ])
        } else {
            !late
        };
        let response = if valid && ready {
            match request.message[2].checked_add(1) {
                Some(value) => [protocol::header(protocol::ECHO_REPLY), instance, value, 0],
                None => [protocol::header(protocol::DENIED), SERVICE, instance, 0],
            }
        } else {
            [protocol::header(protocol::DENIED), SERVICE, instance, 0]
        };
        match sys::reply(endpoint, reply, response) {
            Ok(()) | Err(sys::Error::Kernel(-9)) => {}
            Err(_) => return Err(Failure::Ipc),
        }
    }
}

fn check_authority(registration: Handle, factory: Handle) -> Result<(), Failure> {
    if sys::duplicate(factory, sys::CREATE) != Err(sys::Error::Kernel(-3)) {
        return Err(Failure::Authority);
    }
    let plan = sys::manifest_create(SERVICE, 1, 0).map_err(|_| Failure::Authority)?;
    let denied = sys::manifest_grant(
        plan,
        role::REGISTRATION,
        Grant {
            handle: registration,
            rights: sys::SEND,
        },
        ObjectKind::Endpoint,
    );
    sys::close(plan).map_err(|_| Failure::Cleanup)?;
    if denied != Err(sys::Error::Kernel(-3)) {
        return Err(Failure::Authority);
    }
    Ok(())
}

fn deadline(delay: u64) -> Result<u64, Failure> {
    sys::clock()
        .map_err(|_| Failure::Clock)?
        .checked_add(delay)
        .ok_or(Failure::Clock)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    sys::exit(255)
}
