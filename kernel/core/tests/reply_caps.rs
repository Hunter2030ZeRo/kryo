use kryo_core::ipc::{Completion, Delivery, Endpoint, IpcError, Message, ReplyToken};
use kryo_core::sync::wait::{ParkResult, WaitError, WaitId, WaitTable};
use kryo_core::task::{TaskId, TaskTable};

type Waits = WaitTable<Completion, 8>;

struct Fixture {
    _tasks: TaskTable<(), 3>,
    owner: TaskId,
    caller: TaskId,
    intruder: TaskId,
    waits: Waits,
    endpoint: Endpoint<2, 1>,
    wait: WaitId,
    token: ReplyToken,
}

#[test]
fn direct_and_queued_exchanges_preserve_both_capability_counts() {
    for queued in [false, true] {
        for request_count in 0..=2 {
            for capacity in 0..=2 {
                for reply_count in 0..=capacity {
                    let mut fixture = accepted(request_count, capacity, queued, false, true);
                    assert_eq!(
                        fixture.endpoint.reply_target(
                            fixture.owner,
                            fixture.token,
                            reply_count,
                            &fixture.waits,
                        ),
                        Ok((fixture.caller, fixture.wait)),
                    );
                    fixture
                        .endpoint
                        .reply_caps(
                            fixture.owner,
                            fixture.token,
                            response(),
                            reply_count,
                            &mut fixture.waits,
                        )
                        .unwrap();
                    assert_eq!(
                        fixture.waits.take(fixture.wait),
                        Ok(Completion::RepliedCaps {
                            message: response(),
                            sender: fixture.owner,
                            capabilities: reply_count,
                        }),
                    );
                    assert_eq!(
                        fixture.endpoint.reply_caps(
                            fixture.owner,
                            fixture.token,
                            Message::default(),
                            reply_count,
                            &mut fixture.waits,
                        ),
                        Err(IpcError::InvalidReply),
                    );
                }
            }
        }
    }
}

#[test]
fn insufficient_reply_capacity_keeps_the_token_available_for_retry() {
    for capacity in 0..=1 {
        let mut fixture = accepted(2, capacity, false, false, true);
        assert_eq!(
            fixture.endpoint.reply_caps(
                fixture.owner,
                fixture.token,
                response(),
                capacity + 1,
                &mut fixture.waits,
            ),
            Err(IpcError::Capacity),
        );
        assert!(fixture.waits.is_pending(fixture.wait));
        assert_eq!(
            fixture
                .endpoint
                .reply_target(fixture.owner, fixture.token, capacity, &fixture.waits,),
            Ok((fixture.caller, fixture.wait)),
        );
        fixture
            .endpoint
            .reply_caps(
                fixture.owner,
                fixture.token,
                response(),
                capacity,
                &mut fixture.waits,
            )
            .unwrap();
        assert_eq!(
            fixture.waits.take(fixture.wait),
            Ok(Completion::RepliedCaps {
                message: response(),
                sender: fixture.owner,
                capabilities: capacity,
            }),
        );
    }
}

#[test]
fn plain_replies_preserve_legacy_completion_and_consume_exchange_reservations() {
    for legacy in [false, true] {
        let mut fixture = accepted(0, 0, false, legacy, true);
        assert_eq!(
            fixture
                .endpoint
                .reply_target(fixture.owner, fixture.token, 1, &fixture.waits,),
            Err(IpcError::Capacity),
        );
        fixture
            .endpoint
            .reply(fixture.owner, fixture.token, response(), &mut fixture.waits)
            .unwrap();
        let expected = if legacy {
            Completion::Replied(response())
        } else {
            Completion::RepliedCaps {
                message: response(),
                sender: fixture.owner,
                capabilities: 0,
            }
        };
        assert_eq!(fixture.waits.take(fixture.wait), Ok(expected));
    }
}

#[test]
fn oversized_request_and_reply_capacities_do_not_register_a_call() {
    for (request_count, capacity) in [(3, 0), (0, 3), (usize::MAX, 2), (2, usize::MAX)] {
        let mut tasks = TaskTable::<(), 2>::new();
        let owner = tasks.spawn(()).unwrap();
        let caller = tasks.spawn(()).unwrap();
        let mut waits = Waits::new();
        let mut endpoint = Endpoint::<1, 1>::new(owner);
        let receive = blocked(&mut waits, owner);
        endpoint
            .receive_caps(owner, receive, 2, &mut waits)
            .unwrap();
        let call = blocked(&mut waits, caller);
        assert_eq!(
            endpoint.call_exchange(caller, call, request(), request_count, capacity, &mut waits),
            Err(IpcError::Capacity),
        );
        assert!(waits.is_pending(receive));
        assert!(waits.is_pending(call));
        assert_eq!(
            endpoint.cancel(call, &mut waits),
            Err(IpcError::InvalidWait)
        );
        endpoint
            .call_exchange(caller, call, request(), 2, 2, &mut waits)
            .unwrap();
        let token = received(&mut waits, receive, caller, call, 2);
        endpoint
            .reply_caps(owner, token, response(), 2, &mut waits)
            .unwrap();
        assert_eq!(
            waits.take(call),
            Ok(Completion::RepliedCaps {
                message: response(),
                sender: owner,
                capabilities: 2
            }),
        );
    }
}

#[test]
fn reply_preflight_does_not_consume_tokens_on_permission_or_count_failure() {
    let mut fixture = accepted(0, 2, false, false, true);
    assert_eq!(
        fixture
            .endpoint
            .reply_target(fixture.intruder, fixture.token, 1, &fixture.waits),
        Err(IpcError::PermissionDenied),
    );
    assert_eq!(
        fixture
            .endpoint
            .reply_target(fixture.owner, ReplyToken::from_raw(0), 1, &fixture.waits),
        Err(IpcError::InvalidReply),
    );
    assert_eq!(
        fixture
            .endpoint
            .reply_target(fixture.owner, fixture.token, usize::MAX, &fixture.waits),
        Err(IpcError::Capacity),
    );
    assert!(fixture.waits.is_pending(fixture.wait));
    fixture
        .endpoint
        .reply_caps(
            fixture.owner,
            fixture.token,
            response(),
            2,
            &mut fixture.waits,
        )
        .unwrap();
    assert!(matches!(
        fixture.waits.take(fixture.wait),
        Ok(Completion::RepliedCaps {
            capabilities: 2,
            ..
        })
    ));
}

#[test]
fn accepted_exchange_failures_keep_delivery_state_and_reject_late_replies() {
    for cause in 0..5 {
        let mut fixture = accepted(2, 2, false, false, true);
        let (error, reply_error) = match cause {
            0 => {
                assert_eq!(
                    fixture.endpoint.cancel(fixture.wait, &mut fixture.waits),
                    Ok(Delivery::Accepted)
                );
                (IpcError::Cancelled, IpcError::InvalidReply)
            }
            1 => {
                assert_eq!(
                    fixture.endpoint.timeout(fixture.wait, &mut fixture.waits),
                    Ok(Delivery::Accepted)
                );
                (IpcError::TimedOut, IpcError::InvalidReply)
            }
            2 => {
                fixture
                    .endpoint
                    .task_died(fixture.caller, &mut fixture.waits);
                (IpcError::PeerClosed, IpcError::InvalidReply)
            }
            3 => {
                fixture
                    .endpoint
                    .task_died(fixture.owner, &mut fixture.waits);
                (IpcError::PeerClosed, IpcError::PeerClosed)
            }
            _ => {
                fixture.endpoint.close(&mut fixture.waits);
                (IpcError::Closed, IpcError::Closed)
            }
        };
        assert_eq!(
            fixture.waits.take(fixture.wait),
            Ok(Completion::Failed {
                error,
                delivery: Delivery::Accepted
            }),
        );
        assert_eq!(
            fixture.endpoint.reply_caps(
                fixture.owner,
                fixture.token,
                response(),
                2,
                &mut fixture.waits
            ),
            Err(reply_error),
        );
    }
}

#[test]
fn abandoned_caller_wait_cannot_redirect_a_reply_into_its_reused_slot() {
    let mut fixture = accepted(0, 2, false, false, true);
    // Occupy the earlier receive slot so this allocation reuses the caller slot.
    let _earlier = blocked(&mut fixture.waits, fixture.intruder);
    fixture.waits.abandon(fixture.wait).unwrap();
    let replacement = blocked(&mut fixture.waits, fixture.caller);
    assert_eq!(fixture.wait.as_raw() as u32, replacement.as_raw() as u32);
    assert_ne!(fixture.wait, replacement);
    assert_eq!(
        fixture
            .endpoint
            .reply_target(fixture.owner, fixture.token, 2, &fixture.waits),
        Err(IpcError::InvalidReply),
    );
    assert!(fixture.waits.is_pending(replacement));
    assert_eq!(fixture.waits.take(replacement), Err(WaitError::NotReady));
}

#[test]
fn reply_during_parking_stays_hidden_until_the_caller_handoff() {
    let mut fixture = accepted(0, 1, false, false, false);
    fixture
        .endpoint
        .reply_caps(
            fixture.owner,
            fixture.token,
            response(),
            1,
            &mut fixture.waits,
        )
        .unwrap();
    assert_eq!(fixture.waits.take(fixture.wait), Err(WaitError::NotReady));
    assert_eq!(fixture.waits.park(fixture.wait), Ok(ParkResult::Ready));
    assert_eq!(
        fixture.waits.take(fixture.wait),
        Ok(Completion::RepliedCaps {
            message: response(),
            sender: fixture.owner,
            capabilities: 1
        }),
    );
}

#[test]
fn releasing_a_reply_slot_preserves_the_next_exchange_capacity() {
    let mut fixture = accepted(0, 1, false, false, true);
    let next_receive = blocked(&mut fixture.waits, fixture.owner);
    fixture
        .endpoint
        .receive(fixture.owner, next_receive, &mut fixture.waits)
        .unwrap();
    let next_call = blocked(&mut fixture.waits, fixture.intruder);
    fixture
        .endpoint
        .call_exchange(
            fixture.intruder,
            next_call,
            request(),
            0,
            2,
            &mut fixture.waits,
        )
        .unwrap();
    assert!(fixture.waits.is_pending(next_receive));
    fixture
        .endpoint
        .reply_caps(
            fixture.owner,
            fixture.token,
            response(),
            1,
            &mut fixture.waits,
        )
        .unwrap();
    let token = received(
        &mut fixture.waits,
        next_receive,
        fixture.intruder,
        next_call,
        0,
    );
    assert_ne!(fixture.token, token);
    assert_eq!(
        fixture.endpoint.reply_caps(
            fixture.owner,
            fixture.token,
            response(),
            1,
            &mut fixture.waits
        ),
        Err(IpcError::InvalidReply),
    );
    fixture
        .endpoint
        .reply_caps(fixture.owner, token, response(), 2, &mut fixture.waits)
        .unwrap();
    assert_eq!(
        fixture.waits.take(next_call),
        Ok(Completion::RepliedCaps {
            message: response(),
            sender: fixture.owner,
            capabilities: 2
        }),
    );
}

fn accepted(
    request_count: usize,
    reply_capacity: usize,
    queued: bool,
    legacy: bool,
    parked: bool,
) -> Fixture {
    let mut tasks = TaskTable::<(), 3>::new();
    let [owner, caller, intruder] = core::array::from_fn(|_| tasks.spawn(()).unwrap());
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<2, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    let call = waits.begin(caller).unwrap();
    if parked {
        assert_eq!(waits.park(call), Ok(ParkResult::Blocked));
    }
    if !queued {
        endpoint
            .receive_caps(owner, receive, request_count, &mut waits)
            .unwrap();
    }
    if legacy {
        endpoint
            .call_caps(caller, call, request(), request_count, &mut waits)
            .unwrap();
    } else {
        endpoint
            .call_exchange(
                caller,
                call,
                request(),
                request_count,
                reply_capacity,
                &mut waits,
            )
            .unwrap();
    }
    if queued {
        assert!(waits.is_pending(call));
        endpoint
            .receive_caps(owner, receive, request_count, &mut waits)
            .unwrap();
    }
    let token = received(&mut waits, receive, caller, call, request_count);
    Fixture {
        _tasks: tasks,
        owner,
        caller,
        intruder,
        waits,
        endpoint,
        wait: call,
        token,
    }
}

fn blocked(waits: &mut Waits, task: TaskId) -> WaitId {
    let wait = waits.begin(task).unwrap();
    assert_eq!(waits.park(wait), Ok(ParkResult::Blocked));
    wait
}

fn received(
    waits: &mut Waits,
    wait: WaitId,
    caller: TaskId,
    call: WaitId,
    count: usize,
) -> ReplyToken {
    match waits.take(wait).unwrap() {
        Completion::Received {
            message,
            reply: Some(token),
            sender,
            sender_wait,
            capabilities,
        } => {
            assert_eq!(
                (message, sender, sender_wait, capabilities),
                (request(), caller, call, count)
            );
            token
        }
        other => panic!("Missing accepted exchange: {other:?}"),
    }
}

fn request() -> Message {
    Message([0xfedc_ba98_7654_3210, 2, 3, 4])
}

fn response() -> Message {
    Message([42, 0x8000_0000_0000_0000, 7, 11])
}
