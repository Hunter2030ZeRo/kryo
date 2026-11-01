use kryo_core::capability::{CapabilityError, HandleTable, Rights, TransferSpec};
use kryo_core::ipc::{Completion, Delivery, Endpoint, IpcError, Message, ReplyToken};
use kryo_core::object::{ObjectKind, ObjectTable};
use kryo_core::sync::wait::{ParkResult, WaitId, WaitTable};
use kryo_core::task::{TaskId, TaskTable};

#[test]
fn capacity_mismatch_retains_source_handles_and_staged_references() {
    for capacity in 0..2 {
        for queued in [false, true] {
            for call in [false, true] {
                let (_tasks, [owner, sender, _]) = tasks();
                let objects = ObjectTable::<u64, 1>::new();
                let memory = objects.create(ObjectKind::Memory, 41).unwrap();
                let mut sources = HandleTable::<u64, 1, 1>::new();
                let rights = Rights::READ | Rights::MAP | Rights::TRANSFER;
                let source = sources.insert(memory.try_clone().unwrap(), rights).unwrap();
                let batch = sources
                    .prepare_transfer(&[
                        TransferSpec {
                            handle: source,
                            rights: Rights::READ,
                        },
                        TransferSpec {
                            handle: source,
                            rights: Rights::MAP,
                        },
                    ])
                    .unwrap();
                assert_eq!(memory.reference_count(), 4);

                let mut receivers = HandleTable::<u64, 1, 2>::new();
                let reservation = receivers.reserve(capacity).unwrap();
                let mut waits = Waits::new();
                let mut endpoint = Endpoint::<1, 1>::new(owner);
                let receive = blocked(&mut waits, owner);
                let send = blocked(&mut waits, sender);
                if !queued {
                    endpoint
                        .receive_caps(owner, receive, reservation.capacity(), &mut waits)
                        .unwrap();
                }
                let result = if call {
                    endpoint.call_caps(sender, send, Message([1; 4]), batch.len(), &mut waits)
                } else {
                    endpoint.send_caps(sender, send, Message([1; 4]), batch.len(), &mut waits)
                };
                if queued {
                    assert_eq!(result, Ok(()));
                    endpoint
                        .receive_caps(owner, receive, reservation.capacity(), &mut waits)
                        .unwrap();
                    assert_eq!(
                        waits.take(send),
                        Ok(Completion::Failed {
                            error: IpcError::Capacity,
                            delivery: Delivery::NotDelivered,
                        })
                    );
                } else {
                    assert_eq!(result, Err(IpcError::Capacity));
                    assert!(waits.is_pending(send));
                    assert_eq!(
                        endpoint.cancel(send, &mut waits),
                        Err(IpcError::InvalidWait)
                    );
                    waits.abandon(send).unwrap();
                }
                assert!(waits.is_pending(receive));
                assert_eq!(receivers.live_count(), 0);
                assert_eq!(receivers.reserved_count(), capacity);
                assert_eq!(sources.live_count(), 1);
                assert_eq!(batch.len(), 2);
                assert_eq!(
                    sources
                        .lookup(source, ObjectKind::Memory, rights)
                        .unwrap()
                        .id(),
                    memory.id()
                );
                assert_eq!(memory.reference_count(), 4);

                assert_eq!(
                    endpoint.cancel(receive, &mut waits),
                    Ok(Delivery::NotDelivered)
                );
                waits.take(receive).unwrap();
                receivers.release_reservation(reservation).unwrap();
                assert_eq!(receivers.reserved_count(), 0);
                let full = receivers.reserve(2).unwrap();
                receivers.release_reservation(full).unwrap();
                drop(batch);
                assert_eq!(memory.reference_count(), 2);
                sources.close(source).unwrap();
                drop(memory);
                assert_eq!(objects.live_count(), 0);
            }
        }
    }
}

#[test]
fn accepted_call_grant_survives_source_close_and_caller_cancellation() {
    let (_tasks, [owner, caller, _]) = tasks();
    let objects = ObjectTable::<u64, 1>::new();
    let memory = objects.create(ObjectKind::Memory, 41).unwrap();
    let mut sources = HandleTable::<u64, 1, 1>::new();
    let source = sources
        .insert(
            memory.try_clone().unwrap(),
            Rights::READ | Rights::WRITE | Rights::TRANSFER,
        )
        .unwrap();
    let batch = sources
        .prepare_transfer(&[TransferSpec {
            handle: source,
            rights: Rights::READ,
        }])
        .unwrap();
    let mut receivers = HandleTable::<u64, 1, 2>::new();
    let reservation = receivers.reserve(2).unwrap();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, receive, reservation.capacity(), &mut waits)
        .unwrap();
    let call = blocked(&mut waits, caller);
    endpoint
        .call_caps(caller, call, Message([7; 4]), batch.len(), &mut waits)
        .unwrap();
    let token = received(&mut waits, receive, caller, call, 1).unwrap();
    let [Some(grant), None] = receivers.commit_transfer(reservation, batch).unwrap() else {
        panic!("one received grant expected");
    };
    assert_eq!(receivers.reserved_count(), 0);
    assert_eq!(memory.reference_count(), 3);

    sources.close(source).unwrap();
    assert_eq!(memory.reference_count(), 2);
    assert_eq!(endpoint.cancel(call, &mut waits), Ok(Delivery::Accepted));
    assert_eq!(
        waits.take(call),
        Ok(Completion::Failed {
            error: IpcError::Cancelled,
            delivery: Delivery::Accepted,
        })
    );
    assert_eq!(
        endpoint.reply(owner, token, Message::default(), &mut waits),
        Err(IpcError::InvalidReply)
    );
    let granted = receivers
        .lookup(grant, ObjectKind::Memory, Rights::READ)
        .unwrap();
    assert_eq!(granted.id(), memory.id());
    assert_eq!(granted.with(|value| *value), 41);
    drop(granted);
    for denied in [Rights::WRITE, Rights::TRANSFER] {
        assert_eq!(
            receivers
                .lookup(grant, ObjectKind::Memory, denied)
                .unwrap_err(),
            CapabilityError::PermissionDenied
        );
    }
    receivers.close(grant).unwrap();
    drop(memory);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn receiver_commit_precedes_cleanup_of_an_earlier_ready_sender_slot() {
    let (_tasks, [owner, sender, _]) = tasks();
    let objects = ObjectTable::<u64, 1>::new();
    let memory = objects.create(ObjectKind::Memory, 41).unwrap();
    let mut sources = HandleTable::<u64, 1, 1>::new();
    let source = sources
        .insert(memory.try_clone().unwrap(), Rights::READ | Rights::TRANSFER)
        .unwrap();
    let mut pending = Some(
        sources
            .prepare_transfer(&[TransferSpec {
                handle: source,
                rights: Rights::READ,
            }])
            .unwrap(),
    );
    let mut receivers = HandleTable::<u64, 1, 1>::new();
    let reservation = receivers.reserve(1).unwrap();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let send = blocked(&mut waits, sender);
    endpoint
        .send_caps(
            sender,
            send,
            Message([7; 4]),
            pending.as_ref().unwrap().len(),
            &mut waits,
        )
        .unwrap();
    sources.close(source).unwrap();
    assert_eq!(memory.reference_count(), 2);
    let receive = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, receive, reservation.capacity(), &mut waits)
        .unwrap();
    assert_eq!(send.as_raw() as u32, 1);
    assert_eq!(receive.as_raw() as u32, 2);
    assert_eq!(waits.next_ready(), Some((send, sender)));

    let received_wait = waits
        .ready()
        .find_map(|(wait, task, completion)| {
            (task == owner && matches!(completion, Completion::Received { .. })).then_some(wait)
        })
        .unwrap();
    assert_eq!(received_wait, receive);
    assert_eq!(received(&mut waits, received_wait, sender, send, 1), None);
    let [Some(grant), None] = receivers
        .commit_transfer(reservation, pending.take().unwrap())
        .unwrap()
    else {
        panic!("one received grant expected");
    };
    assert!(pending.is_none());
    assert_eq!(waits.next_ready(), Some((send, sender)));
    assert_eq!(waits.take(send), Ok(Completion::Sent));
    drop(pending);
    assert_eq!(receivers.reserved_count(), 0);
    assert_eq!(waits.next_ready(), None);
    assert_eq!(memory.reference_count(), 2);
    assert_eq!(
        receivers
            .lookup(grant, ObjectKind::Memory, Rights::READ)
            .unwrap()
            .with(|value| *value),
        41
    );
    drop(memory);
    assert_eq!(objects.live_count(), 1);
    receivers.close(grant).unwrap();
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn cancelling_a_receive_releases_reservations_before_a_full_capacity_retry() {
    let (_tasks, [owner, sender, _]) = tasks();
    let objects = ObjectTable::<u64, 1>::new();
    let memory = objects.create(ObjectKind::Memory, 41).unwrap();
    let mut sources = HandleTable::<u64, 1, 1>::new();
    let source = sources
        .insert(memory.try_clone().unwrap(), Rights::READ | Rights::TRANSFER)
        .unwrap();
    let batch = sources
        .prepare_transfer(&[TransferSpec {
            handle: source,
            rights: Rights::READ,
        }])
        .unwrap();
    let mut receivers = HandleTable::<u64, 1, 2>::new();
    let reservation = receivers.reserve(2).unwrap();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let cancelled = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, cancelled, reservation.capacity(), &mut waits)
        .unwrap();
    assert_eq!(
        endpoint.cancel(cancelled, &mut waits),
        Ok(Delivery::NotDelivered)
    );
    assert_eq!(
        waits.take(cancelled),
        Ok(Completion::Failed {
            error: IpcError::Cancelled,
            delivery: Delivery::NotDelivered,
        })
    );
    assert_eq!(receivers.reserved_count(), 2);
    receivers.release_reservation(reservation).unwrap();
    assert_eq!(receivers.reserved_count(), 0);
    assert_eq!(memory.reference_count(), 3);

    let reservation = receivers.reserve(2).unwrap();
    let receive = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, receive, reservation.capacity(), &mut waits)
        .unwrap();
    let send = blocked(&mut waits, sender);
    endpoint
        .send_caps(sender, send, Message([7; 4]), batch.len(), &mut waits)
        .unwrap();
    received(&mut waits, receive, sender, send, 1);
    let [Some(grant), None] = receivers.commit_transfer(reservation, batch).unwrap() else {
        panic!("one received grant expected");
    };
    assert_eq!(waits.take(send), Ok(Completion::Sent));
    assert_eq!(receivers.reserved_count(), 0);
    let unused = receivers.reserve(1).unwrap();
    receivers.release_reservation(unused).unwrap();
    sources.close(source).unwrap();
    receivers.close(grant).unwrap();
    drop(memory);
    assert_eq!(objects.live_count(), 0);
}

#[test]
fn owner_death_releases_wait_reservations_and_unaccepted_batches_separately() {
    let (_tasks, [owner, first, second]) = tasks();
    let objects = ObjectTable::<u64, 1>::new();
    let memory = objects.create(ObjectKind::Memory, 41).unwrap();
    let mut sources = HandleTable::<u64, 1, 1>::new();
    let source = sources
        .insert(memory.try_clone().unwrap(), Rights::READ | Rights::TRANSFER)
        .unwrap();
    let spec = TransferSpec {
        handle: source,
        rights: Rights::READ,
    };
    let accepted_batch = sources.prepare_transfer(&[spec]).unwrap();
    let mut receivers = HandleTable::<u64, 1, 2>::new();
    let reservation = receivers.reserve(2).unwrap();
    let mut waits = Waits::new();
    let mut endpoint = Endpoint::<1, 1>::new(owner);
    let receive = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, receive, reservation.capacity(), &mut waits)
        .unwrap();
    let accepted = blocked(&mut waits, first);
    endpoint
        .call_caps(
            first,
            accepted,
            Message([7; 4]),
            accepted_batch.len(),
            &mut waits,
        )
        .unwrap();
    let token = received(&mut waits, receive, first, accepted, 1).unwrap();
    let [Some(grant), None] = receivers
        .commit_transfer(reservation, accepted_batch)
        .unwrap()
    else {
        panic!("one received grant expected");
    };

    let queued_batch = sources.prepare_transfer(&[spec]).unwrap();
    let queued = blocked(&mut waits, second);
    endpoint
        .call_caps(
            second,
            queued,
            Message([7; 4]),
            queued_batch.len(),
            &mut waits,
        )
        .unwrap();
    let reservation = receivers.reserve(1).unwrap();
    let receive = blocked(&mut waits, owner);
    endpoint
        .receive_caps(owner, receive, reservation.capacity(), &mut waits)
        .unwrap();
    assert!(waits.is_pending(receive));
    assert!(waits.is_pending(queued));
    assert_eq!(memory.reference_count(), 4);
    endpoint.task_died(owner, &mut waits);
    for (wait, delivery) in [
        (receive, Delivery::NotDelivered),
        (queued, Delivery::NotDelivered),
        (accepted, Delivery::Accepted),
    ] {
        assert_eq!(
            waits.take(wait),
            Ok(Completion::Failed {
                error: IpcError::PeerClosed,
                delivery
            })
        );
    }
    assert_eq!(
        endpoint.reply(owner, token, Message::default(), &mut waits),
        Err(IpcError::PeerClosed)
    );
    assert_eq!(receivers.reserved_count(), 1);
    receivers.release_reservation(reservation).unwrap();
    assert_eq!(receivers.reserved_count(), 0);
    drop(queued_batch);
    assert_eq!(memory.reference_count(), 3);
    let remaining = receivers.reserve(1).unwrap();
    receivers.release_reservation(remaining).unwrap();
    receivers.close(grant).unwrap();
    let full = receivers.reserve(2).unwrap();
    receivers.release_reservation(full).unwrap();
    sources.close(source).unwrap();
    drop(memory);
    assert_eq!(objects.live_count(), 0);
}

type Waits = WaitTable<Completion, 4>;

fn tasks() -> (TaskTable<(), 3>, [TaskId; 3]) {
    let mut tasks = TaskTable::new();
    let ids = core::array::from_fn(|_| tasks.spawn(()).unwrap());
    (tasks, ids)
}

fn blocked(waits: &mut Waits, task: TaskId) -> WaitId {
    let wait = waits.begin(task).unwrap();
    assert_eq!(waits.park(wait), Ok(ParkResult::Blocked));
    wait
}

fn received(
    waits: &mut Waits,
    receive: WaitId,
    expected_sender: TaskId,
    expected_wait: WaitId,
    expected_count: usize,
) -> Option<ReplyToken> {
    match waits.take(receive).unwrap() {
        Completion::Received {
            message,
            reply,
            sender,
            sender_wait,
            capabilities,
        } => {
            assert_eq!(message, Message([7; 4]));
            assert_eq!(
                (sender, sender_wait, capabilities),
                (expected_sender, expected_wait, expected_count)
            );
            reply
        }
        other => panic!("expected Received, got {other:?}"),
    }
}
