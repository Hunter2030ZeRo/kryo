use core::cell::Cell;
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::task::{ReapError, TaskError, TaskTable};
use std::rc::Rc;

#[test]
fn round_robin_hands_off_one_saved_context_at_a_time() {
    let mut tasks = TaskTable::<u64, 3>::new();
    let first = tasks.spawn(10).unwrap();
    let second = tasks.spawn(20).unwrap();
    let third = tasks.spawn(30).unwrap();
    for expected in [first, second, third, first, second, third] {
        assert_eq!(tasks.dispatch().unwrap(), Some(expected));
        assert_eq!(tasks.state(expected), Ok(TaskState::Running));
        *tasks.resources_mut(expected).unwrap() += 1;
        assert_eq!(tasks.dispatch(), Err(TaskError::AlreadyRunning));
        assert_eq!(tasks.stop(expected), Err(TaskError::Running));
        assert_eq!(
            tasks.reap(expected, |_| Ok::<_, ()>(())),
            Err(ReapError::Task(TaskError::NotDying))
        );
        assert_eq!(
            tasks.returned(expected, ReturnReason::Preempted),
            Ok(TaskState::Runnable)
        );
        assert_eq!(tasks.current(), None);
        assert_eq!(
            tasks.returned(expected, ReturnReason::Preempted),
            Err(TaskError::NotRunning)
        );
    }
}

#[test]
fn call_waits_for_completion_and_yield_requeues() {
    let mut tasks = TaskTable::<(), 2>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    assert_eq!(tasks.dispatch().unwrap(), Some(first));
    tasks.returned(first, ReturnReason::Call).unwrap();
    assert_eq!(tasks.state(first), Ok(TaskState::Blocked));
    assert_eq!(tasks.dispatch().unwrap(), Some(second));
    tasks.returned(second, ReturnReason::Yielded).unwrap();
    assert_eq!(tasks.dispatch().unwrap(), Some(second));
    tasks.returned(second, ReturnReason::Call).unwrap();
    assert_eq!(tasks.dispatch().unwrap(), None);
    tasks.wake(first).unwrap();
    assert_eq!(tasks.wake(first), Err(TaskError::NotBlocked));
    assert_eq!(tasks.dispatch().unwrap(), Some(first));
}

#[test]
fn wrong_completion_cannot_release_another_tasks_execution_owner() {
    let mut tasks = TaskTable::<(), 2>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    tasks.dispatch().unwrap();
    assert_eq!(
        tasks.returned(second, ReturnReason::Exit),
        Err(TaskError::NotRunning)
    );
    assert_eq!(tasks.current(), Some(first));
    assert_eq!(tasks.state(first), Ok(TaskState::Running));
    assert_eq!(tasks.state(second), Ok(TaskState::Runnable));
    tasks.returned(first, ReturnReason::Exit).unwrap();
    assert_eq!(tasks.dispatch().unwrap(), Some(second));
}

#[test]
fn fault_and_exit_retain_resources_until_cleanup_succeeds() {
    for reason in [ReturnReason::Fault, ReturnReason::Exit] {
        let mut tasks = TaskTable::<u64, 1>::new();
        let id = tasks.spawn(42).unwrap();
        tasks.dispatch().unwrap();
        assert_eq!(tasks.returned(id, reason), Ok(TaskState::Dying));
        assert_eq!(tasks.current(), None);
        assert_eq!(tasks.dispatch().unwrap(), None);
        assert_eq!(
            tasks.reap(id, |_| Err::<(), _>("not quiescent")),
            Err(ReapError::Cleanup("not quiescent"))
        );
        assert_eq!(tasks.state(id), Ok(TaskState::Dying));
        assert_eq!(*tasks.resources_mut(id).unwrap(), 42);
        assert_eq!(tasks.live_count(), 1);
        let resources = tasks.reap(id, |_| Ok::<_, ()>(())).unwrap();
        assert_eq!(resources, 42);
        assert_eq!(tasks.live_count(), 0);
        assert_eq!(tasks.state(id), Err(TaskError::InvalidId));
    }
}

#[test]
fn a_reused_slot_never_accepts_the_old_task_id() {
    let mut tasks = TaskTable::<(), 1>::new();
    let old = tasks.spawn(()).unwrap();
    assert_ne!(old.as_raw(), 0);
    tasks.stop(old).unwrap();
    tasks.reap(old, |_| Ok::<_, ()>(())).unwrap();
    let new = tasks.spawn(()).unwrap();
    assert_ne!(old, new);
    assert_eq!(tasks.state(old), Err(TaskError::InvalidId));
    assert_eq!(tasks.stop(old), Err(TaskError::InvalidId));
    assert_eq!(tasks.state(new), Ok(TaskState::Runnable));
}

#[test]
fn stopping_blocked_or_runnable_tasks_does_not_make_them_running() {
    let mut tasks = TaskTable::<(), 2>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    tasks.dispatch().unwrap();
    tasks.returned(first, ReturnReason::Call).unwrap();
    tasks.stop(first).unwrap();
    tasks.stop(second).unwrap();
    assert_eq!(tasks.stop(first), Err(TaskError::Dying));
    assert_eq!(tasks.dispatch().unwrap(), None);
}

struct Resource(Rc<Cell<usize>>);

impl Drop for Resource {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn capacity_failure_preserves_the_rejected_resource() {
    let drops = Rc::new(Cell::new(0));
    let mut tasks = TaskTable::<Resource, 0>::new();
    assert_eq!(tasks.dispatch().unwrap(), None);
    let error = tasks.spawn(Resource(drops.clone())).unwrap_err();
    assert_eq!(error.error, TaskError::Capacity);
    assert_eq!(drops.get(), 0);
    drop(error.resources);
    assert_eq!(drops.get(), 1);
}

#[test]
fn dropping_a_table_cannot_destroy_unreaped_resources() {
    let drops = Rc::new(Cell::new(0));
    let mut tasks = TaskTable::<Resource, 1>::new();
    let id = tasks.spawn(Resource(drops.clone())).unwrap();
    tasks.stop(id).unwrap();
    drop(tasks);
    assert_eq!(drops.get(), 0);
}

#[test]
fn successful_reap_returns_resources_for_explicit_destruction() {
    let drops = Rc::new(Cell::new(0));
    let mut tasks = TaskTable::<Resource, 1>::new();
    let id = tasks.spawn(Resource(drops.clone())).unwrap();
    tasks.stop(id).unwrap();
    let resource = tasks.reap(id, |_| Ok::<_, ()>(())).unwrap();
    assert_eq!(drops.get(), 0);
    drop(resource);
    assert_eq!(drops.get(), 1);
}
