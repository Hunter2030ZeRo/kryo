use kryo_core::capability::{CapabilityError, HandleTable, Rights, TransferBatch};
use kryo_core::ipc::{Completion, Endpoint, Message};
use kryo_core::object::{ObjectError, ObjectKind, ObjectTable};
use kryo_core::process::{ExitReason, ExitStatus, ProcessExit, ProcessTable, ProcessTableError};
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::storage::Slots;
use kryo_core::sync::{ParkResult, WaitError, WaitTable};
use kryo_core::task::{TaskError, TaskTable};

const EXIT: ExitStatus = ExitStatus {
    reason: ExitReason::Exited,
    code: 42,
};

#[test]
fn growing_tasks_preserve_ids_and_cpu_owners_across_reallocation() {
    let mut tasks = TaskTable::<usize, 0, 128>::growing();
    let ids: Vec<_> = (0..128).map(|value| tasks.spawn(value).unwrap()).collect();
    assert_eq!(tasks.dispatch_on(64), Ok(Some(ids[0])));
    assert_eq!(tasks.dispatch_on(127), Ok(Some(ids[1])));
    for value in 128..512 {
        tasks.spawn(value).unwrap();
    }
    assert_eq!(tasks.current_on(64), Ok(Some(ids[0])));
    assert_eq!(tasks.current_on(127), Ok(Some(ids[1])));
    assert_eq!(*tasks.resources_mut(ids[127]).unwrap(), 127);
    tasks.returned_on(64, ids[0], ReturnReason::Exit).unwrap();
    assert_eq!(
        tasks.reap(ids[0], |_| Err("retry")),
        Err(kryo_core::task::ReapError::Cleanup("retry"))
    );
    assert_eq!(tasks.state(ids[0]), Ok(TaskState::Dying));
    assert_eq!(tasks.reap(ids[0], |_| Ok::<_, ()>(())), Ok(0));
    let replacement = tasks.spawn(900).unwrap();
    assert_ne!(replacement, ids[0]);
    assert_eq!(tasks.state(ids[0]), Err(TaskError::InvalidId));
    assert_eq!(tasks.current_on(127), Ok(Some(ids[1])));
}

#[test]
fn growing_processes_keep_many_members_until_explicit_cleanup() {
    let mut tasks = TaskTable::<(), 0>::growing();
    let mut processes = ProcessTable::<usize, 0, 0>::growing();
    let ids: Vec<_> = (0..128)
        .map(|value| processes.insert(value).unwrap())
        .collect();
    let members: Vec<_> = (0..256).map(|_| tasks.spawn(()).unwrap()).collect();
    for member in &members {
        processes.reserve_thread(ids[0]).unwrap();
        processes.attach(ids[0], *member).unwrap();
    }
    assert_eq!(processes.thread_count(ids[0]), Ok(256));
    assert_eq!(processes.process_for(members[255]), Some(ids[0]));
    assert_eq!(*processes.resources_mut(ids[127]).unwrap(), 127);
    processes.request_exit(ids[0], EXIT).unwrap();
    for member in members {
        processes.end_thread(ids[0], member, EXIT).unwrap();
        processes.detach(ids[0], member).unwrap();
    }
    assert!(processes.reap(ids[0], |_| Err("retry")).is_err());
    assert_eq!(processes.terminal(ids[0]), Ok(Some(EXIT)));
    assert_eq!(processes.reap(ids[0], |_| Ok::<_, ()>(())), Ok(0));
    assert_ne!(processes.insert(999).unwrap(), ids[0]);
    assert_eq!(processes.state(ids[0]), Err(ProcessTableError::InvalidId));
}

#[test]
fn explicit_quotas_return_unpublished_owners_and_reuse_retired_capacity() {
    let mut tasks = TaskTable::<String, 0>::limited(1);
    let first = tasks.spawn("first".into()).unwrap();
    let failure = tasks.spawn("retained".into()).unwrap_err();
    assert_eq!(failure.error, TaskError::Capacity);
    assert_eq!(failure.resources, "retained");
    tasks.stop(first).unwrap();
    tasks.reap(first, |_| Ok::<_, ()>(())).unwrap();
    tasks.spawn(failure.resources).unwrap();

    let mut processes = ProcessTable::<String, 0, 0>::limited(1, 1);
    let process = processes.insert("owner".into()).unwrap();
    let failure = processes.insert("unpublished".into()).unwrap_err();
    assert_eq!(failure.resources, "unpublished");
    assert_eq!(processes.live_count(), 1);
    let mut members = TaskTable::<(), 2>::new();
    let one = members.spawn(()).unwrap();
    let two = members.spawn(()).unwrap();
    processes.attach(process, one).unwrap();
    assert_eq!(
        processes.attach(process, two),
        Err(ProcessTableError::ThreadCapacity)
    );
    assert_eq!(
        processes.threads(process).unwrap().collect::<Vec<_>>(),
        vec![one]
    );
}

#[test]
fn growing_waits_preserve_deadlines_and_parking_handoffs() {
    let mut tasks = TaskTable::<(), 0>::growing();
    let ids: Vec<_> = (0..256).map(|_| tasks.spawn(()).unwrap()).collect();
    let mut waits = WaitTable::<usize, 0>::growing();
    let registrations: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(value, id)| waits.begin_until(*id, value as u64 + 100).unwrap())
        .collect();
    for (value, wait) in registrations.into_iter().enumerate() {
        assert_eq!(waits.complete(wait, value), Ok(None));
        assert_eq!(waits.next_ready(), None);
        assert_eq!(waits.park(wait), Ok(ParkResult::Ready));
        assert_eq!(waits.take(wait), Ok(value));
        assert_eq!(waits.task(wait), Err(WaitError::InvalidId));
    }
    assert_eq!(waits.next_deadline(), None);
}

#[test]
fn growing_arenas_and_handles_retain_refs_and_pending_reservations() {
    let arena = ObjectTable::<usize, 0>::growing();
    let mut handles = HandleTable::<usize, 0, 0>::growing();
    let first = arena.create(ObjectKind::Image, 42).unwrap();
    let retained = first.try_clone().unwrap();
    let reservation = handles.reserve(1).unwrap();
    let old = first.id();
    for value in 0..512 {
        let object = arena.create(ObjectKind::Image, value).unwrap();
        handles.insert(object, Rights::READ | Rights::DUP).unwrap();
    }
    let result = handles
        .commit_transfer(reservation, TransferBatch::from_object(first, Rights::READ))
        .unwrap();
    assert_eq!(retained.id(), old);
    assert_eq!(retained.with(|value| *value), 42);
    assert_eq!(
        handles
            .lookup(result[0].unwrap(), ObjectKind::Image, Rights::READ)
            .unwrap()
            .with(|value| *value),
        42
    );
    assert_eq!(arena.live_count(), 513);
    drop(handles);
    assert_eq!(arena.live_count(), 1);
    drop(retained);
    assert_eq!(arena.live_count(), 0);
}

#[test]
fn growing_capability_quotas_preserve_refs_and_atomic_reservations() {
    let arena = ObjectTable::<usize, 0>::limited(1);
    let object = arena.create(ObjectKind::Image, 42).unwrap();
    let rejected = arena.create(ObjectKind::Image, 99).unwrap_err();
    assert_eq!(rejected.error, ObjectError::Capacity);
    assert_eq!(rejected.value, 99);
    let mut handles = HandleTable::<usize, 0, 0>::limited(1);
    assert_eq!(handles.reserve(2).unwrap_err(), CapabilityError::Capacity);
    assert_eq!(handles.reserved_count(), 0);
    let handle = handles.insert(object, Rights::READ | Rights::DUP).unwrap();
    assert_eq!(
        handles.duplicate(handle, Rights::READ),
        Err(CapabilityError::Capacity)
    );
    assert_eq!(
        handles
            .lookup(handle, ObjectKind::Image, Rights::READ)
            .unwrap()
            .reference_count(),
        2
    );
    handles.close(handle).unwrap();
    assert_eq!(arena.live_count(), 0);
}

#[test]
fn growing_exit_observers_all_receive_one_durable_result() {
    let mut tasks = TaskTable::<(), 0>::growing();
    let target = tasks.spawn(()).unwrap();
    let mut exit = ProcessExit::<0>::growing();
    exit.bind(target).unwrap();
    let mut waits = WaitTable::<Completion, 0>::growing();
    let registrations: Vec<_> = (0..128)
        .map(|_| {
            let task = tasks.spawn(()).unwrap();
            let wait = waits.begin(task).unwrap();
            exit.wait(task, wait, &mut waits).unwrap();
            wait
        })
        .collect();
    exit.finish(EXIT, &mut waits).unwrap();
    for wait in registrations {
        waits.park(wait).unwrap();
        assert_eq!(waits.take(wait), Ok(Completion::ProcessExited(EXIT)));
    }
}

#[test]
fn growing_endpoint_preserves_fifo_and_all_accepted_reply_identities() {
    let mut tasks = TaskTable::<(), 0>::growing();
    let owner = tasks.spawn(()).unwrap();
    let senders: Vec<_> = (0..128).map(|_| tasks.spawn(()).unwrap()).collect();
    let mut waits = WaitTable::<Completion, 0>::growing();
    let mut endpoint = Endpoint::<0, 0>::growing(owner);
    let pending: Vec<_> = senders
        .iter()
        .enumerate()
        .map(|(index, sender)| {
            let wait = waits.begin(*sender).unwrap();
            waits.park(wait).unwrap();
            endpoint
                .call(*sender, wait, Message([index as u64, 0, 0, 0]), &mut waits)
                .unwrap();
            wait
        })
        .collect();
    let mut replies = Vec::new();
    for (index, sender) in senders.iter().enumerate() {
        let receiver = waits.begin(owner).unwrap();
        waits.park(receiver).unwrap();
        endpoint.receive(owner, receiver, &mut waits).unwrap();
        let Completion::Received {
            message,
            reply: Some(token),
            sender: actual,
            ..
        } = waits.take(receiver).unwrap()
        else {
            panic!("Missing call")
        };
        assert_eq!(actual, *sender);
        assert_eq!(message.0[0], index as u64);
        replies.push(token);
    }
    for (index, (token, wait)) in replies.into_iter().zip(pending).enumerate() {
        let message = Message([index as u64 + 1, 0, 0, 0]);
        endpoint.reply(owner, token, message, &mut waits).unwrap();
        assert_eq!(waits.take(wait), Ok(Completion::Replied(message)));
    }
}

#[test]
fn storage_failure_does_not_publish_a_partial_growth() {
    let mut slots = Slots::<Option<usize>, 0>::limited(2);
    slots.reserve_len(1, || Some(7)).unwrap();
    assert!(slots.reserve_len(3, || Some(9)).is_err());
    assert_eq!(&*slots, &[Some(7)]);
    assert!(slots.try_reserve(usize::MAX).is_err());
    assert_eq!(&*slots, &[Some(7)]);
    let mut unbounded = Slots::<u64, 0>::growing();
    assert!(unbounded.try_reserve(usize::MAX).is_err());
    assert!(unbounded.is_empty());
    assert_eq!(unbounded.try_push(42), Ok(0));
}
