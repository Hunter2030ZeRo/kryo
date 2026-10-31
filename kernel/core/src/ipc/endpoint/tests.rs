use super::*;
use crate::sync::wait::ParkResult;
use crate::task::TaskTable;

#[test]
fn reply_generation_wrap_retires_the_slot_and_rejects_unaccepted_calls() {
    let mut tasks = TaskTable::<(), 3>::new();
    let owner = tasks.spawn(()).unwrap();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    let mut waits = WaitTable::<Completion, 4>::new();
    let mut endpoint = Endpoint::<2, 1>::new(owner);
    endpoint.replies[0].generation = u32::MAX;
    let receive = waits.begin(owner).unwrap();
    waits.park(receive).unwrap();
    endpoint.receive(owner, receive, &mut waits).unwrap();
    let first_wait = waits.begin(first).unwrap();
    waits.park(first_wait).unwrap();
    endpoint
        .call(first, first_wait, Message([1; 4]), &mut waits)
        .unwrap();
    let token = match waits.take(receive).unwrap() {
        Completion::Received {
            reply: Some(token), ..
        } => token,
        _ => panic!("missing token"),
    };
    assert_eq!(token.as_raw() >> 32, u64::from(u32::MAX));
    let second_wait = waits.begin(second).unwrap();
    waits.park(second_wait).unwrap();
    endpoint
        .call(second, second_wait, Message([2; 4]), &mut waits)
        .unwrap();
    let next_receive = waits.begin(owner).unwrap();
    assert_eq!(waits.park(next_receive), Ok(ParkResult::Blocked));
    endpoint.receive(owner, next_receive, &mut waits).unwrap();
    endpoint
        .reply(owner, token, Message([3; 4]), &mut waits)
        .unwrap();
    assert_eq!(
        waits.take(first_wait),
        Ok(Completion::Replied(Message([3; 4])))
    );
    assert_eq!(
        waits.take(second_wait),
        Ok(Completion::Failed {
            error: IpcError::Capacity,
            delivery: Delivery::NotDelivered,
        })
    );
    assert!(waits.is_pending(next_receive));
    assert_eq!(
        endpoint.reply(owner, token, Message::default(), &mut waits),
        Err(IpcError::InvalidReply)
    );
    let fresh = waits.begin(second).unwrap();
    assert_eq!(
        endpoint.call(second, fresh, Message::default(), &mut waits),
        Err(IpcError::Capacity)
    );
    endpoint.close(&mut waits);
}
