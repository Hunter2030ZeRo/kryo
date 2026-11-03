use super::{IpcRuntime, Tasks};
use kryo_core::task::TaskId;
use kryo_hal::UserContext;
use kryo_interfaces::syscall::{Call, Error};

#[cfg(feature = "boot-tests")]
pub(crate) mod checks;

pub(crate) fn dispatch(
    caller: TaskId,
    call: Call,
    context: UserContext,
    tasks: &mut Tasks<'_, '_>,
    ipc: &mut IpcRuntime<'_>,
) -> Result<Option<u64>, Error> {
    if call == Call::Wake {
        let key = tasks
            .resources_mut(caller)
            .map_err(|_| Error::PeerClosed)?
            .process
            .space
            .private_word_key(context.rdi)
            .map_err(|_| Error::InvalidArgument)?;
        let maximum = usize::try_from(context.rsi).map_err(|_| Error::InvalidArgument)?;
        return Ok(Some(ipc.wake_words(key, maximum) as u64));
    }
    let expected = u32::try_from(context.rsi).map_err(|_| Error::InvalidArgument)?;
    // Every CPU lease has ended before dispatch. Compare and queue publication
    // share BSP ownership with wake and mapping retirement; users cannot race
    // a store between these operations in the current execution-batch model.
    let (key, value, account) = {
        let owner = tasks.resources_mut(caller).map_err(|_| Error::PeerClosed)?;
        let (key, value) = owner
            .process
            .space
            .private_word(context.rdi)
            .map_err(|_| Error::InvalidArgument)?;
        (key, value, owner.process.account.clone())
    };
    if value != expected {
        return Ok(Some(1));
    }
    let deadline = (call == Call::ParkUntil).then_some(context.rdx);
    if deadline.is_some_and(|deadline| {
        deadline <= kryo_hal::monotonic_time().expect("Private wait clock invariant")
    }) {
        return Err(Error::TimedOut);
    }
    let length = ipc
        .words
        .len()
        .checked_add(1)
        .ok_or(Error::OutOfResources)?;
    ipc.words
        .prepare(length)
        .map_err(|_| Error::OutOfResources)?;
    let wait = ipc.begin_wait_for(caller, deadline, account.as_ref())?;
    ipc.words
        .push(key, wait)
        .expect("Prepared private word queue invariant");
    tasks.resources_mut(caller).unwrap().wait = Some(wait);
    ipc.waits
        .park(wait)
        .expect("Private word handoff invariant");
    Ok(None)
}
