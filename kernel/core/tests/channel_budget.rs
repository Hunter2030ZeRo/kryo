use core::cell::RefCell;
use kryo_core::capability::{CapabilityError, HandleTable, Rights, TransferSpec};
use kryo_core::ipc::{ChannelBudget, Endpoint, IpcError, Notification};
use kryo_core::object::{ObjectKind, ObjectTable};
use kryo_core::sync::wait::WaitTable;
use kryo_core::task::TaskTable;

#[test]
fn cloned_factories_share_exhaustion_and_reservation_rollback() {
    let budget = ChannelBudget::new(2);
    let delegated = budget.clone();
    assert_eq!(delegated.limit(), 2);
    let first = budget.try_reserve().unwrap();
    let second = delegated.try_reserve().unwrap();
    assert!(matches!(budget.try_reserve(), Err(IpcError::Capacity)));
    assert!(matches!(delegated.try_reserve(), Err(IpcError::Capacity)));
    assert_eq!(budget.live_count(), 2);
    drop(first);
    let replacement = delegated.try_reserve().unwrap();
    assert_eq!(budget.live_count(), 2);
    drop((second, replacement));
    assert_eq!(budget.live_count(), 0);
    assert!(matches!(
        ChannelBudget::new(0).try_reserve(),
        Err(IpcError::Capacity)
    ));
}

#[test]
fn failed_object_and_handle_publication_preserve_the_charge_until_rollback() {
    let mut tasks = TaskTable::<(), 1>::new();
    let owner = tasks.spawn(()).unwrap();
    let budget = ChannelBudget::new(1);
    let full = ObjectTable::<Notification, 0>::new();
    let failure = full
        .create(
            ObjectKind::Notification,
            Notification::leased(owner, budget.try_reserve().unwrap()),
        )
        .unwrap_err();
    assert_eq!(budget.live_count(), 1);
    drop(failure.value);
    assert_eq!(budget.live_count(), 0);

    let objects = ObjectTable::<Notification, 1>::new();
    let notification = objects
        .create(
            ObjectKind::Notification,
            Notification::leased(owner, budget.try_reserve().unwrap()),
        )
        .unwrap();
    let mut handles = HandleTable::<Notification, 1, 0>::new();
    let failure = handles.insert(notification, Rights::WAIT).unwrap_err();
    assert_eq!(budget.live_count(), 1);
    drop(failure.object);
    assert_eq!(budget.live_count(), 0);
}

#[test]
fn closed_endpoint_remains_charged_through_transfer_and_stale_references() {
    let mut tasks = TaskTable::<(), 1>::new();
    let owner = tasks.spawn(()).unwrap();
    let budget = ChannelBudget::new(1);
    let objects = ObjectTable::<RefCell<Endpoint<0, 0>>, 1>::new();
    let endpoint = objects
        .create(
            ObjectKind::Endpoint,
            RefCell::new(Endpoint::leased(owner, budget.try_reserve().unwrap())),
        )
        .unwrap();
    let mut creator = HandleTable::<RefCell<Endpoint<0, 0>>, 1, 1>::new();
    let source = creator
        .insert(
            endpoint.try_clone().unwrap(),
            Rights::SEND | Rights::TRANSFER,
        )
        .unwrap();
    let batch = creator
        .prepare_transfer(&[TransferSpec {
            handle: source,
            rights: Rights::SEND,
        }])
        .unwrap();
    drop(creator);
    assert_eq!(budget.live_count(), 1);

    let mut receiver = HandleTable::<RefCell<Endpoint<0, 0>>, 1, 1>::new();
    let reservation = receiver.reserve(1).unwrap();
    let received = receiver.commit_transfer(reservation, batch).unwrap()[0].unwrap();
    let retained = receiver
        .lookup(received, ObjectKind::Endpoint, Rights::SEND)
        .unwrap();
    endpoint.with(|endpoint| endpoint.borrow_mut().close(&mut WaitTable::<_, 0>::new()));
    drop(endpoint);
    receiver.close(received).unwrap();
    assert_eq!(budget.live_count(), 1);
    retained.with(|endpoint| assert!(endpoint.borrow().is_closed()));
    drop(retained);
    assert_eq!(budget.live_count(), 0);
}

#[test]
fn notification_owner_death_does_not_refund_a_retained_payload() {
    let mut tasks = TaskTable::<(), 1>::new();
    let owner = tasks.spawn(()).unwrap();
    let budget = ChannelBudget::new(1);
    let objects = ObjectTable::<RefCell<Notification>, 1>::new();
    let notification = objects
        .create(
            ObjectKind::Notification,
            RefCell::new(Notification::leased(owner, budget.try_reserve().unwrap())),
        )
        .unwrap();
    let peer = notification.try_clone().unwrap();
    notification.with(|notification| {
        notification
            .borrow_mut()
            .task_died(owner, &mut WaitTable::<_, 0>::new());
    });
    drop(notification);
    assert_eq!(budget.live_count(), 1);
    peer.with(|notification| {
        assert_eq!(
            notification
                .borrow_mut()
                .signal(&mut WaitTable::<_, 0>::new()),
            Err(IpcError::PeerClosed)
        );
    });
    drop(peer);
    assert_eq!(budget.live_count(), 0);
}

#[test]
fn factory_capabilities_delegate_creation_without_resetting_the_budget() {
    let budget = ChannelBudget::new(1);
    let objects = ObjectTable::<ChannelBudget, 1>::new();
    let factory = objects
        .create(ObjectKind::ChannelFactory, budget.clone())
        .unwrap();
    let mut creator = HandleTable::<ChannelBudget, 1, 3>::new();
    let handle = creator
        .insert(factory, Rights::CREATE | Rights::DUP | Rights::TRANSFER)
        .unwrap();
    let duplicate = creator.duplicate(handle, Rights::CREATE).unwrap();
    assert_eq!(
        creator.duplicate(handle, Rights::CREATE | Rights::CONTROL),
        Err(CapabilityError::PermissionDenied)
    );
    assert!(matches!(
        creator.prepare_transfer(&[TransferSpec {
            handle,
            rights: Rights::CREATE | Rights::CONTROL,
        }]),
        Err(CapabilityError::PermissionDenied)
    ));
    let batch = creator
        .prepare_transfer(&[TransferSpec {
            handle,
            rights: Rights::CREATE,
        }])
        .unwrap();
    let mut peer = HandleTable::<ChannelBudget, 1, 1>::new();
    let reservation = peer.reserve(1).unwrap();
    let received = peer.commit_transfer(reservation, batch).unwrap()[0].unwrap();
    let lease = creator
        .lookup(duplicate, ObjectKind::ChannelFactory, Rights::CREATE)
        .unwrap()
        .with(ChannelBudget::try_reserve)
        .unwrap();
    let delegated = peer
        .lookup(received, ObjectKind::ChannelFactory, Rights::CREATE)
        .unwrap();
    assert!(matches!(
        delegated.with(ChannelBudget::try_reserve),
        Err(IpcError::Capacity)
    ));
    assert_eq!(
        peer.duplicate(received, Rights::CREATE),
        Err(CapabilityError::PermissionDenied)
    );
    assert!(matches!(
        peer.prepare_transfer(&[TransferSpec {
            handle: received,
            rights: Rights::CREATE,
        }]),
        Err(CapabilityError::PermissionDenied)
    ));
    drop((creator, peer, delegated));
    assert_eq!(budget.live_count(), 1);
    drop(lease);
    assert_eq!(budget.live_count(), 0);
}
