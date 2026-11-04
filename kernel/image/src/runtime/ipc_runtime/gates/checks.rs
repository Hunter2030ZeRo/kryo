use super::*;
use crate::runtime::RefCell;
use kryo_core::capability::TransferSpec;
use kryo_core::ipc::{ChannelBudget, Endpoint};
use kryo_core::task::TaskTable;

pub(crate) fn run() {
    let mut owners = TaskTable::<(), 2>::new();
    let owner = owners.spawn(()).unwrap();
    let other = owners.spawn(()).unwrap();
    delegation(owner, other);
    closed_reuse(owner);
    publication_failure(owner);
}

fn root<'objects>(
    objects: &'objects Objects,
    owner: TaskId,
    budget: &ChannelBudget,
    ipc: &mut IpcRuntime<'objects>,
) -> Object<'objects> {
    let root = objects
        .create(
            ObjectKind::Endpoint,
            KernelObject::Endpoint(RefCell::new(Endpoint::leased(
                owner,
                budget.try_reserve().unwrap(),
            ))),
        )
        .unwrap();
    ipc.register(root.try_clone().unwrap());
    root
}

fn delegation(owner: TaskId, other: TaskId) {
    let objects = Objects::growing();
    let mut ipc = IpcRuntime::growing(&objects);
    let mut tasks = Tasks::growing();
    let budget = ChannelBudget::new(2);
    let root = root(&objects, owner, &budget, &mut ipc);
    let mut sender = Handles::growing();
    let mut receiver = Handles::growing();
    assert_eq!(
        ipc.create_gate(other, root.try_clone().unwrap(), 7, &mut sender),
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        ipc.create_gate(owner, root.try_clone().unwrap(), 0, &mut sender),
        Err(Error::InvalidArgument)
    );
    let label = 0xfedc_ba98_7654_3210;
    let gate = ipc
        .create_gate(owner, root.try_clone().unwrap(), label, &mut sender)
        .unwrap();
    assert_eq!(
        ipc.create_gate(owner, root.try_clone().unwrap(), label, &mut sender),
        Err(Error::Busy)
    );
    assert_eq!(
        ipc.create_gate(owner, root.try_clone().unwrap(), 8, &mut sender),
        Err(Error::OutOfResources)
    );
    let copy = sender.duplicate(gate, Rights::SEND | Rights::DUP).unwrap();
    let loan = sender
        .lookup(gate, ObjectKind::EndpointGate, Rights::SEND)
        .unwrap();
    assert_eq!(ipc.gate_target(&loan).unwrap().0.id(), root.id());
    let batch = sender
        .prepare_transfer(&[TransferSpec {
            handle: gate,
            rights: Rights::SEND,
        }])
        .unwrap();
    sender.close(gate).unwrap();
    sender.close(copy).unwrap();
    ipc.settle(&mut tasks);
    let scans = ipc.gate_scans;
    for _ in 0..32 {
        ipc.settle(&mut tasks);
    }
    assert_eq!(
        ipc.gate_scans, scans,
        "Steady settlement must not scan gates"
    );
    drop(loan);
    ipc.settle(&mut tasks);
    assert_eq!(
        ipc.gate_scans, scans,
        "Staged transfer still owns the authority"
    );
    let reservation = receiver.reserve(1).unwrap();
    let received = receiver.commit_transfer(reservation, batch).unwrap()[0].unwrap();
    let retained = receiver
        .lookup(received, ObjectKind::EndpointGate, Rights::SEND)
        .unwrap();
    let (_, actual) = ipc.gate_target(&retained).unwrap();
    assert_eq!(actual, label);
    drop(retained);
    receiver.close(received).unwrap();
    ipc.settle(&mut tasks);
    assert_eq!(ipc.gate_count(), 1);
    assert_eq!(
        budget.live_count(),
        2,
        "Pending closure retains its payload and credit"
    );
    assert_eq!(
        ipc.create_gate(owner, root.try_clone().unwrap(), label, &mut sender),
        Err(Error::Busy)
    );
    let wait = ipc.waits.begin(owner).unwrap();
    ipc.waits.park(wait).unwrap();
    root.with(|object| {
        let KernelObject::Endpoint(endpoint) = object else {
            unreachable!()
        };
        endpoint
            .borrow_mut()
            .receive(owner, wait, &mut ipc.waits)
            .unwrap();
    });
    let key = match ipc.waits.take(wait).unwrap() {
        Completion::GateClosed { label: actual, key } => {
            assert_eq!(actual, label);
            key
        }
        completion => panic!("Expected gate event {completion:?}"),
    };
    ipc.retire_gate_event(key);
    assert_eq!(ipc.gate_count(), 0);
    assert_eq!(budget.live_count(), 1);
    drop(root);
    ipc.settle(&mut tasks);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(budget.live_count(), 0);
    assert_eq!(sender.reserved_count(), 0);
    assert_eq!(receiver.reserved_count(), 0);
}

fn closed_reuse(owner: TaskId) {
    let objects = Objects::growing();
    let mut ipc = IpcRuntime::growing(&objects);
    let mut tasks = Tasks::growing();
    let budget = ChannelBudget::new(8);
    let first = root(&objects, owner, &budget, &mut ipc);
    let mut handles = Handles::growing();
    let old = ipc
        .create_gate(owner, first.try_clone().unwrap(), 42, &mut handles)
        .unwrap();
    let stale = handles
        .lookup(old, ObjectKind::EndpointGate, Rights::SEND)
        .unwrap();
    first.with(|object| {
        let KernelObject::Endpoint(endpoint) = object else {
            unreachable!()
        };
        endpoint.borrow_mut().close(&mut ipc.waits);
    });
    ipc.settle(&mut tasks);
    assert_eq!(ipc.gate_count(), 0);
    assert_eq!(
        budget.live_count(),
        2,
        "Closed payload remains charged through stale handles"
    );
    let second = root(&objects, owner, &budget, &mut ipc);
    let new = ipc
        .create_gate(owner, second.try_clone().unwrap(), 42, &mut handles)
        .unwrap();
    let current = handles
        .lookup(new, ObjectKind::EndpointGate, Rights::SEND)
        .unwrap();
    assert_eq!(ipc.gate_target(&stale).err(), Some(Error::PeerClosed));
    assert_eq!(ipc.gate_target(&current).unwrap().0.id(), second.id());
    drop(current);
    handles.close(old).unwrap();
    drop(stale);
    drop(first);
    handles.close(new).unwrap();
    ipc.settle(&mut tasks);
    assert_eq!(ipc.gate_count(), 1);
    // Owner death discards the pending event without needing another Receive.
    ipc.task_died(owner);
    ipc.settle(&mut tasks);
    assert_eq!(ipc.gate_count(), 0);
    drop(second);
    ipc.settle(&mut tasks);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(budget.live_count(), 0);
}

fn publication_failure(owner: TaskId) {
    for arena_limit in [1, 8] {
        let objects = Objects::limited(arena_limit);
        let mut ipc = IpcRuntime::growing(&objects);
        let budget = ChannelBudget::new(2);
        let root = root(&objects, owner, &budget, &mut ipc);
        let mut handles = Handles::limited(if arena_limit == 1 { 8 } else { 0 });
        assert_eq!(
            ipc.create_gate(owner, root.try_clone().unwrap(), 7, &mut handles),
            Err(Error::OutOfResources)
        );
        assert_eq!(handles.reserved_count(), 0);
        assert_eq!(handles.live_count(), 0);
        assert_eq!(ipc.gate_count(), 0);
        assert_eq!(objects.live_count(), 1);
        assert_eq!(budget.live_count(), 1);
        drop(root);
        ipc.settle(&mut Tasks::growing());
        assert_eq!(objects.live_count(), 0);
        assert_eq!(budget.live_count(), 0);
    }
}
