use kryo_core::ipc::{Completion, Endpoint, GateKey, IpcError, LabeledMessage, Message};
use kryo_core::sync::wait::{WaitId, WaitTable};
use kryo_core::task::{TaskId, TaskTable};

type Waits = WaitTable<Completion, 4>;
fn blocked(waits: &mut Waits, task: TaskId) -> WaitId {
    let wait = waits.begin(task).unwrap();
    waits.park(wait).unwrap();
    wait
}
fn setup() -> (TaskTable<(), 3>, [TaskId; 3]) {
    let mut tasks = TaskTable::new();
    let ids = [
        tasks.spawn(()).unwrap(),
        tasks.spawn(()).unwrap(),
        tasks.spawn(()).unwrap(),
    ];
    (tasks, ids)
}
#[test]
fn gated_call_preserves_label_words_grants_and_reply_contract() {
    let (_tasks, [owner, sender, other]) = setup();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<2, 2>::new(owner);
    let label = 0xfedc_ba98_7654_3210;
    let key = GateKey {
        registry: 3,
        object: 0x8000_0001_0000_0004,
    };
    assert_eq!(
        endpoint.reserve_gate(other, label, key),
        Err(IpcError::PermissionDenied)
    );
    let event = endpoint.reserve_gate(owner, label, key).unwrap();
    assert_eq!(
        endpoint.reserve_gate(owner, label, key),
        Err(IpcError::AlreadyWaiting)
    );
    let call = blocked(&mut waits, sender);
    endpoint
        .call_labeled(
            sender,
            call,
            LabeledMessage {
                message: Message([u64::MAX; 4]),
                label,
            },
            2,
            Some(1),
            &mut waits,
        )
        .unwrap();
    let receive = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, receive, 2, &mut waits)
        .unwrap();
    let token = match waits.take(receive).unwrap() {
        Completion::Received {
            label: actual,
            message,
            reply: Some(token),
            capabilities: 2,
            ..
        } => {
            assert_eq!(actual, label);
            assert_eq!(message, Message([u64::MAX; 4]));
            token
        }
        actual => panic!("Unexpected completion {actual:?}"),
    };
    endpoint
        .reply_caps(owner, token, Message([42; 4]), 1, &mut waits)
        .unwrap();
    assert!(matches!(
        waits.take(call),
        Ok(Completion::RepliedCaps {
            capabilities: 1,
            ..
        })
    ));
    endpoint.close_gate(event, key, &mut waits);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::GateClosed { label, key })
    );
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert!(waits.is_pending(receive), "Closure is one shot");
    endpoint.close(&mut waits);
    assert!(matches!(waits.take(receive), Ok(Completion::Failed { .. })));
}
#[test]
fn pending_closure_alternates_with_messages_and_plain_senders_stamp_zero() {
    let (_tasks, [owner, sender, _]) = setup();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<2, 1>::new(owner);
    let key = GateKey {
        registry: 0,
        object: 9,
    };
    let event = endpoint.reserve_gate(owner, 7, key).unwrap();
    let send = blocked(&mut waits, sender);
    endpoint
        .send(sender, send, Message([1; 4]), &mut waits)
        .unwrap();
    endpoint.close_gate(event, key, &mut waits);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::GateClosed { label: 7, key })
    );
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert!(matches!(
        waits.take(receive),
        Ok(Completion::Received { label: 0, .. })
    ));
    assert_eq!(waits.take(send), Ok(Completion::Sent));
}
#[test]
fn closed_root_discards_pending_events_and_rejects_new_gate_publication() {
    let (_tasks, [owner, _, _]) = setup();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let key = GateKey {
        registry: 0,
        object: 1,
    };
    let event = endpoint.reserve_gate(owner, 17, key).unwrap();
    endpoint.close_gate(event, key, &mut waits);
    endpoint.task_died(owner, &mut waits);
    assert_eq!(
        endpoint.reserve_gate(owner, 18, key),
        Err(IpcError::PeerClosed)
    );
    let receive = blocked(&mut waits, owner);
    assert_eq!(
        endpoint.receive(owner, receive, &mut waits),
        Err(IpcError::PeerClosed)
    );
    waits.abandon(receive).unwrap();
}

#[test]
fn reply_capacity_storage_never_truncates_public_usize_values() {
    let (_tasks, [owner, sender, _]) = setup();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let call = blocked(&mut waits, sender);
    for capacity in [3, 256, usize::MAX] {
        assert_eq!(
            endpoint.call_exchange(sender, call, Message([0; 4]), 0, capacity, &mut waits),
            Err(IpcError::Capacity)
        );
        assert_eq!(
            endpoint.call_labeled(
                sender,
                call,
                LabeledMessage {
                    message: Message([0; 4]),
                    label: 7
                },
                0,
                Some(capacity),
                &mut waits
            ),
            Err(IpcError::Capacity)
        );
        assert!(waits.is_pending(call));
    }
    waits.abandon(call).unwrap();
}

#[test]
fn closure_fifo_survives_multiple_events_and_vacant_slot_reuse() {
    let (_tasks, [owner, _, _]) = setup();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let mut slots = [0; 3];
    for (index, slot) in slots.iter_mut().enumerate() {
        *slot = endpoint
            .reserve_gate(
                owner,
                index as u64 + 1,
                GateKey {
                    registry: index,
                    object: index as u64 + 1,
                },
            )
            .unwrap();
    }
    for index in [2, 0, 1] {
        endpoint.close_gate(
            slots[index],
            GateKey {
                registry: index,
                object: index as u64 + 1,
            },
            &mut waits,
        );
    }
    for label in [3, 1, 2] {
        let receive = blocked(&mut waits, owner);
        endpoint.receive(owner, receive, &mut waits).unwrap();
        assert!(
            matches!(waits.take(receive), Ok(Completion::GateClosed { label: actual, .. }) if actual == label)
        );
    }
    let key = GateKey {
        registry: 7,
        object: 8,
    };
    let reused = endpoint.reserve_gate(owner, 9, key).unwrap();
    endpoint.close_gate(reused, key, &mut waits);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::GateClosed { label: 9, key })
    );
}

#[test]
fn reply_slot_pressure_cannot_block_an_independent_gate_event() {
    let (_tasks, [owner, first, second]) = setup();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<2, 1>::new(owner);
    let first_call = blocked(&mut waits, first);
    endpoint
        .call(first, first_call, Message([1; 4]), &mut waits)
        .unwrap();
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let token = match waits.take(receive).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!(),
    };
    let key = GateKey {
        registry: 0,
        object: 1,
    };
    let event = endpoint.reserve_gate(owner, 5, key).unwrap();
    endpoint.close_gate(event, key, &mut waits);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::GateClosed { label: 5, key })
    );
    let queued = blocked(&mut waits, second);
    endpoint
        .call(second, queued, Message([2; 4]), &mut waits)
        .unwrap();
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert!(waits.is_pending(receive));
    let key = GateKey {
        registry: 1,
        object: 2,
    };
    let event = endpoint.reserve_gate(owner, 6, key).unwrap();
    endpoint.close_gate(event, key, &mut waits);
    assert_eq!(
        waits.take(receive),
        Ok(Completion::GateClosed { label: 6, key })
    );
    assert!(waits.is_pending(queued));
    endpoint
        .reply(owner, token, Message([42; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(first_call),
        Ok(Completion::Replied(Message([42; 4])))
    );
    endpoint.close(&mut waits);
    waits.take(queued).unwrap();
}
