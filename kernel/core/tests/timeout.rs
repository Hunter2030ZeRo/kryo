use kryo_core::ipc::{Completion, Delivery, Endpoint, IpcError, Message, Notification};
use kryo_core::sync::{ParkResult, WaitId, WaitTable};
use kryo_core::task::{TaskId, TaskTable};

type Waits = WaitTable<Completion, 3>;

#[test]
fn queued_sender_timeout_detaches_request_before_wait_reuse() {
    for call in [false, true] {
        let (_tasks, [owner, sender, _]) = tasks();
        let mut waits = Waits::new();
        let mut endpoint = Endpoint::<2, 1>::new(owner);
        let send = blocked(&mut waits, sender, 10);
        if call {
            endpoint.call(sender, send, Message([1; 4]), &mut waits)
        } else {
            endpoint.send(sender, send, Message([1; 4]), &mut waits)
        }
        .unwrap();
        assert_eq!(waits.next_expired(10), Some(send));
        assert_eq!(
            endpoint.timeout(send, &mut waits),
            Ok(Delivery::NotDelivered)
        );
        assert_eq!(waits.take(send), Ok(timed_out(Delivery::NotDelivered)));

        let receive = blocked(&mut waits, owner, 20);
        endpoint.receive(owner, receive, &mut waits).unwrap();
        assert!(waits.is_pending(receive));
        let next = blocked(&mut waits, sender, 30);
        endpoint
            .send(sender, next, Message([2; 4]), &mut waits)
            .unwrap();
        let Completion::Received {
            message,
            sender_wait,
            ..
        } = waits.take(receive).unwrap()
        else {
            panic!("new request expected");
        };
        assert_eq!(message, Message([2; 4]));
        assert_eq!(sender_wait, next);
        assert_eq!(waits.take(next), Ok(Completion::Sent));
        assert_eq!(
            endpoint.timeout(send, &mut waits),
            Err(IpcError::InvalidWait)
        );
    }
}

#[test]
fn receiver_timeout_does_not_consume_a_future_message() {
    let (_tasks, [owner, sender, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<2, 1>::new(owner);
    let old = blocked(&mut waits, owner, 10);
    endpoint.receive(owner, old, &mut waits).unwrap();
    assert_eq!(
        endpoint.timeout(old, &mut waits),
        Ok(Delivery::NotDelivered)
    );
    assert_eq!(waits.take(old), Ok(timed_out(Delivery::NotDelivered)));
    let send = blocked(&mut waits, sender, 20);
    endpoint
        .send(sender, send, Message([5; 4]), &mut waits)
        .unwrap();
    assert!(waits.is_pending(send));
    let receive = blocked(&mut waits, owner, 30);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    assert!(matches!(
        waits.take(receive),
        Ok(Completion::Received {
            message: Message([5, 5, 5, 5]),
            ..
        })
    ));
    assert_eq!(waits.take(send), Ok(Completion::Sent));
}

#[test]
fn accepted_timeout_retires_reply_token_and_allows_reuse() {
    let (_tasks, [owner, caller, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner, 100);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let call = blocked(&mut waits, caller, 10);
    endpoint
        .call(caller, call, Message([1; 4]), &mut waits)
        .unwrap();
    let Completion::Received {
        reply: Some(old_token),
        ..
    } = waits.take(receive).unwrap()
    else {
        panic!("accepted call expected");
    };
    assert_eq!(endpoint.timeout(call, &mut waits), Ok(Delivery::Accepted));
    assert_eq!(waits.take(call), Ok(timed_out(Delivery::Accepted)));
    assert_eq!(
        endpoint.reply(owner, old_token, Message([2; 4]), &mut waits),
        Err(IpcError::InvalidReply)
    );

    let receive = blocked(&mut waits, owner, 100);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let call = blocked(&mut waits, caller, 20);
    endpoint
        .call(caller, call, Message([3; 4]), &mut waits)
        .unwrap();
    let Completion::Received {
        reply: Some(token), ..
    } = waits.take(receive).unwrap()
    else {
        panic!("next call expected");
    };
    assert_ne!(old_token, token);
    assert_eq!(
        endpoint.reply(owner, old_token, Message([4; 4]), &mut waits),
        Err(IpcError::InvalidReply)
    );
    assert!(waits.is_pending(call));
    endpoint
        .reply(owner, token, Message([5; 4]), &mut waits)
        .unwrap();
    assert_eq!(waits.take(call), Ok(Completion::Replied(Message([5; 4]))));
}

#[test]
fn a_reply_that_wins_cannot_be_replaced_by_timeout() {
    let (_tasks, [owner, caller, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner, 100);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let call = blocked(&mut waits, caller, 10);
    endpoint
        .call(caller, call, Message([1; 4]), &mut waits)
        .unwrap();
    let Completion::Received {
        reply: Some(token), ..
    } = waits.take(receive).unwrap()
    else {
        panic!("accepted call expected");
    };
    endpoint
        .reply(owner, token, Message([9; 4]), &mut waits)
        .unwrap();
    assert_eq!(waits.next_deadline(), None);
    assert_eq!(waits.next_expired(u64::MAX), None);
    assert_eq!(
        endpoint.timeout(call, &mut waits),
        Err(IpcError::InvalidWait)
    );
    assert_eq!(waits.take(call), Ok(Completion::Replied(Message([9; 4]))));
}

#[test]
fn timeout_batch_cannot_accept_an_expired_request_when_a_reply_slot_opens() {
    for second_deadline in [20, 50] {
        let (_tasks, [owner, first, second]) = tasks();
        let mut waits = Waits::new();
        let mut endpoint = Endpoint::<1, 1>::new(owner);
        let receive = blocked(&mut waits, owner, 100);
        endpoint.receive(owner, receive, &mut waits).unwrap();
        let accepted = blocked(&mut waits, first, 10);
        endpoint
            .call(first, accepted, Message([1; 4]), &mut waits)
            .unwrap();
        assert!(matches!(
            waits.take(receive),
            Ok(Completion::Received { reply: Some(_), .. })
        ));
        let receive = blocked(&mut waits, owner, 100);
        endpoint.receive(owner, receive, &mut waits).unwrap();
        let queued = blocked(&mut waits, second, second_deadline);
        endpoint
            .call(second, queued, Message([2; 4]), &mut waits)
            .unwrap();
        assert!(waits.is_pending(receive));

        assert_eq!(waits.next_expired(30), Some(accepted));
        assert_eq!(
            endpoint.timeout(accepted, &mut waits),
            Ok(Delivery::Accepted)
        );
        assert!(waits.is_pending(receive));
        assert!(waits.is_pending(queued));
        if second_deadline <= 30 {
            assert_eq!(waits.next_expired(30), Some(queued));
            assert_eq!(
                endpoint.timeout(queued, &mut waits),
                Ok(Delivery::NotDelivered)
            );
        }
        assert_eq!(waits.next_expired(30), None);
        endpoint.progress(&mut waits);
        assert_eq!(waits.take(accepted), Ok(timed_out(Delivery::Accepted)));
        if second_deadline <= 30 {
            assert!(waits.is_pending(receive));
            assert_eq!(waits.take(queued), Ok(timed_out(Delivery::NotDelivered)));
        } else {
            let Completion::Received {
                reply: Some(token),
                sender,
                ..
            } = waits.take(receive).unwrap()
            else {
                panic!("unexpired request must progress after timeout batch");
            };
            assert_eq!(sender, second);
            endpoint
                .reply(owner, token, Message([3; 4]), &mut waits)
                .unwrap();
            assert_eq!(waits.take(queued), Ok(Completion::Replied(Message([3; 4]))));
        }
    }
}

#[test]
fn parking_timeout_waits_for_handoff() {
    let (_tasks, [owner, sender, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let wait = waits.begin_until(sender, 10).unwrap();
    endpoint
        .send(sender, wait, Message::default(), &mut waits)
        .unwrap();
    assert_eq!(
        endpoint.timeout(wait, &mut waits),
        Ok(Delivery::NotDelivered)
    );
    assert_eq!(waits.next_ready(), None);
    assert_eq!(waits.next_expired(10), None);
    assert_eq!(waits.park(wait), Ok(ParkResult::Ready));
    assert_eq!(waits.take(wait), Ok(timed_out(Delivery::NotDelivered)));
}

#[test]
fn signal_and_timeout_select_one_result_without_losing_the_next_signal() {
    for timeout_first in [false, true] {
        for parking in [false, true] {
            let (_tasks, [owner, _, _]) = tasks();
            let mut waits = Waits::new();
            let mut notification = Notification::new(owner);
            let wait = waits.begin_until(owner, 10).unwrap();
            if !parking {
                waits.park(wait).unwrap();
            }
            notification.wait(owner, wait, &mut waits).unwrap();
            if timeout_first {
                notification.timeout(wait, &mut waits).unwrap();
                notification.signal(&mut waits).unwrap();
            } else {
                notification.signal(&mut waits).unwrap();
                assert_eq!(
                    notification.timeout(wait, &mut waits),
                    Err(IpcError::InvalidWait)
                );
            }
            if parking {
                assert_eq!(waits.next_ready(), None);
                waits.park(wait).unwrap();
            }
            assert_eq!(waits.next_deadline(), None);
            assert_eq!(
                waits.take(wait),
                Ok(if timeout_first {
                    timed_out(Delivery::NotDelivered)
                } else {
                    Completion::Notified
                })
            );

            if timeout_first {
                let next = blocked(&mut waits, owner, 20);
                notification.wait(owner, next, &mut waits).unwrap();
                assert_eq!(waits.take(next), Ok(Completion::Notified));
            }
        }
    }
}

#[test]
fn cancellation_or_closure_before_timeout_keeps_its_result() {
    for close in [false, true] {
        let (_tasks, [owner, caller, _]) = tasks();
        let mut waits = Waits::new();
        let mut endpoint = Endpoint::<1, 1>::new(owner);
        let wait = blocked(&mut waits, caller, 10);
        endpoint
            .call(caller, wait, Message::default(), &mut waits)
            .unwrap();
        let error = if close {
            endpoint.close(&mut waits);
            IpcError::Closed
        } else {
            endpoint.cancel(wait, &mut waits).unwrap();
            IpcError::Cancelled
        };
        assert_eq!(
            endpoint.timeout(wait, &mut waits),
            Err(IpcError::InvalidWait)
        );
        assert_eq!(waits.next_deadline(), None);
        assert_eq!(
            waits.take(wait),
            Ok(Completion::Failed {
                error,
                delivery: Delivery::NotDelivered
            })
        );
    }
}

fn timed_out(delivery: Delivery) -> Completion {
    Completion::Failed {
        error: IpcError::TimedOut,
        delivery,
    }
}

fn blocked(waits: &mut Waits, task: TaskId, deadline: u64) -> WaitId {
    let wait = waits.begin_until(task, deadline).unwrap();
    assert_eq!(waits.park(wait), Ok(ParkResult::Blocked));
    wait
}

fn tasks() -> (TaskTable<(), 3>, [TaskId; 3]) {
    let mut tasks = TaskTable::new();
    let ids = core::array::from_fn(|_| tasks.spawn(()).unwrap());
    (tasks, ids)
}
