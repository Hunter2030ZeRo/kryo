use super::{IpcRuntime, Tasks};
use kryo_core::mm::vm::{PrivateError, VmError};
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_core::task::TaskId;
use kryo_hal::UserContext;
use kryo_interfaces::syscall::{Call, Error};

pub(crate) fn dispatch(
    caller: TaskId,
    call: Call,
    context: UserContext,
    tasks: &mut Tasks<'_, '_>,
    ipc: &mut IpcRuntime<'_>,
) -> Result<u64, Error> {
    let process = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
    let (result, pending) = {
        let owner = tasks.process_mut(process).map_err(|_| Error::PeerClosed)?;
        let result = match call {
            Call::VmReserve => {
                let account = owner.account.as_ref().ok_or(Error::PermissionDenied)?;
                owner
                    .private_vm
                    .reserve(&owner.space, account, context.rdi, context.rsi)
            }
            Call::VmCommit => owner
                .private_vm
                .commit(&mut owner.space, context.rdi, context.rsi)
                .map(|()| 0),
            Call::VmDecommit => {
                let space = owner.space.identity();
                owner
                    .private_vm
                    .decommit_with(&mut owner.space, context.rdi, context.rsi, |start, end| {
                        ipc.invalidate_words(space, start, end)
                    })
                    .map(|()| 0)
            }
            Call::VmRelease => {
                if context.rsi != 0 {
                    return Err(Error::InvalidArgument);
                }
                let space = owner.space.identity();
                owner
                    .private_vm
                    .release_with(&mut owner.space, context.rdi, |start, end| {
                        ipc.invalidate_words(space, start, end)
                    })
                    .map(|()| 0)
            }
            _ => return Err(Error::InvalidCall),
        };
        (
            result.map_err(map_error),
            owner.private_vm.retirement_pending(),
        )
    };
    if pending {
        // A failed rollback must not expose a partially published commit. The
        // process keeps every owner/charge until normal teardown can retire it.
        tasks
            .stop_process(
                process,
                ExitStatus {
                    reason: ExitReason::Fault,
                    code: 0x200,
                },
            )
            .map_err(|_| Error::PeerClosed)?;
        kryo_hal::emergency_write_str("VM: incomplete retirement quarantined process\n");
        return Err(Error::PeerClosed);
    }
    result
}

fn map_error<E>(error: PrivateError<E>) -> Error {
    match error {
        PrivateError::InvalidRange | PrivateError::Conflict => Error::InvalidArgument,
        PrivateError::OutOfResources | PrivateError::RetirementPending => Error::OutOfResources,
        PrivateError::Vm(
            VmError::InvalidAddress
            | VmError::AlreadyMapped
            | VmError::NotMapped
            | VmError::InvalidContents
            | VmError::InvalidPermissions,
        ) => Error::InvalidArgument,
        PrivateError::Vm(_) => Error::OutOfResources,
    }
}
