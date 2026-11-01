use super::{IpcRuntime, map_error};
use crate::runtime::{Handles, KernelObject, Objects, RefCell};
use kryo_core::capability::{Rights, TransferSpec};
use kryo_core::ipc::{
    ChannelBudget, Completion, Delivery, Endpoint, IpcError, Message, Notification,
};
use kryo_core::object::ObjectKind;
use kryo_core::task::{TaskId, TaskTable};
use kryo_interfaces::syscall::Error;

pub(crate) fn run() {
    let mut tasks = TaskTable::<(), 3>::new();
    let owner = tasks.spawn(()).unwrap();
    let sender = tasks.spawn(()).unwrap();
    let sibling = tasks.spawn(()).unwrap();
    registry_only(owner, sender);
    owner_exit(owner, sender, sibling);
    accepted_transfer(owner, sender);
    device_retirement(owner);
    kryo_hal::emergency_write_str("IPC: channel registry retirement and retained charges OK\n");
}

fn registry_only(owner: TaskId, sender: TaskId) {
    let objects = Objects::new();
    let budget = ChannelBudget::new(1);
    let mut ipc = IpcRuntime::with_objects(&objects);
    let endpoint = objects
        .create(
            ObjectKind::Endpoint,
            KernelObject::Endpoint(RefCell::new(Endpoint::leased(
                owner,
                budget.try_reserve().unwrap(),
            ))),
        )
        .unwrap();
    let wait = ipc.begin_wait(sender, None).unwrap();
    endpoint.with(|object| {
        let KernelObject::Endpoint(endpoint) = object else {
            unreachable!()
        };
        endpoint
            .borrow_mut()
            .call(sender, wait, Message([7; 4]), &mut ipc.waits)
            .unwrap();
    });
    ipc.waits.park(wait).unwrap();
    ipc.register(endpoint);
    assert!(ipc.retire_channels());
    assert_eq!(
        ipc.waits.take(wait),
        Ok(Completion::Failed {
            error: IpcError::Closed,
            delivery: Delivery::NotDelivered,
        })
    );
    assert_eq!(map_error(IpcError::Closed), Error::PeerClosed);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(budget.live_count(), 0);

    let notification = objects
        .create(
            ObjectKind::Notification,
            KernelObject::Notification(RefCell::new(Notification::leased(
                owner,
                budget.try_reserve().unwrap(),
            ))),
        )
        .unwrap();
    let wait = ipc.begin_wait(owner, None).unwrap();
    notification.with(|object| {
        let KernelObject::Notification(notification) = object else {
            unreachable!()
        };
        notification
            .borrow_mut()
            .wait(owner, wait, &mut ipc.waits)
            .unwrap();
    });
    ipc.waits.park(wait).unwrap();
    ipc.register(notification);
    assert!(ipc.retire_channels());
    assert_eq!(
        ipc.waits.take(wait),
        Ok(Completion::Failed {
            error: IpcError::Closed,
            delivery: Delivery::NotDelivered,
        })
    );
    assert_eq!(objects.live_count(), 0);
    assert_eq!(budget.live_count(), 0);
}

fn owner_exit(owner: TaskId, sender: TaskId, sibling: TaskId) {
    let objects = Objects::new();
    let budget = ChannelBudget::new(1);
    let mut ipc = IpcRuntime::with_objects(&objects);
    let endpoint = objects
        .create(
            ObjectKind::Endpoint,
            KernelObject::Endpoint(RefCell::new(Endpoint::leased(
                owner,
                budget.try_reserve().unwrap(),
            ))),
        )
        .unwrap();
    ipc.register(endpoint.try_clone().unwrap());
    let receive = ipc.begin_wait(owner, None).unwrap();
    let call = ipc.begin_wait(sender, None).unwrap();
    endpoint.with(|object| {
        let KernelObject::Endpoint(endpoint) = object else {
            unreachable!()
        };
        let mut endpoint = endpoint.borrow_mut();
        endpoint.receive(owner, receive, &mut ipc.waits).unwrap();
        endpoint
            .call(sender, call, Message([11; 4]), &mut ipc.waits)
            .unwrap();
    });
    ipc.waits.park(receive).unwrap();
    ipc.waits.park(call).unwrap();
    assert!(matches!(
        ipc.waits.take(receive),
        Ok(Completion::Received { reply: Some(_), .. })
    ));
    ipc.task_died(sibling);
    assert!(!ipc.retire_channels());
    ipc.task_died(owner);
    assert_eq!(
        ipc.waits.take(call),
        Ok(Completion::Failed {
            error: IpcError::PeerClosed,
            delivery: Delivery::Accepted,
        })
    );
    assert!(ipc.retire_channels());
    assert_eq!(endpoint.reference_count(), 1);
    assert_eq!(budget.live_count(), 1);
    let retry = ipc.begin_wait(sender, None).unwrap();
    endpoint.with(|object| {
        let KernelObject::Endpoint(endpoint) = object else {
            unreachable!()
        };
        assert_eq!(
            endpoint
                .borrow_mut()
                .call(sender, retry, Message::default(), &mut ipc.waits),
            Err(IpcError::PeerClosed)
        );
    });
    ipc.waits.abandon(retry).unwrap();
    drop(endpoint);
    assert_eq!(budget.live_count(), 0);
    assert_eq!(objects.live_count(), 0);
}

fn accepted_transfer(owner: TaskId, sender: TaskId) {
    let objects = Objects::new();
    let budget = ChannelBudget::new(1);
    let mut ipc = IpcRuntime::with_objects(&objects);
    let notification = objects
        .create(
            ObjectKind::Notification,
            KernelObject::Notification(RefCell::new(Notification::leased(
                owner,
                budget.try_reserve().unwrap(),
            ))),
        )
        .unwrap();
    ipc.register(notification.try_clone().unwrap());
    let mut source = Handles::new();
    let handle = source
        .insert(notification, Rights::SIGNAL | Rights::TRANSFER)
        .unwrap();
    let batch = source
        .prepare_transfer(&[TransferSpec {
            handle,
            rights: Rights::SIGNAL,
        }])
        .unwrap();
    let send = ipc.begin_wait(sender, None).unwrap();
    let receive = ipc.begin_wait(owner, None).unwrap();
    ipc.hold_transfer(sender, send, batch);
    let mut endpoint = Endpoint::<2, 2>::new(owner);
    endpoint
        .receive_caps(owner, receive, 1, &mut ipc.waits)
        .unwrap();
    endpoint
        .send_caps(sender, send, Message::default(), 1, &mut ipc.waits)
        .unwrap();
    ipc.waits.park(receive).unwrap();
    ipc.waits.park(send).unwrap();
    source.close(handle).unwrap();
    assert!(!ipc.retire_channels());
    assert_eq!(budget.live_count(), 1);
    assert!(matches!(
        ipc.waits.take(receive),
        Ok(Completion::Received {
            capabilities: 1,
            ..
        })
    ));
    let transfer = ipc.take_transfer(send).unwrap();
    assert_eq!(transfer.sender, sender);
    let mut receiver = Handles::new();
    let reserved = receiver.reserve(1).unwrap();
    let installed = receiver.commit_transfer(reserved, transfer.batch).unwrap()[0].unwrap();
    assert_eq!(ipc.waits.take(send), Ok(Completion::Sent));
    ipc.task_died(owner);
    assert!(ipc.retire_channels());
    assert_eq!(budget.live_count(), 1);
    receiver.close(installed).unwrap();
    assert_eq!(budget.live_count(), 0);
    assert_eq!(objects.live_count(), 0);
}

fn device_retirement(owner: TaskId) {
    let objects = Objects::new();
    let budget = ChannelBudget::new(1);
    let mut ipc = IpcRuntime::with_objects(&objects);
    let signal = objects
        .create(
            ObjectKind::Notification,
            KernelObject::Notification(RefCell::new(Notification::leased(
                owner,
                budget.try_reserve().unwrap(),
            ))),
        )
        .unwrap();
    let id = signal.id();
    ipc.register(signal.try_clone().unwrap());
    ipc.device_signal = Some(signal);
    ipc.task_died(owner);
    assert!(!ipc.retire_channels());
    assert_eq!(ipc.channels.iter().flatten().count(), 1);
    assert_eq!(budget.live_count(), 1);
    // Hardware retirement releases these two roots together, including retries.
    ipc.unregister(id);
    assert_eq!(budget.live_count(), 1);
    ipc.device_signal = None;
    assert_eq!(budget.live_count(), 0);
    assert_eq!(objects.live_count(), 0);
}
