#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_libsys::{self as sys, Grant, Handle, ObjectKind};
use kryo_services::{Manifest, RoleRule, ServiceId, protocol, role};

mod budgets;

const ECHO_SERVICE: u64 = 10;
const ALLOWED: u64 = 20;
const DENIED: u64 = 21;
const WAIT_NS: u64 = 1_000_000_000;
const ROLES: [RoleRule; 4] = [
    RoleRule {
        role: role::DIRECTORY,
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
    main = sym client_main,
);

#[repr(u64)]
enum Failure {
    Startup = 200,
    Authority,
    Protocol,
    Ipc,
    Clock,
    Quota,
    Cleanup,
    BudgetHandles,
    BudgetMemory,
    BudgetThreads,
    BudgetPolicy,
}

extern "C" fn client_main(raw: u64) -> ! {
    let record = Handle::from_raw(raw).unwrap_or_else(|| sys::exit(Failure::Startup as u64));
    match run(record) {
        Ok(()) => sys::exit(0),
        Err(error) => sys::exit(error as u64),
    }
}

fn run(record: Handle) -> Result<(), Failure> {
    let startup = sys::startup(record).map_err(|_| Failure::Startup)?;
    if startup.service != ALLOWED && startup.service != DENIED {
        return Err(Failure::Startup);
    }
    Manifest {
        service: ServiceId::new(startup.service).map_err(|_| Failure::Startup)?,
        roles: &ROLES,
    }
    .validate(&startup)
    .map_err(|_| Failure::Startup)?;
    let gateway = startup
        .grant(role::DIRECTORY)
        .ok_or(Failure::Startup)?
        .handle;
    let checkpoint = startup
        .grant(role::CHECKPOINT)
        .ok_or(Failure::Startup)?
        .handle;
    let factory = startup.grant(role::FACTORY).ok_or(Failure::Startup)?.handle;
    let debug = startup.grant(role::DEBUG).ok_or(Failure::Startup)?.handle;
    sys::close(record).map_err(|_| Failure::Cleanup)?;
    budgets::check(factory, debug, startup.service)?;
    let callback = sys::create_endpoint(factory).map_err(|_| Failure::Authority)?;
    if startup.service == ALLOWED {
        allowed(gateway, checkpoint, callback)?;
    } else {
        denied(gateway, checkpoint, callback, factory)?;
    }
    sys::debug_log(debug, 8, startup.service).map_err(|_| Failure::Ipc)?;
    for handle in [callback, gateway, checkpoint, factory, debug] {
        sys::close(handle).map_err(|_| Failure::Cleanup)?;
    }
    Ok(())
}

fn allowed(gateway: Handle, checkpoint: Handle, callback: Handle) -> Result<(), Failure> {
    let denied = query(
        gateway,
        callback,
        [
            protocol::header(protocol::LOOKUP),
            ALLOWED,
            ECHO_SERVICE,
            sys::SEND | sys::CONTROL,
        ],
    )?;
    expect_denied(denied)?;
    expect_denied(query(
        gateway,
        callback,
        [
            protocol::header(protocol::RESTART),
            ALLOWED,
            ECHO_SERVICE,
            1,
        ],
    )?)?;
    let first = lookup(gateway, callback, 1)?;
    echo(first, 1)?;
    late_reply(first, 1)?;
    echo(first, 1)?;
    check_reduced(first, callback)?;
    let restarted = query(
        checkpoint,
        callback,
        [
            protocol::header(protocol::RESTART),
            ALLOWED,
            ECHO_SERVICE,
            1,
        ],
    )?;
    if restarted.message != [protocol::header(protocol::RESTARTED), ECHO_SERVICE, 2, 0]
        || restarted.handles != [None; 2]
        || restarted.reply.is_some()
    {
        return Err(Failure::Protocol);
    }
    let failure = sys::call_until(
        first,
        [protocol::header(protocol::ECHO), 1, 41, 0],
        deadline()?,
    )
    .err()
    .ok_or(Failure::Authority)?;
    if failure.error != sys::Error::Kernel(-7) || failure.accepted {
        return Err(Failure::Authority);
    }
    let second = lookup(gateway, callback, 2)?;
    if first == second {
        return Err(Failure::Authority);
    }
    echo(second, 2)?;
    sys::close(first).map_err(|_| Failure::Cleanup)?;
    sys::close(second).map_err(|_| Failure::Cleanup)?;
    Ok(())
}

fn denied(
    gateway: Handle,
    checkpoint: Handle,
    callback: Handle,
    factory: Handle,
) -> Result<(), Failure> {
    let mut held = [None; 7];
    for (index, slot) in held.iter_mut().enumerate() {
        *slot = Some(if index % 2 == 0 {
            sys::create_endpoint(factory).map_err(|_| Failure::Quota)?
        } else {
            sys::create_notification(factory).map_err(|_| Failure::Quota)?
        });
    }
    if sys::create_endpoint(factory) != Err(sys::Error::Kernel(-5))
        || sys::create_notification(factory) != Err(sys::Error::Kernel(-5))
    {
        return Err(Failure::Quota);
    }
    let released = query(
        checkpoint,
        callback,
        [protocol::header(protocol::QUOTA_HELD), DENIED, 8, 0],
    )?;
    if released.message != [protocol::header(protocol::QUOTA_RELEASE), DENIED, 0, 0]
        || released.handles != [None; 2]
        || released.reply.is_some()
    {
        return Err(Failure::Protocol);
    }
    for handle in held.into_iter().flatten() {
        sys::close(handle).map_err(|_| Failure::Cleanup)?;
    }
    let reused = sys::create_notification(factory).map_err(|_| Failure::Quota)?;
    sys::close(reused).map_err(|_| Failure::Cleanup)?;
    sys::send_until(
        gateway,
        [
            protocol::header(protocol::LOOKUP),
            ALLOWED,
            ECHO_SERVICE,
            sys::SEND,
        ],
        deadline()?,
    )
    .map_err(|_| Failure::Ipc)?;
    sys::send_caps_until(
        gateway,
        [
            protocol::header(protocol::LOOKUP),
            ALLOWED,
            ECHO_SERVICE,
            sys::SEND,
        ],
        &[Grant {
            handle: callback,
            rights: sys::SEND | sys::TRANSFER,
        }],
        deadline()?,
    )
    .map_err(|_| Failure::Ipc)?;
    let invalid = sys::receive_until(
        callback,
        1,
        sys::clock()
            .map_err(|_| Failure::Clock)?
            .checked_add(2_000_000)
            .ok_or(Failure::Clock)?,
    )
    .err()
    .ok_or(Failure::Authority)?;
    if invalid.error != sys::Error::Kernel(-10) || invalid.accepted {
        return Err(Failure::Authority);
    }
    expect_denied(query(
        gateway,
        callback,
        [
            protocol::header(protocol::LOOKUP) ^ (1 << 32),
            ALLOWED,
            ECHO_SERVICE,
            sys::SEND,
        ],
    )?)?;
    // The claimed ID is deliberately the authorized client's ID. Admission is
    // bound to this process's private gateway, rather than these message words.
    expect_denied(query(
        gateway,
        callback,
        [
            protocol::header(protocol::LOOKUP),
            ALLOWED,
            ECHO_SERVICE,
            sys::SEND,
        ],
    )?)?;
    expect_denied(query(
        gateway,
        callback,
        [protocol::header(protocol::REGISTER), ECHO_SERVICE, 1, 0],
    )?)?;
    Ok(())
}

fn lookup(gateway: Handle, callback: Handle, instance: u64) -> Result<Handle, Failure> {
    let received = query(
        gateway,
        callback,
        [
            protocol::header(protocol::LOOKUP),
            ALLOWED,
            ECHO_SERVICE,
            sys::SEND,
        ],
    )?;
    if received.message != [protocol::header(protocol::FOUND), ECHO_SERVICE, instance, 0]
        || received.reply.is_some()
        || received.handles[1].is_some()
    {
        return Err(Failure::Protocol);
    }
    let endpoint = received.handles[0].ok_or(Failure::Authority)?;
    let info = sys::handle_info(endpoint).map_err(|_| Failure::Authority)?;
    if info.kind != ObjectKind::Endpoint || info.rights != sys::SEND {
        return Err(Failure::Authority);
    }
    Ok(endpoint)
}

fn query(
    gateway: Handle,
    callback: Handle,
    message: sys::Message,
) -> Result<sys::Received, Failure> {
    sys::send_caps_until(
        gateway,
        message,
        &[Grant {
            handle: callback,
            rights: sys::SEND,
        }],
        deadline()?,
    )
    .map_err(|_| Failure::Ipc)?;
    sys::receive_until(callback, 1, deadline()?).map_err(|_| Failure::Ipc)
}

fn expect_denied(received: sys::Received) -> Result<(), Failure> {
    if protocol::opcode(received.message) != Ok(protocol::DENIED)
        || received.handles != [None; 2]
        || received.reply.is_some()
    {
        return Err(Failure::Authority);
    }
    Ok(())
}

fn echo(endpoint: Handle, instance: u64) -> Result<(), Failure> {
    let response = sys::call_until(
        endpoint,
        [protocol::header(protocol::ECHO), instance, 41, 0],
        deadline()?,
    )
    .map_err(|_| Failure::Ipc)?;
    if response != [protocol::header(protocol::ECHO_REPLY), instance, 42, 0] {
        return Err(Failure::Protocol);
    }
    Ok(())
}

fn late_reply(endpoint: Handle, instance: u64) -> Result<(), Failure> {
    let failure = sys::call_until(
        endpoint,
        [protocol::header(protocol::ECHO_LATE_REPLY), instance, 41, 0],
        deadline()?,
    )
    .err()
    .ok_or(Failure::Protocol)?;
    if failure.error != sys::Error::Kernel(-8) || !failure.accepted {
        return Err(Failure::Protocol);
    }
    Ok(())
}

fn check_reduced(endpoint: Handle, callback: Handle) -> Result<(), Failure> {
    if sys::close_endpoint(endpoint) != Err(sys::Error::Kernel(-3)) {
        return Err(Failure::Authority);
    }
    let denied = sys::receive_until(endpoint, 0, deadline()?)
        .err()
        .ok_or(Failure::Authority)?;
    if denied.error != sys::Error::Kernel(-3) || denied.accepted {
        return Err(Failure::Authority);
    }
    let denied = sys::send_caps_until(
        callback,
        [0; 4],
        &[Grant {
            handle: endpoint,
            rights: sys::SEND,
        }],
        deadline()?,
    )
    .err()
    .ok_or(Failure::Authority)?;
    if denied.error != sys::Error::Kernel(-3) || denied.accepted {
        return Err(Failure::Authority);
    }
    Ok(())
}

fn deadline() -> Result<u64, Failure> {
    sys::clock()
        .map_err(|_| Failure::Clock)?
        .checked_add(WAIT_NS)
        .ok_or(Failure::Clock)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    sys::exit(255)
}
