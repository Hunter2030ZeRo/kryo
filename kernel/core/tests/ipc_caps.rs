use kryo_core::ipc::{
    Completion, Delivery, Endpoint, IpcError, MAX_CAPABILITIES, Message, ReplyToken,
};
use kryo_core::sync::wait::{ParkResult, WaitId, WaitTable};
use kryo_core::task::{TaskId, TaskTable};

#[test]
fn direct_and_queued_messages_keep_sender_and_capability_metadata() {
    assert_eq!(size_of::<Message>(), 32);
    assert_eq!(MAX_CAPABILITIES, 2);
    for count in 0..=MAX_CAPABILITIES {
        for capacity in count..=MAX_CAPABILITIES {
            for call in [false, true] {
                for queued in [false, true] {
                    let (_tasks, [owner, sender, _, _]) = tasks();
                    let mut waits = Waits::new();
                    let mut endpoint = Endpoint::<1, 1>::new(owner);
                    let receive = blocked(&mut waits, owner);
                    let send = blocked(&mut waits, sender);
                    let message = Message([41, 7, 9, 11]);
                    if !queued {
                        endpoint
                            .receive_caps(owner, receive, capacity, &mut waits)
                            .unwrap();
                    }
                    if call {
                        endpoint
                            .call_caps(sender, send, message, count, &mut waits)
                            .unwrap();
                    } else {
                        endpoint
                            .send_caps(sender, send, message, count, &mut waits)
                            .unwrap();
                    }
                    if queued {
                        assert!(waits.is_pending(send));
                        endpoint
                            .receive_caps(owner, receive, capacity, &mut waits)
                            .unwrap();
                    }
                    let reply = received(&mut waits, receive, message, sender, send, count);
                    assert_eq!(reply.is_some(), call);
                    if let Some(reply) = reply {
                        assert!(waits.is_pending(send));
                        endpoint
                            .reply(owner, reply, Message([42; 4]), &mut waits)
                            .unwrap();
                        assert_eq!(waits.take(send), Ok(Completion::Replied(Message([42; 4]))));
                    } else {
                        assert_eq!(waits.take(send), Ok(Completion::Sent));
                    }
                }
            }
        }
    }
}

#[test]
fn direct_capacity_rejection_keeps_both_waits_and_reply_slots_unchanged() {
    for capacity in 0..MAX_CAPABILITIES {
        for count in capacity + 1..=MAX_CAPABILITIES {
            for call in [false, true] {
                let (_tasks, [owner, sender, _, _]) = tasks();
                let mut waits = Waits::new();
                let mut endpoint = Endpoint::<1, 1>::new(owner);
                let receive = blocked(&mut waits, owner);
                endpoint
                    .receive_caps(owner, receive, capacity, &mut waits)
                    .unwrap();
                let send = blocked(&mut waits, sender);
                let result = if call {
                    endpoint.call_caps(sender, send, Message([1; 4]), count, &mut waits)
                } else {
                    endpoint.send_caps(sender, send, Message([1; 4]), count, &mut waits)
                };
                assert_eq!(result, Err(IpcError::Capacity));
                assert!(waits.is_pending(receive));
                assert!(waits.is_pending(send));
                assert_eq!(
                    endpoint.cancel(send, &mut waits),
                    Err(IpcError::InvalidWait)
                );
                if call {
                    endpoint
                        .call_caps(sender, send, Message([2; 4]), capacity, &mut waits)
                        .unwrap();
                } else {
                    endpoint
                        .send_caps(sender, send, Message([2; 4]), capacity, &mut waits)
                        .unwrap();
                }
                let token = received(&mut waits, receive, Message([2; 4]), sender, send, capacity);
                assert_eq!(token.is_some(), call);
                endpoint.close(&mut waits);
            }
        }
    }
}

#[test]
fn direct_oversized_call_is_rejected_even_while_another_reply_is_pending() {
    let (_tasks, [owner, first, second, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let accepted = blocked(&mut waits, first);
    endpoint
        .call(first, accepted, Message::default(), &mut waits)
        .unwrap();
    let token = received(&mut waits, receive, Message::default(), first, accepted, 0).unwrap();

    let receive = blocked(&mut waits, owner);
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let rejected = blocked(&mut waits, second);
    assert_eq!(
        endpoint.call_caps(second, rejected, Message::default(), 1, &mut waits),
        Err(IpcError::Capacity)
    );
    assert!(waits.is_pending(receive));
    assert!(waits.is_pending(rejected));
    assert_eq!(
        endpoint.cancel(rejected, &mut waits),
        Err(IpcError::InvalidWait)
    );
    endpoint
        .reply(owner, token, Message::default(), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(accepted),
        Ok(Completion::Replied(Message::default()))
    );
    endpoint
        .send(second, rejected, Message([2; 4]), &mut waits)
        .unwrap();
    received(&mut waits, receive, Message([2; 4]), second, rejected, 0);
    assert_eq!(waits.take(rejected), Ok(Completion::Sent));
}

#[test]
fn out_of_range_capability_counts_do_not_register_requests_or_receivers() {
    for count in [MAX_CAPABILITIES + 1, usize::MAX] {
        let (_tasks, [owner, sender, _, _]) = tasks();
        let mut waits = Waits::new();
        let mut endpoint = Endpoint::<1, 1>::new(owner);
        let receive = blocked(&mut waits, owner);
        assert_eq!(
            endpoint.receive_caps(owner, receive, count, &mut waits),
            Err(IpcError::Capacity)
        );
        assert!(waits.is_pending(receive));
        assert_eq!(
            endpoint.cancel(receive, &mut waits),
            Err(IpcError::InvalidWait)
        );
        endpoint
            .receive_caps(owner, receive, 2, &mut waits)
            .unwrap();
        let send = blocked(&mut waits, sender);
        assert_eq!(
            endpoint.send_caps(sender, send, Message::default(), count, &mut waits),
            Err(IpcError::Capacity)
        );
        assert_eq!(
            endpoint.call_caps(sender, send, Message::default(), count, &mut waits),
            Err(IpcError::Capacity)
        );
        assert!(waits.is_pending(receive));
        assert!(waits.is_pending(send));
        endpoint
            .send_caps(sender, send, Message::default(), 2, &mut waits)
            .unwrap();
        received(&mut waits, receive, Message::default(), sender, send, 2);
        assert_eq!(waits.take(send), Ok(Completion::Sent));
    }
}

#[test]
fn queued_capacity_rejection_preserves_fifo_order_for_matching_senders() {
    for capacity in 0..MAX_CAPABILITIES {
        for call in [false, true] {
            let (_tasks, [owner, rejected, first, second]) = tasks();
            let mut waits = Waits::new();
            let mut endpoint = Endpoint::<3, 1>::new(owner);
            let rejected_wait = blocked(&mut waits, rejected);
            if call {
                endpoint
                    .call_caps(
                        rejected,
                        rejected_wait,
                        Message([1; 4]),
                        capacity + 1,
                        &mut waits,
                    )
                    .unwrap();
            } else {
                endpoint
                    .send_caps(
                        rejected,
                        rejected_wait,
                        Message([1; 4]),
                        capacity + 1,
                        &mut waits,
                    )
                    .unwrap();
            }
            let first_wait = blocked(&mut waits, first);
            endpoint
                .send_caps(first, first_wait, Message([2; 4]), capacity, &mut waits)
                .unwrap();
            let second_wait = blocked(&mut waits, second);
            endpoint
                .send(second, second_wait, Message([3; 4]), &mut waits)
                .unwrap();

            let receive = blocked(&mut waits, owner);
            endpoint
                .receive_caps(owner, receive, capacity, &mut waits)
                .unwrap();
            assert_eq!(waits.take(rejected_wait), Ok(capacity_failure()));
            received(
                &mut waits,
                receive,
                Message([2; 4]),
                first,
                first_wait,
                capacity,
            );
            assert_eq!(waits.take(first_wait), Ok(Completion::Sent));
            assert!(waits.is_pending(second_wait));
            let receive = blocked(&mut waits, owner);
            endpoint.receive(owner, receive, &mut waits).unwrap();
            received(&mut waits, receive, Message([3; 4]), second, second_wait, 0);
            assert_eq!(waits.take(second_wait), Ok(Completion::Sent));
        }
    }
}

#[test]
fn rejecting_the_last_queued_sender_keeps_the_receiver_waiting() {
    let (_tasks, [owner, sender, _, _]) = tasks();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let send = blocked(&mut waits, sender);
    endpoint
        .call_caps(sender, send, Message::default(), 2, &mut waits)
        .unwrap();
    let receive = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, receive, 1, &mut waits)
        .unwrap();
    assert_eq!(waits.take(send), Ok(capacity_failure()));
    assert!(waits.is_pending(receive));
    let retry = blocked(&mut waits, sender);
    endpoint
        .call_caps(sender, retry, Message([2; 4]), 1, &mut waits)
        .unwrap();
    let token = received(&mut waits, receive, Message([2; 4]), sender, retry, 1).unwrap();
    endpoint
        .reply(owner, token, Message::default(), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(retry),
        Ok(Completion::Replied(Message::default()))
    );
}

#[test]
fn cancelling_or_losing_queued_senders_never_accepts_their_capabilities() {
    for call in [false, true] {
        for death in [false, true] {
            let (_tasks, [owner, sender, next, _]) = tasks();
            let mut waits = Waits::new();
            let mut endpoint = Endpoint::<1, 1>::new(owner);
            let send = blocked(&mut waits, sender);
            if call {
                endpoint
                    .call_caps(sender, send, Message([1; 4]), 2, &mut waits)
                    .unwrap();
            } else {
                endpoint
                    .send_caps(sender, send, Message([1; 4]), 2, &mut waits)
                    .unwrap();
            }
            if death {
                endpoint.task_died(sender, &mut waits);
            } else {
                assert_eq!(
                    endpoint.cancel(send, &mut waits),
                    Ok(Delivery::NotDelivered)
                );
            }
            assert_eq!(
                waits.take(send),
                Ok(Completion::Failed {
                    error: if death {
                        IpcError::PeerClosed
                    } else {
                        IpcError::Cancelled
                    },
                    delivery: Delivery::NotDelivered,
                })
            );
            let receive = blocked(&mut waits, owner);
            endpoint
                .receive_caps(owner, receive, 2, &mut waits)
                .unwrap();
            assert!(waits.is_pending(receive));
            let send = blocked(&mut waits, next);
            endpoint
                .send_caps(next, send, Message([2; 4]), 1, &mut waits)
                .unwrap();
            received(&mut waits, receive, Message([2; 4]), next, send, 1);
            assert_eq!(waits.take(send), Ok(Completion::Sent));
        }
    }
}

#[test]
fn accepted_capability_calls_keep_delivery_status_after_cancellation_or_death() {
    for death in [false, true] {
        let (_tasks, [owner, sender, _, _]) = tasks();
        let mut waits = Waits::new();
        let mut endpoint = Endpoint::<1, 1>::new(owner);
        let receive = blocked(&mut waits, owner);
        endpoint
            .receive_caps(owner, receive, 2, &mut waits)
            .unwrap();
        let send = blocked(&mut waits, sender);
        endpoint
            .call_caps(sender, send, Message([1; 4]), 2, &mut waits)
            .unwrap();
        let token = received(&mut waits, receive, Message([1; 4]), sender, send, 2).unwrap();
        if death {
            endpoint.task_died(sender, &mut waits);
        } else {
            assert_eq!(endpoint.cancel(send, &mut waits), Ok(Delivery::Accepted));
        }
        assert_eq!(
            waits.take(send),
            Ok(Completion::Failed {
                error: if death {
                    IpcError::PeerClosed
                } else {
                    IpcError::Cancelled
                },
                delivery: Delivery::Accepted,
            })
        );
        assert_eq!(
            endpoint.reply(owner, token, Message::default(), &mut waits),
            Err(IpcError::InvalidReply)
        );
        assert!(!endpoint.is_closed());
    }
}

#[test]
fn cancelled_or_stale_receivers_do_not_leave_their_capacity_behind() {
    for abandoned in [false, true] {
        let (_tasks, [owner, sender, _, _]) = tasks();
        let mut waits = Waits::new();
        let mut endpoint = Endpoint::<1, 1>::new(owner);
        let old_receive = blocked(&mut waits, owner);
        endpoint
            .receive_caps(owner, old_receive, 2, &mut waits)
            .unwrap();
        if abandoned {
            waits.abandon(old_receive).unwrap();
        } else {
            assert_eq!(
                endpoint.cancel(old_receive, &mut waits),
                Ok(Delivery::NotDelivered)
            );
            assert_eq!(
                waits.take(old_receive),
                Ok(Completion::Failed {
                    error: IpcError::Cancelled,
                    delivery: Delivery::NotDelivered,
                })
            );
        }
        let receive = blocked(&mut waits, owner);
        endpoint.receive(owner, receive, &mut waits).unwrap();
        let send = blocked(&mut waits, sender);
        assert_eq!(
            endpoint.send_caps(sender, send, Message::default(), 1, &mut waits),
            Err(IpcError::Capacity)
        );
        assert!(waits.is_pending(receive));
        endpoint
            .send(sender, send, Message::default(), &mut waits)
            .unwrap();
        received(&mut waits, receive, Message::default(), sender, send, 0);
        assert_eq!(waits.take(send), Ok(Completion::Sent));
    }
}

type Waits = WaitTable<Completion, 8>;

fn tasks() -> (TaskTable<(), 4>, [TaskId; 4]) {
    let mut tasks = TaskTable::new();
    let ids = core::array::from_fn(|_| tasks.spawn(()).unwrap());
    (tasks, ids)
}

fn blocked(waits: &mut Waits, task: TaskId) -> WaitId {
    let wait = waits.begin(task).unwrap();
    assert_eq!(waits.park(wait), Ok(ParkResult::Blocked));
    wait
}

fn capacity_failure() -> Completion {
    Completion::Failed {
        error: IpcError::Capacity,
        delivery: Delivery::NotDelivered,
    }
}

fn received(
    waits: &mut Waits,
    wait: WaitId,
    message: Message,
    sender: TaskId,
    sender_wait: WaitId,
    capabilities: usize,
) -> Option<ReplyToken> {
    match waits.take(wait).unwrap() {
        Completion::Received {
            message: actual_message,
            reply,
            sender: actual_sender,
            sender_wait: actual_wait,
            capabilities: actual_count,
        } => {
            assert_eq!(
                (actual_message, actual_sender, actual_wait, actual_count),
                (message, sender, sender_wait, capabilities)
            );
            reply
        }
        other => panic!("expected Received, got {other:?}"),
    }
}
