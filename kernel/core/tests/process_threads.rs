use kryo_core::process::{
    ExitReason, ExitStatus, ProcessReapError, ProcessState, ProcessTable, ProcessTableError,
};
use kryo_core::task::{TaskId, TaskTable};
use std::cell::Cell;
use std::rc::Rc;

const FIRST: ExitStatus = ExitStatus {
    reason: ExitReason::Exited,
    code: 11,
};
const LAST: ExitStatus = ExitStatus {
    reason: ExitReason::Exited,
    code: 22,
};
const FAULT: ExitStatus = ExitStatus {
    reason: ExitReason::Fault,
    code: 14,
};

#[test]
fn threads_share_one_resource_owner_until_every_private_cleanup_retires() {
    let (_tasks, [first, second, _, _]) = tasks();
    let drops = Rc::new(Cell::new(0));
    let mut processes = ProcessTable::<Owner, 1, 2>::new();
    let process = processes.insert(Owner::new(&drops, 1)).unwrap();
    processes.attach(process, first).unwrap();
    processes.attach(process, second).unwrap();
    processes.resources_mut(process).unwrap().value = 42;
    assert_eq!(processes.process_for(first), Some(process));
    assert_eq!(processes.process_for(second), Some(process));
    assert_eq!(processes.resources_mut(process).unwrap().value, 42);
    assert_eq!(drops.get(), 0);

    assert_eq!(processes.end_thread(process, first, FIRST), Ok(None));
    assert_eq!(processes.state(process), Ok(ProcessState::Active));
    assert_eq!(processes.thread_count(process), Ok(2));
    processes.detach(process, first).unwrap();
    assert_eq!(processes.thread_count(process), Ok(1));
    assert_eq!(processes.resources_mut(process).unwrap().value, 42);
    assert_eq!(drops.get(), 0);
    assert_eq!(processes.end_thread(process, second, LAST), Ok(Some(LAST)));
    assert_eq!(processes.thread_count(process), Ok(1));
    assert!(matches!(
        processes.reap(process, |_| Ok::<_, ()>(())),
        Err(ProcessReapError::Process(ProcessTableError::HasThreads))
    ));
    processes.detach(process, second).unwrap();
    let resources = processes.reap(process, |_| Ok::<_, ()>(())).unwrap();
    assert_eq!(resources.value, 42);
    assert_eq!(drops.get(), 0);
    drop(resources);
    assert_eq!(drops.get(), 1);
}

#[test]
fn the_first_process_exit_reason_wins_after_all_execution_handoffs() {
    let (_tasks, [first, second, _, _]) = tasks();
    let mut processes = ProcessTable::<(), 1, 2>::new();
    let process = processes.insert(()).unwrap();
    processes.attach(process, first).unwrap();
    processes.attach(process, second).unwrap();
    processes.request_exit(process, FAULT).unwrap();
    processes.request_exit(process, FIRST).unwrap();
    assert_eq!(processes.state(process), Ok(ProcessState::Exiting));
    assert_eq!(processes.terminal(process), Ok(None));
    assert_eq!(processes.end_thread(process, first, FIRST), Ok(None));
    assert_eq!(processes.terminal(process), Ok(None));
    assert_eq!(processes.end_thread(process, second, LAST), Ok(Some(FAULT)));
    assert_eq!(
        processes.end_thread(process, second, FIRST),
        Ok(Some(FAULT))
    );
    assert_eq!(processes.terminal(process), Ok(Some(FAULT)));
    processes.detach(process, first).unwrap();
    processes.detach(process, second).unwrap();
    assert_eq!(processes.terminal(process), Ok(Some(FAULT)));
}

#[test]
fn last_voluntary_exit_selects_process_status_in_either_handoff_order() {
    let (_tasks, ids) = tasks();
    for order in [[ids[0], ids[1]], [ids[1], ids[0]]] {
        let mut processes = ProcessTable::<(), 1, 2>::new();
        let process = processes.insert(()).unwrap();
        processes.attach(process, ids[0]).unwrap();
        processes.attach(process, ids[1]).unwrap();
        assert_eq!(processes.end_thread(process, order[0], FIRST), Ok(None));
        assert_eq!(processes.end_thread(process, order[0], FAULT), Ok(None));
        assert_eq!(
            processes.end_thread(process, order[1], LAST),
            Ok(Some(LAST))
        );
        processes.request_exit(process, FAULT).unwrap();
        assert_eq!(processes.terminal(process), Ok(Some(LAST)));
    }
}

#[test]
fn attach_rejects_capacity_duplicates_and_exiting_without_changing_ownership() {
    let (_tasks, [first, second, third, _]) = tasks();
    let mut processes = ProcessTable::<(), 2, 1>::new();
    let one = processes.insert(()).unwrap();
    let two = processes.insert(()).unwrap();
    processes.attach(one, first).unwrap();
    assert_eq!(
        processes.attach(one, first),
        Err(ProcessTableError::DuplicateThread)
    );
    assert_eq!(
        processes.attach(two, first),
        Err(ProcessTableError::DuplicateThread)
    );
    assert_eq!(
        processes.attach(one, second),
        Err(ProcessTableError::ThreadCapacity)
    );
    assert_eq!(processes.process_for(second), None);
    processes.attach(two, second).unwrap();
    processes.end_thread(one, first, FIRST).unwrap();
    assert_eq!(
        processes.attach(two, first),
        Err(ProcessTableError::DuplicateThread)
    );
    processes.detach(one, first).unwrap();
    assert_eq!(
        processes.attach(one, third),
        Err(ProcessTableError::Exiting)
    );
    assert_eq!(processes.process_for(third), None);
    assert_eq!(
        processes.threads(two).unwrap().collect::<Vec<_>>(),
        [second]
    );
}

#[test]
fn a_live_thread_cannot_detach_and_an_ended_member_prevents_process_reap() {
    let (_tasks, [first, _, _, _]) = tasks();
    let mut processes = ProcessTable::<u8, 1, 1>::new();
    let process = processes.insert(7).unwrap();
    processes.attach(process, first).unwrap();
    assert_eq!(
        processes.detach(process, first),
        Err(ProcessTableError::ThreadNotEnded)
    );
    assert_eq!(
        processes.reap(process, |_| Ok::<_, ()>(())),
        Err(ProcessReapError::Process(ProcessTableError::NotExiting))
    );
    assert_eq!(
        processes.abort(process, |_| Ok::<_, ()>(())),
        Err(ProcessReapError::Process(ProcessTableError::HasThreads))
    );
    processes.request_exit(process, FIRST).unwrap();
    assert_eq!(
        processes.reap(process, |_| Ok::<_, ()>(())),
        Err(ProcessReapError::Process(ProcessTableError::HasThreads))
    );
    processes.end_thread(process, first, FIRST).unwrap();
    assert_eq!(
        processes.reap(process, |_| Ok::<_, ()>(())),
        Err(ProcessReapError::Process(ProcessTableError::HasThreads))
    );
    processes.detach(process, first).unwrap();
    assert_eq!(processes.reap(process, |_| Ok::<_, ()>(())), Ok(7));
}

#[test]
fn cleanup_failure_keeps_resources_status_and_process_generation_for_retry() {
    let (_tasks, [task, _, _, _]) = tasks();
    let mut processes = ProcessTable::<u64, 1, 1>::new();
    let process = processes.insert(1).unwrap();
    processes.attach(process, task).unwrap();
    processes.end_thread(process, task, FIRST).unwrap();
    processes.detach(process, task).unwrap();
    assert_eq!(
        processes.reap(process, |value| {
            *value = 2;
            Err("pending")
        }),
        Err(ProcessReapError::Cleanup("pending"))
    );
    assert_eq!(processes.terminal(process), Ok(Some(FIRST)));
    assert_eq!(*processes.resources_mut(process).unwrap(), 2);
    assert_eq!(processes.live_count(), 1);
    let rejected = processes.insert(3).unwrap_err();
    assert_eq!(rejected.error, ProcessTableError::Capacity);
    assert_eq!(rejected.resources, 3);
    assert_eq!(processes.reap(process, |_| Ok::<_, ()>(())), Ok(2));
    let replacement = processes.insert(4).unwrap();
    assert_ne!(replacement, process);
    assert_eq!(processes.state(process), Err(ProcessTableError::InvalidId));
    assert_eq!(processes.terminal(replacement), Ok(None));
}

#[test]
fn stale_process_and_thread_generations_cannot_redirect_membership() {
    let mut tasks = TaskTable::<(), 1>::new();
    let old_task = tasks.spawn(()).unwrap();
    let mut processes = ProcessTable::<(), 1, 1>::new();
    let old_process = processes.insert(()).unwrap();
    processes.attach(old_process, old_task).unwrap();
    tasks.stop(old_task).unwrap();
    processes.end_thread(old_process, old_task, FIRST).unwrap();
    tasks.reap(old_task, |_| Ok::<_, ()>(())).unwrap();
    processes.detach(old_process, old_task).unwrap();
    processes.reap(old_process, |_| Ok::<_, ()>(())).unwrap();
    let new_task = tasks.spawn(()).unwrap();
    let new_process = processes.insert(()).unwrap();
    processes.attach(new_process, new_task).unwrap();
    assert_ne!(old_task, new_task);
    assert_ne!(old_process, new_process);
    assert_eq!(processes.process_for(old_task), None);
    assert_eq!(processes.process_for(new_task), Some(new_process));
    assert_eq!(
        processes.request_exit(old_process, FAULT),
        Err(ProcessTableError::InvalidId)
    );
    assert_eq!(
        processes.end_thread(new_process, old_task, FAULT),
        Err(ProcessTableError::InvalidThread)
    );
    assert_eq!(
        processes.detach(new_process, old_task),
        Err(ProcessTableError::InvalidThread)
    );
    assert_eq!(processes.state(new_process), Ok(ProcessState::Active));
    assert_eq!(processes.terminal(new_process), Ok(None));
}

#[test]
fn unpublished_abort_retries_cleanup_without_losing_its_owner() {
    let drops = Rc::new(Cell::new(0));
    let mut processes = ProcessTable::<Owner, 1, 1>::new();
    let process = processes.insert(Owner::new(&drops, 7)).unwrap();
    assert_eq!(processes.terminal(process), Ok(None));
    assert!(matches!(
        processes.abort(process, |_| Err("pending")),
        Err(ProcessReapError::Cleanup("pending"))
    ));
    assert_eq!(drops.get(), 0);
    assert_eq!(processes.state(process), Ok(ProcessState::Active));
    let resources = processes.abort(process, |_| Ok::<_, ()>(())).unwrap();
    assert_eq!(drops.get(), 0);
    drop(resources);
    assert_eq!(drops.get(), 1);
    let replacement = processes.insert(Owner::new(&drops, 8)).unwrap();
    assert_ne!(replacement, process);
    drop(processes.abort(replacement, |_| Ok::<_, ()>(())).unwrap());
    assert_eq!(drops.get(), 2);
}

#[test]
fn dropping_a_registry_does_not_destroy_unretired_process_resources() {
    let drops = Rc::new(Cell::new(0));
    let mut processes = ProcessTable::<Owner, 1, 1>::new();
    processes.insert(Owner::new(&drops, 9)).unwrap();
    drop(processes);
    assert_eq!(drops.get(), 0);
}

#[test]
fn zero_capacity_and_empty_processes_have_no_terminal_thread_event() {
    let (_tasks, [task, _, _, _]) = tasks();
    let mut none = ProcessTable::<u8, 0, 0>::new();
    assert_eq!(none.insert(7).unwrap_err().resources, 7);
    assert_eq!(none.live_count(), 0);
    assert_eq!(none.ids().count(), 0);
    let mut processes = ProcessTable::<(), 1, 0>::new();
    let process = processes.insert(()).unwrap();
    assert_eq!(
        processes.attach(process, task),
        Err(ProcessTableError::ThreadCapacity)
    );
    assert_eq!(
        processes.end_thread(process, task, FIRST),
        Err(ProcessTableError::InvalidThread)
    );
    assert_eq!(processes.terminal(process), Ok(None));
    assert_eq!(processes.state(process), Ok(ProcessState::Active));
    processes.abort(process, |_| Ok::<_, ()>(())).unwrap();
}

struct Owner {
    drops: Rc<Cell<usize>>,
    value: u64,
}

impl Owner {
    fn new(drops: &Rc<Cell<usize>>, value: u64) -> Self {
        Self {
            drops: Rc::clone(drops),
            value,
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

fn tasks() -> (TaskTable<(), 4>, [TaskId; 4]) {
    let mut tasks = TaskTable::new();
    let ids = core::array::from_fn(|_| tasks.spawn(()).unwrap());
    (tasks, ids)
}
