#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_libsys::{self as sys, Grant, Handle, ObjectKind, StartupGrant};
use kryo_services::State;
use kryo_services::{
    Access, Gateway, Instance, Manifest, Record, Registry, RoleRule, ServiceId, protocol, role,
};

const ROOT: u64 = 1;
const SERVER: u64 = 10;
const DIAGNOSTIC_SERVICE: u64 = 11;
const ALLOWED: u64 = 20;
const DENIED: u64 = 21;
const WAIT_NS: u64 = 1_000_000_000;
const POLL_NS: u64 = 1_000_000;
const CHECK_NS: u64 = 5_000_000;
const RETRY_NS: u64 = 100_000_000;
const ROOT_ROLES: [RoleRule; 7] = [
    RoleRule {
        role: role::FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights: sys::CREATE,
        required: true,
    },
    RoleRule {
        role: role::DEBUG,
        kind: ObjectKind::DebugConsole,
        rights: sys::WRITE | sys::TRANSFER,
        required: true,
    },
    RoleRule {
        role: role::PEER_IMAGE,
        kind: ObjectKind::Image,
        rights: sys::EXECUTE,
        required: true,
    },
    RoleRule {
        role: role::CLIENT_IMAGE,
        kind: ObjectKind::Image,
        rights: sys::EXECUTE,
        required: true,
    },
    RoleRule {
        role: role::SERVER_FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights: sys::CREATE | sys::TRANSFER,
        required: true,
    },
    RoleRule {
        role: role::ALLOWED_FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights: sys::CREATE | sys::TRANSFER,
        required: true,
    },
    RoleRule {
        role: role::DENIED_FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights: sys::CREATE | sys::TRANSFER,
        required: true,
    },
];
const SERVER_ROLES: [RoleRule; 4] = [
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
const CLIENT_ROLES: [RoleRule; 4] = [
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
    main = sym supervisor_main,
);

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
enum Failure {
    Startup = 200,
    Authority,
    Policy,
    Protocol,
    Ipc,
    Clock,
    Process,
    Cleanup,
    Stalled,
}

struct Authority {
    factory: Handle,
    debug: Handle,
    server_image: Handle,
    client_image: Handle,
    server_factory: Handle,
    allowed_factory: Handle,
    denied_factory: Handle,
}

struct Server {
    instance: Instance,
    process: Handle,
    checkpoint: Option<Handle>,
}
struct Client {
    endpoint: Handle,
    checkpoint: Handle,
    gateway: Gateway,
    process: Handle,
    finished: bool,
}

extern "C" fn supervisor_main(raw: u64) -> ! {
    let record = Handle::from_raw(raw).unwrap_or_else(|| sys::exit(Failure::Startup as u64));
    match run(record) {
        Ok(()) => sys::exit(0),
        Err(error) => sys::exit(error as u64),
    }
}

fn run(record: Handle) -> Result<(), Failure> {
    let startup = sys::startup(record).map_err(|_| Failure::Startup)?;
    Manifest {
        service: id(ROOT)?,
        roles: &ROOT_ROLES,
    }
    .validate(&startup)
    .map_err(|_| Failure::Startup)?;
    if startup.argument > 1 {
        return Err(Failure::Startup);
    }
    let authority = Authority {
        factory: grant(&startup, role::FACTORY)?,
        debug: grant(&startup, role::DEBUG)?,
        server_image: grant(&startup, role::PEER_IMAGE)?,
        client_image: grant(&startup, role::CLIENT_IMAGE)?,
        server_factory: grant(&startup, role::SERVER_FACTORY)?,
        allowed_factory: grant(&startup, role::ALLOWED_FACTORY)?,
        denied_factory: grant(&startup, role::DENIED_FACTORY)?,
    };
    sys::close(record).map_err(|_| Failure::Cleanup)?;
    let mut records = [
        Record::new(id(SERVER)?),
        Record::new(id(ALLOWED)?),
        Record::new(id(DENIED)?),
    ];
    let access = [Access {
        client: id(ALLOWED)?,
        target: id(SERVER)?,
        rights: sys::SEND,
    }];
    let mut registry = Registry::new(&mut records, &access).map_err(|_| Failure::Policy)?;
    reject_malformed_registration(&authority)?;
    let mut server = start_server(&authority, &mut registry)?;
    let mut allowed = start_client(&authority, &registry, ALLOWED, authority.allowed_factory)?;
    let mut denied = start_client(&authority, &registry, DENIED, authority.denied_factory)?;
    exercise(
        &authority,
        &mut registry,
        &mut server,
        &mut allowed,
        &mut denied,
    )?;
    close_server_checkpoint(&mut server)?;
    close_client(&mut allowed)?;
    close_client(&mut denied)?;
    if startup.argument == 1 {
        stop_server(&mut registry, server)?;
        for handle in [
            authority.factory,
            authority.server_image,
            authority.client_image,
            authority.server_factory,
            authority.allowed_factory,
            authority.denied_factory,
        ] {
            sys::close(handle).map_err(|_| Failure::Cleanup)?;
        }
        sys::debug_log(authority.debug, 6, 2).map_err(|_| Failure::Ipc)?;
        sys::close(authority.debug).map_err(|_| Failure::Cleanup)?;
        return Ok(());
    }
    if startup.argument != 0 {
        return Err(Failure::Startup);
    }
    sys::debug_log(authority.debug, 6, 2).map_err(|_| Failure::Ipc)?;
    monitor(&authority, &mut registry, server)
}

fn monitor(
    authority: &Authority,
    registry: &mut Registry<'_, '_>,
    initial: Server,
) -> Result<(), Failure> {
    let mut current = Some(initial);
    let mut last_failure = None;
    let mut observed = false;
    loop {
        if let Some(server) = current.as_ref() {
            sys::sleep_for(CHECK_NS).map_err(|_| Failure::Clock)?;
            if exited(server.process)? {
                let ended = current.take().ok_or(Failure::Policy)?;
                stop_server(registry, ended)?;
            } else if !observed {
                sys::debug_log(authority.debug, 7, server.instance.generation)
                    .map_err(|_| Failure::Ipc)?;
                observed = true;
            }
        }
        if current.is_none() {
            match start_server(authority, registry) {
                Ok(mut server) => {
                    close_server_checkpoint(&mut server)?;
                    sys::debug_log(authority.debug, 9, server.instance.generation)
                        .map_err(|_| Failure::Ipc)?;
                    current = Some(server);
                    last_failure = None;
                }
                Err(error @ (Failure::Clock | Failure::Cleanup)) => return Err(error),
                Err(error) => {
                    if last_failure != Some(error) {
                        sys::debug_log(authority.debug, 10, error as u64)
                            .map_err(|_| Failure::Ipc)?;
                        last_failure = Some(error);
                    }
                    // Retained client capabilities keep their original account
                    // charged. Backoff never replaces that account or its epoch.
                    sys::sleep_for(RETRY_NS).map_err(|_| Failure::Clock)?;
                }
            }
        }
    }
}

fn exercise(
    authority: &Authority,
    registry: &mut Registry<'_, '_>,
    server: &mut Server,
    allowed: &mut Client,
    denied: &mut Client,
) -> Result<(), Failure> {
    let limit = deadline(5_000_000_000)?;
    let mut quota_waiter = None;
    let mut restarted = false;
    let mut late_probe_completed = false;
    while !allowed.finished || !denied.finished {
        if sys::clock().map_err(|_| Failure::Clock)? >= limit {
            return Err(Failure::Stalled);
        }
        late_checkpoint(server, allowed.process, &mut late_probe_completed)?;
        if !denied.finished {
            if let Some((request, callback)) = poll(denied.checkpoint)? {
                if !restarted
                    && request.message == [protocol::header(protocol::QUOTA_HELD), DENIED, 8, 0]
                    && quota_waiter.is_none()
                {
                    quota_waiter = Some(callback);
                } else {
                    deny(callback, 0)?;
                    sys::close(callback).map_err(|_| Failure::Cleanup)?;
                }
            }
            if let Some((request, callback)) = poll(denied.endpoint)? {
                respond_lookup(registry, &denied.gateway, request.message, callback)?;
                sys::close(callback).map_err(|_| Failure::Cleanup)?;
            }
        }
        // The authorized client must run while the other account remains full.
        if !allowed.finished && (quota_waiter.is_some() || restarted) {
            if let Some((request, callback)) = poll(allowed.endpoint)? {
                respond_lookup(registry, &allowed.gateway, request.message, callback)?;
                sys::close(callback).map_err(|_| Failure::Cleanup)?;
            }
            if let Some((request, callback)) = poll(allowed.checkpoint)? {
                if !restarted
                    && request.message == [protocol::header(protocol::RESTART), ALLOWED, SERVER, 1]
                {
                    // The separately granted bootstrap checkpoint admits this
                    // one transition; lookup gateways never grant service control.
                    *server = restart(authority, registry, server)?;
                    restarted = true;
                    let quota = quota_waiter.take().ok_or(Failure::Protocol)?;
                    sys::send_until(
                        quota,
                        [protocol::header(protocol::QUOTA_RELEASE), DENIED, 0, 0],
                        deadline(WAIT_NS)?,
                    )
                    .map_err(|_| Failure::Ipc)?;
                    sys::close(quota).map_err(|_| Failure::Cleanup)?;
                    sys::send_until(
                        callback,
                        [
                            protocol::header(protocol::RESTARTED),
                            SERVER,
                            server.instance.generation,
                            0,
                        ],
                        deadline(WAIT_NS)?,
                    )
                    .map_err(|_| Failure::Ipc)?;
                } else {
                    deny(callback, 0)?;
                }
                sys::close(callback).map_err(|_| Failure::Cleanup)?;
            }
        }
        allowed.finished = exited(allowed.process)?;
        denied.finished = exited(denied.process)?;
    }
    if !restarted
        || !late_probe_completed
        || server.instance.generation != 2
        || quota_waiter.is_some()
    {
        return Err(Failure::Protocol);
    }
    for process in [allowed.process, denied.process] {
        expect_exit(process, false)?;
    }
    Ok(())
}

fn respond_lookup(
    registry: &Registry<'_, '_>,
    gateway: &Gateway,
    message: sys::Message,
    callback: Handle,
) -> Result<(), Failure> {
    let binding = if protocol::opcode(message) == Ok(protocol::LOOKUP) {
        ServiceId::new(message[2])
            .ok()
            .and_then(|target| registry.lookup(gateway, target, message[3]).ok())
    } else {
        None
    };
    if let Some(binding) = binding {
        delivery(sys::send_caps_until(
            callback,
            [
                protocol::header(protocol::FOUND),
                binding.instance.service.get(),
                binding.instance.generation,
                0,
            ],
            &[Grant {
                handle: binding.endpoint,
                rights: binding.rights,
            }],
            deadline(WAIT_NS)?,
        ))
    } else {
        deny(callback, message[2])
    }
}

fn deny(callback: Handle, target: u64) -> Result<(), Failure> {
    delivery(sys::send_until(
        callback,
        [protocol::header(protocol::DENIED), target, 0, 0],
        deadline(WAIT_NS)?,
    ))
}

fn delivery(result: Result<(), sys::IpcError>) -> Result<(), Failure> {
    match result {
        Ok(()) => Ok(()),
        Err(failure) if matches!(failure.error, sys::Error::Kernel(-7 | -8 | -10)) => Ok(()),
        Err(_) => Err(Failure::Ipc),
    }
}

fn start_server(authority: &Authority, registry: &mut Registry<'_, '_>) -> Result<Server, Failure> {
    start_server_for(authority, registry, SERVER, 0)
}

fn start_server_for(
    authority: &Authority,
    registry: &mut Registry<'_, '_>,
    service: u64,
    argument: u64,
) -> Result<Server, Failure> {
    let instance = registry.begin(id(service)?).map_err(|_| Failure::Policy)?;
    let gate = match sys::create_endpoint(authority.factory) {
        Ok(gate) => gate,
        Err(_) => {
            registry.failed(instance).map_err(|_| Failure::Policy)?;
            return Err(Failure::Authority);
        }
    };
    let checkpoint = match sys::create_endpoint(authority.factory) {
        Ok(checkpoint) => checkpoint,
        Err(_) => {
            close_endpoint(gate)?;
            registry.failed(instance).map_err(|_| Failure::Policy)?;
            return Err(Failure::Authority);
        }
    };
    let grants = [
        entry(role::REGISTRATION, ObjectKind::Endpoint, sys::SEND, gate),
        entry(
            role::CHECKPOINT,
            ObjectKind::Endpoint,
            sys::SEND,
            checkpoint,
        ),
        entry(
            role::FACTORY,
            ObjectKind::ChannelFactory,
            sys::CREATE,
            authority.server_factory,
        ),
        entry(
            role::DEBUG,
            ObjectKind::DebugConsole,
            sys::WRITE,
            authority.debug,
        ),
    ];
    let process = match launch(
        authority.server_image,
        instance,
        argument,
        &SERVER_ROLES,
        &grants,
    ) {
        Ok(process) => process,
        Err(error) => {
            close_endpoint(gate)?;
            close_endpoint(checkpoint)?;
            registry.failed(instance).map_err(|_| Failure::Policy)?;
            return Err(error);
        }
    };
    let mut advertisements = [None; 2];
    let registration = (|| {
        let received = sys::receive_until(gate, 2, deadline(WAIT_NS)?).map_err(|_| Failure::Ipc)?;
        // Receive has already installed these references, even if its envelope
        // is invalid. Capture every owner before inspecting protocol fields.
        advertisements = received.handles;
        if received.reply.is_some() || advertisements[1].is_some() {
            return Err(Failure::Protocol);
        }
        let endpoint = advertisements[0].ok_or(Failure::Protocol)?;
        if protocol::opcode(received.message) != Ok(protocol::REGISTER) || received.message[3] != 0
        {
            return Err(Failure::Protocol);
        }
        let claimed = Instance::new(id(received.message[1])?, received.message[2])
            .map_err(|_| Failure::Protocol)?;
        let info = sys::handle_info(endpoint).map_err(|_| Failure::Authority)?;
        registry
            .register(instance, claimed, endpoint, info)
            .map_err(|_| Failure::Policy)?;
        advertisements[0] = None;
        Ok(())
    })();
    let closed = close_endpoint(gate);
    if let Some(error) = registration.err().or(closed.err()) {
        discard(advertisements)?;
        if let Some(endpoint) = registry.failed(instance).map_err(|_| Failure::Cleanup)? {
            sys::close(endpoint).map_err(|_| Failure::Cleanup)?;
        }
        close_endpoint(checkpoint)?;
        stop_process(process).map_err(|_| Failure::Cleanup)?;
        sys::close(process).map_err(|_| Failure::Cleanup)?;
        return Err(error);
    }
    Ok(Server {
        instance,
        process,
        checkpoint: Some(checkpoint),
    })
}

fn reject_malformed_registration(authority: &Authority) -> Result<(), Failure> {
    let service = id(DIAGNOSTIC_SERVICE)?;
    let mut records = [Record::new(service)];
    let mut registry = Registry::new(&mut records, &[]).map_err(|_| Failure::Policy)?;
    if !matches!(
        start_server_for(authority, &mut registry, DIAGNOSTIC_SERVICE, 1),
        Err(Failure::Protocol)
    ) || registry.state(service).map_err(|_| Failure::Policy)? != State::Failed
    {
        return Err(Failure::Protocol);
    }
    let mut channels = [None; 8];
    for slot in &mut channels {
        *slot =
            Some(sys::create_endpoint(authority.server_factory).map_err(|_| Failure::Authority)?);
    }
    if sys::create_endpoint(authority.server_factory) != Err(sys::Error::Kernel(-5)) {
        return Err(Failure::Authority);
    }
    for channel in channels.into_iter().flatten() {
        sys::close(channel).map_err(|_| Failure::Cleanup)?;
    }
    Ok(())
}

fn late_checkpoint(server: &Server, client: Handle, completed: &mut bool) -> Result<(), Failure> {
    let checkpoint = server.checkpoint.ok_or(Failure::Policy)?;
    let received = match sys::receive_for(checkpoint, 0, POLL_NS) {
        Ok(received) => received,
        Err(failure) if failure.error == sys::Error::Kernel(-10) => return Ok(()),
        Err(_) => return Err(Failure::Ipc),
    };
    let Some(reply) = received.reply else {
        return Ok(());
    };
    if !*completed
        && server.instance.service.get() == SERVER
        && server.instance.generation == 1
        && received.message == [protocol::header(protocol::LATE_ACK), SERVER, 1, 0]
    {
        if !sys::cancel_wait(client).map_err(|_| Failure::Process)? {
            return Err(Failure::Protocol);
        }
        *completed = true;
        sys::reply(
            checkpoint,
            reply,
            [protocol::header(protocol::LATE_RESUME), SERVER, 1, 0],
        )
        .map_err(|_| Failure::Ipc)?;
    } else {
        let _ = sys::reply(
            checkpoint,
            reply,
            [
                protocol::header(protocol::DENIED),
                SERVER,
                server.instance.generation,
                0,
            ],
        );
    }
    Ok(())
}

fn start_client(
    authority: &Authority,
    registry: &Registry<'_, '_>,
    service: u64,
    factory: Handle,
) -> Result<Client, Failure> {
    let endpoint = sys::create_endpoint(authority.factory).map_err(|_| Failure::Authority)?;
    let checkpoint = match sys::create_endpoint(authority.factory) {
        Ok(checkpoint) => checkpoint,
        Err(_) => {
            close_endpoint(endpoint)?;
            return Err(Failure::Authority);
        }
    };
    let gateway = registry
        .bind_gateway(id(service)?)
        .map_err(|_| Failure::Policy)?;
    let grants = [
        entry(role::DIRECTORY, ObjectKind::Endpoint, sys::SEND, endpoint),
        entry(
            role::CHECKPOINT,
            ObjectKind::Endpoint,
            sys::SEND,
            checkpoint,
        ),
        entry(
            role::FACTORY,
            ObjectKind::ChannelFactory,
            sys::CREATE,
            factory,
        ),
        entry(
            role::DEBUG,
            ObjectKind::DebugConsole,
            sys::WRITE,
            authority.debug,
        ),
    ];
    match launch(
        authority.client_image,
        Instance::new(id(service)?, 1).map_err(|_| Failure::Policy)?,
        0,
        &CLIENT_ROLES,
        &grants,
    ) {
        Ok(process) => Ok(Client {
            endpoint,
            checkpoint,
            gateway,
            process,
            finished: false,
        }),
        Err(error) => {
            close_endpoint(endpoint)?;
            close_endpoint(checkpoint)?;
            Err(error)
        }
    }
}

fn launch(
    image: Handle,
    instance: Instance,
    argument: u64,
    rules: &[RoleRule],
    grants: &[StartupGrant],
) -> Result<Handle, Failure> {
    Manifest {
        service: instance.service,
        roles: rules,
    }
    .validate_grants(grants)
    .map_err(|_| Failure::Policy)?;
    let plan = sys::manifest_create(instance.service.get(), instance.generation, argument)
        .map_err(|_| Failure::Authority)?;
    let result = (|| {
        for grant in grants {
            sys::manifest_grant(
                plan,
                grant.role,
                Grant {
                    handle: grant.handle,
                    rights: grant.rights,
                },
                grant.kind,
            )
            .map_err(|_| Failure::Authority)?;
        }
        sys::spawn_manifest(image, plan).map_err(|_| Failure::Process)
    })();
    sys::close(plan).map_err(|_| Failure::Cleanup)?;
    result
}

fn restart(
    authority: &Authority,
    registry: &mut Registry<'_, '_>,
    server: &Server,
) -> Result<Server, Failure> {
    stop_server(
        registry,
        Server {
            instance: server.instance,
            process: server.process,
            checkpoint: server.checkpoint,
        },
    )?;
    start_server(authority, registry)
}

fn stop_server(registry: &mut Registry<'_, '_>, mut server: Server) -> Result<(), Failure> {
    if let Some(endpoint) = registry
        .stopping(server.instance)
        .map_err(|_| Failure::Policy)?
    {
        sys::close(endpoint).map_err(|_| Failure::Cleanup)?;
    }
    close_server_checkpoint(&mut server)?;
    stop_process(server.process)?;
    registry
        .stopped(server.instance)
        .map_err(|_| Failure::Policy)?;
    sys::close(server.process).map_err(|_| Failure::Cleanup)?;
    Ok(())
}

fn close_server_checkpoint(server: &mut Server) -> Result<(), Failure> {
    if let Some(checkpoint) = server.checkpoint.take() {
        close_endpoint(checkpoint)?;
    }
    Ok(())
}

fn stop_process(process: Handle) -> Result<(), Failure> {
    match sys::task_stop(process, 0) {
        Ok(()) | Err(sys::Error::Kernel(-7)) => {}
        Err(_) => return Err(Failure::Process),
    }
    expect_exit(process, true)
}

fn expect_exit(process: Handle, stopped: bool) -> Result<(), Failure> {
    let status =
        sys::process_wait_until(process, deadline(WAIT_NS)?).map_err(|_| Failure::Process)?;
    if !stopped && (status.reason != sys::ExitReason::Exited || status.code != 0) {
        return Err(Failure::Process);
    }
    Ok(())
}

fn exited(process: Handle) -> Result<bool, Failure> {
    Ok(sys::task_status(process).map_err(|_| Failure::Process)? == sys::TaskStatus::Exited as u64)
}

fn close_client(client: &mut Client) -> Result<(), Failure> {
    close_endpoint(client.endpoint)?;
    close_endpoint(client.checkpoint)?;
    sys::close(client.process).map_err(|_| Failure::Cleanup)
}

fn close_endpoint(endpoint: Handle) -> Result<(), Failure> {
    sys::close_endpoint(endpoint).map_err(|_| Failure::Cleanup)?;
    sys::close(endpoint).map_err(|_| Failure::Cleanup)
}

fn poll(endpoint: Handle) -> Result<Option<(sys::Received, Handle)>, Failure> {
    let request = match sys::receive_for(endpoint, 1, POLL_NS) {
        Ok(request) => request,
        Err(failure) if failure.error == sys::Error::Kernel(-10) => return Ok(None),
        Err(_) => return Err(Failure::Ipc),
    };
    if let Some(reply) = request.reply {
        // A malformed caller must not block the directory or acquire authority.
        let _ = sys::reply(
            endpoint,
            reply,
            [protocol::header(protocol::DENIED), 0, 0, 0],
        );
        discard(request.handles)?;
        return Ok(None);
    }
    let Some(callback) = request.handles[0] else {
        return Ok(None);
    };
    let info = sys::handle_info(callback).map_err(|_| Failure::Authority)?;
    if request.handles[1].is_some() || info.kind != ObjectKind::Endpoint || info.rights != sys::SEND
    {
        discard(request.handles)?;
        return Ok(None);
    }
    Ok(Some((request, callback)))
}

fn discard(handles: [Option<Handle>; 2]) -> Result<(), Failure> {
    for handle in handles.into_iter().flatten() {
        sys::close(handle).map_err(|_| Failure::Cleanup)?;
    }
    Ok(())
}

fn grant(startup: &sys::Startup, role: u64) -> Result<Handle, Failure> {
    startup
        .grant(role)
        .map(|grant| grant.handle)
        .ok_or(Failure::Startup)
}

fn entry(role: u64, kind: ObjectKind, rights: u64, handle: Handle) -> StartupGrant {
    StartupGrant {
        role,
        kind,
        rights,
        handle,
    }
}

fn id(value: u64) -> Result<ServiceId, Failure> {
    ServiceId::new(value).map_err(|_| Failure::Policy)
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
