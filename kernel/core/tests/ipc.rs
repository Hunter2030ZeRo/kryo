use kryo_core::ipc::{Completion, Delivery, Endpoint, IpcError, Message, Notification};
use kryo_core::sync::wait::{ParkResult, WaitId, WaitTable};
use kryo_core::task::{TaskId, TaskTable};
#[test]
fn multiple_queued_senders_are_received_in_fifo_order() {
    let (_tasks, [owner, first, second, third]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<3, 1>::new(owner);
    let senders = [first, second, third];
    let mut ids = [None; 3];
    for (index, sender) in senders.into_iter().enumerate() {
        let wait = blocked(&mut waits, sender);
        endpoint
            .send(sender, wait, Message([index as u64; 4]), &mut waits)
            .unwrap();
        ids[index] = Some(wait);
    }
    for (index, id) in ids.into_iter().enumerate() {
        let receive = blocked(&mut waits, owner);
        endpoint.receive(owner, receive, &mut waits).unwrap();
        assert_eq!(
            waits.take(receive),
            Ok(Completion::Received {
                message: Message([index as u64; 4]),
                reply: None
            })
        );
        assert_eq!(waits.take(id.unwrap()), Ok(Completion::Sent));
    }
}

#[test]
fn accepted_caller_death_revokes_reply_without_closing_the_endpoint() {
    let (_tasks, [owner, caller, next, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let call = blocked(&mut waits, caller);
    endpoint
        .call(caller, call, Message::default(), &mut waits)
        .unwrap();
    let token = match waits.take(receive).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!("missing token"),
    };
    endpoint.task_died(caller, &mut waits);
    assert_eq!(
        waits.take(call),
        Ok(Completion::Failed {
            error: IpcError::PeerClosed,
            delivery: Delivery::Accepted
        })
    );
    assert!(!endpoint.is_closed());
    assert_eq!(
        endpoint.reply(owner, token, Message::default(), &mut waits),
        Err(IpcError::InvalidReply)
    );
    let next_wait = blocked(&mut waits, next);
    endpoint
        .call(next, next_wait, Message([2; 4]), &mut waits)
        .unwrap();
    let receiver = blocked(&mut waits, owner);
    endpoint.receive(owner, receiver, &mut waits).unwrap();
    let fresh = match waits.take(receiver).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!("missing fresh token"),
    };
    endpoint
        .reply(owner, fresh, Message([3; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(next_wait),
        Ok(Completion::Replied(Message([3; 4])))
    );
}

#[test]
fn zero_reply_capacity_rejects_calls_but_accepts_plain_rendezvous() {
    let (_tasks, [owner, sender, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<0, 0>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let send = blocked(&mut waits, sender);
    assert_eq!(
        endpoint.call(sender, send, Message::default(), &mut waits),
        Err(IpcError::Capacity)
    );
    assert!(waits.is_pending(receive));
    assert!(waits.is_pending(send));
    endpoint
        .send(sender, send, Message([1; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::Received {
            message: Message([1; 4]),
            reply: None
        })
    );
    assert_eq!(waits.take(send), Ok(Completion::Sent));
}

#[test]
fn malformed_reply_tokens_do_not_change_a_valid_call() {
    use kryo_core::ipc::ReplyToken;
    let (_tasks, [owner, caller, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let call = blocked(&mut waits, caller);
    endpoint
        .call(caller, call, Message::default(), &mut waits)
        .unwrap();
    let valid = match waits.take(receive).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!("missing token"),
    };
    for raw in [0, 1, 1 << 32, u64::MAX] {
        assert_eq!(
            endpoint.reply(
                owner,
                ReplyToken::from_raw(raw),
                Message::default(),
                &mut waits
            ),
            Err(IpcError::InvalidReply)
        );
        assert!(waits.is_pending(call));
    }
    endpoint
        .reply(owner, valid, Message([1; 4]), &mut waits)
        .unwrap();
    assert_eq!(waits.take(call), Ok(Completion::Replied(Message([1; 4]))));
}

type Waits = WaitTable<Completion, 8>;

fn tasks() -> (TaskTable<(), 4>, [TaskId; 4]) {
    let mut tasks = TaskTable::new();
    let ids = [
        tasks.spawn(()).unwrap(),
        tasks.spawn(()).unwrap(),
        tasks.spawn(()).unwrap(),
        tasks.spawn(()).unwrap(),
    ];
    (tasks, ids)
}

fn blocked(waits: &mut Waits, task: TaskId) -> WaitId {
    let id = waits.begin(task).unwrap();
    assert_eq!(waits.park(id), Ok(ParkResult::Blocked));
    id
}

#[test]
fn rendezvous_transfers_inline_data_and_completes_each_wait_once() {
    let (_tasks, [owner, sender, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let send = blocked(&mut waits, sender);
    endpoint
        .send(sender, send, Message([1, 2, 3, 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::Received {
            message: Message([1, 2, 3, 4]),
            reply: None
        })
    );
    assert_eq!(waits.take(send), Ok(Completion::Sent));
    assert!(waits.take(send).is_err());
    assert_eq!(waits.next_ready(), None);
}

#[test]
fn completion_before_park_is_not_published_as_an_early_wakeup() {
    let (_tasks, [owner, sender, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<0, 1>::new(owner);
    let receive = waits.begin(owner).unwrap();
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let send = waits.begin(sender).unwrap();
    endpoint
        .send(sender, send, Message([4; 4]), &mut waits)
        .unwrap();
    assert_eq!(waits.next_ready(), None);
    assert!(waits.take(receive).is_err());
    assert_eq!(waits.park(receive), Ok(ParkResult::Ready));
    assert_eq!(waits.park(send), Ok(ParkResult::Ready));
    assert!(matches!(
        waits.take(receive),
        Ok(Completion::Received { .. })
    ));
    assert_eq!(waits.take(send), Ok(Completion::Sent));
}

#[test]
fn queued_senders_keep_fifo_order_and_capacity_failure_does_not_register_a_wait() {
    let (_tasks, [owner, first, second, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let first_wait = blocked(&mut waits, first);
    endpoint
        .send(first, first_wait, Message([11; 4]), &mut waits)
        .unwrap();
    let second_wait = blocked(&mut waits, second);
    assert_eq!(
        endpoint.send(second, second_wait, Message([22; 4]), &mut waits),
        Err(IpcError::Capacity)
    );
    assert!(waits.is_pending(second_wait));
    waits.abandon(second_wait).unwrap();

    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::Received {
            message: Message([11; 4]),
            reply: None
        })
    );
    assert_eq!(waits.take(first_wait), Ok(Completion::Sent));
}

#[test]
fn owner_and_wait_identity_are_validated_before_registration() {
    let (_tasks, [owner, sender, other, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let wait = blocked(&mut waits, other);
    assert_eq!(
        endpoint.receive(other, wait, &mut waits),
        Err(IpcError::PermissionDenied)
    );
    assert_eq!(
        endpoint.send(sender, wait, Message::default(), &mut waits),
        Err(IpcError::InvalidWait)
    );
    assert!(waits.is_pending(wait));
    waits.abandon(wait).unwrap();

    let wait = blocked(&mut waits, owner);
    endpoint.receive(owner, wait, &mut waits).unwrap();
    assert_eq!(
        endpoint.send(owner, wait, Message::default(), &mut waits),
        Err(IpcError::AlreadyWaiting)
    );
    endpoint.close(&mut waits);
}

#[test]
fn call_waits_for_a_single_reply_from_the_bound_receiver() {
    let (_tasks, [owner, caller, intruder, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let call = blocked(&mut waits, caller);
    endpoint
        .call(caller, call, Message([9; 4]), &mut waits)
        .unwrap();
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let token = match waits.take(receive).unwrap() {
        Completion::Received {
            message: Message([9, 9, 9, 9]),
            reply: Some(token),
        } => token,
        other => panic!("{other:?}"),
    };
    assert!(waits.is_pending(call));
    assert_eq!(
        endpoint.reply(intruder, token, Message([33; 4]), &mut waits),
        Err(IpcError::PermissionDenied)
    );
    assert!(waits.is_pending(call));
    endpoint
        .reply(owner, token, Message([44; 4]), &mut waits)
        .unwrap();
    assert_eq!(waits.take(call), Ok(Completion::Replied(Message([44; 4]))));
    assert_eq!(
        endpoint.reply(owner, token, Message([55; 4]), &mut waits),
        Err(IpcError::InvalidReply)
    );
}

#[test]
fn cancelling_before_acceptance_prevents_delivery() {
    let (_tasks, [owner, caller, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let call = blocked(&mut waits, caller);
    endpoint
        .call(caller, call, Message([1; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        endpoint.cancel(call, &mut waits),
        Ok(Delivery::NotDelivered)
    );
    assert_eq!(
        waits.take(call),
        Ok(Completion::Failed {
            error: IpcError::Cancelled,
            delivery: Delivery::NotDelivered
        })
    );
    assert_eq!(
        endpoint.cancel(call, &mut waits),
        Err(IpcError::InvalidWait)
    );

    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert!(waits.is_pending(receive));
    let send = blocked(&mut waits, caller);
    endpoint
        .send(caller, send, Message([2; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::Received {
            message: Message([2; 4]),
            reply: None
        })
    );
    assert_eq!(waits.take(send), Ok(Completion::Sent));
}

#[test]
fn accepted_call_cancellation_reports_uncertainty_and_rejects_late_reply() {
    let (_tasks, [owner, caller, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let call = blocked(&mut waits, caller);
    endpoint
        .call(caller, call, Message::default(), &mut waits)
        .unwrap();
    let token = match waits.take(receive).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!("missing reply token"),
    };
    assert_eq!(endpoint.cancel(call, &mut waits), Ok(Delivery::Accepted));
    assert_eq!(
        waits.take(call),
        Ok(Completion::Failed {
            error: IpcError::Cancelled,
            delivery: Delivery::Accepted
        })
    );

    let fresh_call = blocked(&mut waits, caller);
    endpoint
        .call(caller, fresh_call, Message([2; 4]), &mut waits)
        .unwrap();
    let fresh_receive = blocked(&mut waits, owner);
    endpoint.receive(owner, fresh_receive, &mut waits).unwrap();
    let fresh_token = match waits.take(fresh_receive).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!("missing fresh token"),
    };
    assert_ne!(token, fresh_token);
    assert_eq!(
        endpoint.reply(owner, token, Message([1; 4]), &mut waits),
        Err(IpcError::InvalidReply)
    );
    assert!(waits.is_pending(fresh_call));
    endpoint
        .reply(owner, fresh_token, Message([3; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(fresh_call),
        Ok(Completion::Replied(Message([3; 4])))
    );
}

#[test]
fn endpoint_close_drains_receiver_queued_call_and_accepted_call_once() {
    let (_tasks, [owner, first, second, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<2, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let accepted = blocked(&mut waits, first);
    endpoint
        .call(first, accepted, Message([1; 4]), &mut waits)
        .unwrap();
    let token = match waits.take(receive).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!("missing token"),
    };
    let queued = blocked(&mut waits, second);
    endpoint
        .call(second, queued, Message([2; 4]), &mut waits)
        .unwrap();
    let receiver = blocked(&mut waits, owner);
    endpoint.receive(owner, receiver, &mut waits).unwrap();
    assert!(waits.is_pending(receiver));

    endpoint.close(&mut waits);
    endpoint.close(&mut waits);
    for (id, delivery) in [
        (accepted, Delivery::Accepted),
        (queued, Delivery::NotDelivered),
        (receiver, Delivery::NotDelivered),
    ] {
        assert_eq!(
            waits.take(id),
            Ok(Completion::Failed {
                error: IpcError::Closed,
                delivery
            })
        );
        assert!(waits.take(id).is_err());
    }
    assert!(endpoint.is_closed());
    assert_eq!(
        endpoint.reply(owner, token, Message::default(), &mut waits),
        Err(IpcError::Closed)
    );
}

#[test]
fn receiver_owner_death_closes_the_endpoint_and_sender_death_detaches_requests() {
    let (_tasks, [owner, caller, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let queued = blocked(&mut waits, caller);
    endpoint
        .send(caller, queued, Message::default(), &mut waits)
        .unwrap();
    endpoint.task_died(caller, &mut waits);
    assert_eq!(
        waits.take(queued),
        Ok(Completion::Failed {
            error: IpcError::PeerClosed,
            delivery: Delivery::NotDelivered
        })
    );
    assert!(!endpoint.is_closed());
    let receiver = blocked(&mut waits, owner);
    endpoint.receive(owner, receiver, &mut waits).unwrap();
    endpoint.task_died(owner, &mut waits);
    assert_eq!(
        waits.take(receiver),
        Ok(Completion::Failed {
            error: IpcError::PeerClosed,
            delivery: Delivery::NotDelivered
        })
    );
    assert!(endpoint.is_closed());
}

#[test]
fn completed_and_reused_wait_slots_cannot_receive_stale_queued_messages() {
    let (_tasks, [owner, caller, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let old = blocked(&mut waits, caller);
    endpoint
        .send(caller, old, Message([1; 4]), &mut waits)
        .unwrap();
    waits
        .complete(
            old,
            Completion::Failed {
                error: IpcError::Cancelled,
                delivery: Delivery::NotDelivered,
            },
        )
        .unwrap();
    waits.take(old).unwrap();
    let fresh = blocked(&mut waits, caller);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert!(waits.is_pending(receive));
    endpoint
        .send(caller, fresh, Message([2; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(receive),
        Ok(Completion::Received {
            message: Message([2; 4]),
            reply: None
        })
    );
    assert_eq!(waits.take(fresh), Ok(Completion::Sent));
}

#[test]
fn notification_coalesces_signals_without_losing_signal_before_wait() {
    let (_tasks, [owner, _, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut notification = Notification::new(owner);
    notification.signal(&mut waits).unwrap();
    notification.signal(&mut waits).unwrap();
    let first = blocked(&mut waits, owner);
    notification.wait(owner, first, &mut waits).unwrap();
    assert_eq!(waits.take(first), Ok(Completion::Notified));
    let second = blocked(&mut waits, owner);
    notification.wait(owner, second, &mut waits).unwrap();
    assert!(waits.is_pending(second));
    notification.signal(&mut waits).unwrap();
    notification.signal(&mut waits).unwrap();
    assert_eq!(waits.take(second), Ok(Completion::Notified));
    let third = blocked(&mut waits, owner);
    notification.wait(owner, third, &mut waits).unwrap();
    assert_eq!(waits.take(third), Ok(Completion::Notified));
}

#[test]
fn notification_cancellation_and_owner_death_complete_once() {
    let (_tasks, [owner, other, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut notification = Notification::new(owner);
    let wrong = blocked(&mut waits, other);
    assert_eq!(
        notification.wait(other, wrong, &mut waits),
        Err(IpcError::PermissionDenied)
    );
    waits.abandon(wrong).unwrap();
    let waiting = blocked(&mut waits, owner);
    notification.wait(owner, waiting, &mut waits).unwrap();
    notification.cancel(waiting, &mut waits).unwrap();
    assert_eq!(
        waits.take(waiting),
        Ok(Completion::Failed {
            error: IpcError::Cancelled,
            delivery: Delivery::NotDelivered
        })
    );
    notification.signal(&mut waits).unwrap();
    let fresh = blocked(&mut waits, owner);
    notification.wait(owner, fresh, &mut waits).unwrap();
    assert_eq!(waits.take(fresh), Ok(Completion::Notified));
    let dying = blocked(&mut waits, owner);
    notification.wait(owner, dying, &mut waits).unwrap();
    notification.task_died(owner, &mut waits);
    notification.close(&mut waits);
    assert_eq!(
        waits.take(dying),
        Ok(Completion::Failed {
            error: IpcError::PeerClosed,
            delivery: Delivery::NotDelivered
        })
    );
    assert_eq!(notification.signal(&mut waits), Err(IpcError::PeerClosed));
}
