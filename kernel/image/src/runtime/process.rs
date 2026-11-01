use super::launch::{self, PendingLoad};
use super::{CallStats, IpcRuntime, KernelObject, KernelRegion, Object, Tasks};
use core::cell::RefCell;
use kryo_core::capability::{Handle, Rights, TransferBatch, TransferSpec};
use kryo_core::ipc::{Completion, Delivery};
use kryo_core::object::ObjectKind;
use kryo_core::process::ProcessId;
use kryo_core::task::TaskId;
use kryo_hal::UserContext;
use kryo_interfaces::syscall::Error;

enum Startup<'objects> {
    Legacy(super::Batch<'objects>),
    Manifest(super::startup::Prepared<'objects>),
}

pub(super) fn spawn<'kernel, 'objects>(
    caller: TaskId,
    context: UserContext,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    region: &mut KernelRegion,
) -> Result<u64, Error> {
    spawn_inner(caller, context, tasks, ipc, stats, region, false)
}

pub(super) fn spawn_manifest<'kernel, 'objects>(
    caller: TaskId,
    context: UserContext,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    region: &mut KernelRegion,
) -> Result<u64, Error> {
    spawn_inner(caller, context, tasks, ipc, stats, region, true)
}

fn spawn_inner<'kernel, 'objects>(
    caller: TaskId,
    context: UserContext,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    stats: &mut CallStats,
    region: &mut KernelRegion,
    manifest: bool,
) -> Result<u64, Error> {
    let arena = ipc.arena.ok_or(Error::InvalidCall)?;
    if tasks.reserve_process().is_err() {
        return Err(Error::OutOfResources);
    }
    let owner = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
    let parent = tasks.resources_mut(caller).map_err(|_| Error::PeerClosed)?;
    let pool = parent.pool;
    if let Some(pending) = parent.process.pending.as_mut() {
        if !pending.close(pool, region) {
            return Err(Error::OutOfResources);
        }
        parent.process.pending = None;
    }
    let first_slot = region
        .find_stack(pool, parent.direct)
        .map_err(|_| Error::OutOfResources)?;
    let image = parent
        .process
        .handles
        .lookup(
            Handle::from_raw(context.rdi),
            ObjectKind::Image,
            Rights::EXECUTE,
        )
        .map_err(super::native::map_error)?;
    let bytes = image.with(|object| match object {
        KernelObject::Image(bytes) => *bytes,
        _ => unreachable!("Image capability kind invariant"),
    });
    let count = if manifest {
        0
    } else {
        usize::try_from(context.rsi).map_err(|_| Error::InvalidArgument)?
    };
    if count > 2 {
        return Err(Error::InvalidArgument);
    }
    let mut specs = [TransferSpec {
        handle: Handle::INVALID,
        rights: Rights::READ,
    }; 2];
    for index in 0..count {
        specs[index] = TransferSpec {
            handle: Handle::from_raw([context.r12, context.r13][index]),
            rights: Rights::from_bits([context.r14, context.r15][index])
                .ok_or(Error::InvalidArgument)?,
        };
    }
    let startup = if manifest {
        Startup::Manifest(super::startup::prepare(
            owner,
            Handle::from_raw(context.rsi),
            &parent.process.handles,
        )?)
    } else {
        Startup::Legacy(
            parent
                .process
                .handles
                .prepare_transfer(&specs[..count])
                .map_err(super::native::map_error)?,
        )
    };
    let mut reservation = Some(
        parent
            .process
            .handles
            .reserve(1)
            .map_err(super::native::map_error)?,
    );
    let (pool, direct, memory) = (parent.pool, parent.direct, parent.memory);
    let result = (|| {
        let mut child = match launch::prepare(pool, direct, memory, region, first_slot, bytes) {
            Ok(child) => child,
            Err(failure) => {
                retain_failed(tasks, caller, failure.pending, region);
                return Err(failure.error);
            }
        };
        let installed = match startup {
            Startup::Legacy(batch) => child
                .process
                .handles
                .reserve(count)
                .map_err(super::native::map_error)
                .map(|slots| {
                    let grants = child
                        .process
                        .handles
                        .commit_transfer(slots, batch)
                        .unwrap_or_else(|_| panic!("Fresh child startup grant invariant"));
                    (
                        grants[0].map_or(0, |handle| handle.as_raw()),
                        grants[1].map_or(0, |handle| handle.as_raw()),
                    )
                }),
            Startup::Manifest(manifest) => manifest
                .install(&mut child.process.handles, arena)
                .map(|record| (record.as_raw(), 0)),
        };
        match installed {
            Ok((first, second)) => {
                child.context.rdi = first;
                child.context.rsi = second;
            }
            Err(error) => {
                retain_failed(tasks, caller, PendingLoad::from_resources(child), region);
                return Err(error);
            }
        }
        let control = match arena.create(
            ObjectKind::ProcessControl,
            KernelObject::ProcessControl(RefCell::new(super::ProcessControl::new())),
        ) {
            Ok(control) => control,
            Err(_) => {
                retain_failed(tasks, caller, PendingLoad::from_resources(child), region);
                return Err(Error::OutOfResources);
            }
        };
        child.process.control = match control.try_clone() {
            Ok(owner) => Some(owner),
            Err(_) => {
                retain_failed(tasks, caller, PendingLoad::from_resources(child), region);
                return Err(Error::OutOfResources);
            }
        };
        #[cfg(feature = "boot-tests")]
        if manifest {
            super::startup::checks::before_publication();
        }
        let task = match tasks.spawn(child) {
            Ok(task) => task,
            Err(failure) => {
                retain_failed(
                    tasks,
                    caller,
                    PendingLoad::from_resources(failure.resources),
                    region,
                );
                return Err(Error::OutOfResources);
            }
        };
        bind(&control, task, tasks.process_for(task).unwrap());
        let handles = tasks
            .resources_mut(caller)
            .unwrap()
            .process
            .handles
            .commit_transfer(
                reservation.take().unwrap(),
                TransferBatch::from_object(
                    control,
                    Rights::INSPECT
                        | Rights::CONTROL
                        | Rights::WAIT
                        | Rights::DUP
                        | Rights::TRANSFER,
                ),
            )
            .unwrap_or_else(|_| panic!("Reserved process handle publication invariant"));
        stats.spawned += 1;
        let _ = kryo_hal::print(format_args!(
            "PROCESS: spawned id={:#x} parent={:#x}\n",
            task.as_raw(),
            caller.as_raw()
        ));
        Ok(handles[0].unwrap().as_raw())
    })();
    if let Some(reservation) = reservation {
        tasks
            .resources_mut(caller)
            .unwrap()
            .process
            .handles
            .release_reservation(reservation)
            .unwrap_or_else(|_| panic!("Spawn reservation rollback invariant"));
    }
    result
}

fn retain_failed<'kernel, 'objects>(
    tasks: &mut Tasks<'kernel, 'objects>,
    caller: TaskId,
    mut pending: PendingLoad<'kernel, 'objects>,
    region: &mut KernelRegion,
) {
    let parent = tasks.resources_mut(caller).unwrap();
    if !pending.close(parent.pool, region) {
        assert!(
            parent.process.pending.is_none(),
            "One retained creation per parent invariant"
        );
        parent.process.pending = Some(pending);
    }
}

pub(super) fn bind(control: &Object<'_>, task: TaskId, id: ProcessId) {
    control.with(|object| {
        let KernelObject::ProcessControl(process) = object else {
            unreachable!()
        };
        let mut process = process.borrow_mut();
        if process.process.is_some() {
            assert_eq!(process.process, Some(id));
            assert_eq!(process.exit.task(), Some(task));
            return;
        }
        process.process = Some(id);
        process
            .exit
            .bind(task)
            .expect("New process control binding invariant");
    });
}

pub(super) fn wait<'objects>(
    caller: TaskId,
    control: Object<'objects>,
    deadline: Option<u64>,
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) -> Result<Option<u64>, Error> {
    if deadline.is_some_and(|deadline| {
        deadline <= kryo_hal::monotonic_time().expect("Missing process clock")
    }) {
        return Err(Error::TimedOut);
    }
    let target = control
        .with(|object| {
            let KernelObject::ProcessControl(control) = object else {
                unreachable!()
            };
            control.borrow().process
        })
        .ok_or(Error::PeerClosed)?;
    if tasks.process_for(caller) == Some(target) {
        return Err(Error::PermissionDenied);
    }
    let wait = ipc.begin_wait(caller, deadline)?;
    tasks.resources_mut(caller).unwrap().wait = Some(wait);
    let result = control.with(|object| {
        let KernelObject::ProcessControl(process) = object else {
            unreachable!()
        };
        process.borrow_mut().wait(caller, wait, &mut ipc.waits)
    });
    ipc.observe(wait, control);
    if let Err(error) = result {
        ipc.waits
            .complete(
                wait,
                Completion::Failed {
                    error,
                    delivery: Delivery::NotDelivered,
                },
            )
            .expect("Rejected observation completed its wait");
    }
    ipc.waits
        .park(wait)
        .expect("Process observation handoff invariant");
    Ok(None)
}
