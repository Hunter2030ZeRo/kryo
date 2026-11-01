use core::cell::Cell;
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::task::{ReapError, TaskError, TaskTable};
use std::rc::Rc;

#[test]
fn each_cpu_owns_a_distinct_running_task() {
    let mut tasks = TaskTable::<u64, 3, 4>::new();
    let ids = core::array::from_fn::<_, 3, _>(|index| tasks.spawn(index as u64).unwrap());
    assert!(!tasks.has_running());
    for (cpu, task) in ids.into_iter().enumerate() {
        assert_eq!(tasks.dispatch_on(cpu), Ok(Some(task)));
        assert_eq!(tasks.current_on(cpu), Ok(Some(task)));
        assert_eq!(tasks.state(task), Ok(TaskState::Running));
        assert_eq!(tasks.dispatch_on(cpu), Err(TaskError::AlreadyRunning));
    }
    assert_eq!(tasks.current(), Some(ids[0]));
    assert!(tasks.has_running());
    assert!(!tasks.has_runnable());
    assert_eq!(tasks.dispatch_on(3), Ok(None));
    assert_eq!(tasks.current_on(3), Ok(None));
    for cpu in [2, 0, 1] {
        tasks
            .returned_on(cpu, ids[cpu], ReturnReason::Call)
            .unwrap();
        assert_eq!(tasks.current_on(cpu), Ok(None));
    }
    assert!(!tasks.has_running());
}

#[test]
fn wrong_cpu_completion_preserves_all_execution_owners() {
    let mut tasks = TaskTable::<(), 2, 3>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    tasks.dispatch_on(0).unwrap();
    tasks.dispatch_on(1).unwrap();
    for (cpu, task) in [(0, second), (1, first), (2, first), (2, second)] {
        assert_eq!(
            tasks.returned_on(cpu, task, ReturnReason::Exit),
            Err(TaskError::NotRunning)
        );
        assert_eq!(tasks.current_on(0), Ok(Some(first)));
        assert_eq!(tasks.current_on(1), Ok(Some(second)));
        assert_eq!(tasks.state(first), Ok(TaskState::Running));
        assert_eq!(tasks.state(second), Ok(TaskState::Running));
    }
    assert_eq!(
        tasks.returned(second, ReturnReason::Exit),
        Err(TaskError::NotRunning)
    );
    tasks
        .returned_on(1, second, ReturnReason::Preempted)
        .unwrap();
    assert!(tasks.has_running());
    assert_eq!(tasks.current_on(0), Ok(Some(first)));
    assert_eq!(tasks.dispatch_on(2), Ok(Some(second)));
    assert_eq!(tasks.dispatch_on(1), Ok(None));
    assert_eq!(tasks.current_on(2), Ok(Some(second)));
    assert_eq!(
        tasks.returned_on(1, second, ReturnReason::Exit),
        Err(TaskError::NotRunning)
    );
    tasks.returned_on(2, second, ReturnReason::Exit).unwrap();
    tasks.returned(first, ReturnReason::Exit).unwrap();
    assert!(!tasks.has_running());
}

#[test]
fn invalid_cpu_indices_never_change_round_robin_or_owner_state() {
    let mut tasks = TaskTable::<(), 2, 2>::new();
    let first = tasks.spawn(()).unwrap();
    let second = tasks.spawn(()).unwrap();
    for cpu in [2, usize::MAX] {
        assert_eq!(tasks.dispatch_on(cpu), Err(TaskError::InvalidCpu));
        assert_eq!(tasks.current_on(cpu), Err(TaskError::InvalidCpu));
        assert_eq!(
            tasks.returned_on(cpu, first, ReturnReason::Exit),
            Err(TaskError::InvalidCpu)
        );
    }
    assert!(!tasks.has_running());
    assert_eq!(tasks.dispatch(), Ok(Some(first)));
    assert_eq!(tasks.dispatch_on(1), Ok(Some(second)));
    assert_eq!(
        tasks.returned_on(2, first, ReturnReason::Exit),
        Err(TaskError::InvalidCpu)
    );
    assert_eq!(tasks.current(), Some(first));
    assert_eq!(tasks.current_on(1), Ok(Some(second)));

    let mut no_cpus = TaskTable::<(), 1, 0>::new();
    let task = no_cpus.spawn(()).unwrap();
    assert_eq!(no_cpus.current(), None);
    assert_eq!(no_cpus.dispatch(), Err(TaskError::InvalidCpu));
    assert_eq!(
        no_cpus.returned(task, ReturnReason::Exit),
        Err(TaskError::InvalidCpu)
    );
    assert!(!no_cpus.has_running());
}

#[test]
fn running_tasks_cannot_stop_or_reap_until_their_own_cpu_handoff() {
    let mut tasks = TaskTable::<u8, 2, 2>::new();
    let first = tasks.spawn(41).unwrap();
    let second = tasks.spawn(42).unwrap();
    tasks.dispatch_on(0).unwrap();
    tasks.dispatch_on(1).unwrap();
    let cleanup_calls = Cell::new(0);
    for task in [first, second] {
        assert_eq!(tasks.stop(task), Err(TaskError::Running));
        assert_eq!(
            tasks.reap(task, |_| {
                cleanup_calls.set(cleanup_calls.get() + 1);
                Ok::<_, ()>(())
            }),
            Err(ReapError::Task(TaskError::NotDying))
        );
    }
    assert_eq!(cleanup_calls.get(), 0);
    tasks.returned_on(0, first, ReturnReason::Fault).unwrap();
    assert_eq!(tasks.current_on(0), Ok(None));
    assert_eq!(tasks.current_on(1), Ok(Some(second)));
    assert_eq!(
        tasks.reap(first, |_| Err("pending cleanup")),
        Err(ReapError::Cleanup("pending cleanup"))
    );
    assert_eq!(tasks.state(first), Ok(TaskState::Dying));
    assert_eq!(tasks.current_on(1), Ok(Some(second)));
    assert_eq!(tasks.reap(first, |_| Ok::<_, ()>(())), Ok(41));
    assert_eq!(tasks.state(second), Ok(TaskState::Running));
    tasks.returned_on(1, second, ReturnReason::Call).unwrap();
    tasks.stop(second).unwrap();
    assert_eq!(tasks.reap(second, |_| Ok::<_, ()>(())), Ok(42));
    assert!(!tasks.has_running());
}

#[test]
fn stale_returns_cannot_release_a_reused_task_on_another_cpu() {
    let mut tasks = TaskTable::<(), 1, 2>::new();
    let old = tasks.spawn(()).unwrap();
    tasks.dispatch_on(1).unwrap();
    tasks.returned_on(1, old, ReturnReason::Exit).unwrap();
    tasks.reap(old, |_| Ok::<_, ()>(())).unwrap();
    let current = tasks.spawn(()).unwrap();
    assert_ne!(current, old);
    assert_eq!(tasks.dispatch_on(0), Ok(Some(current)));
    for cpu in 0..2 {
        assert_eq!(
            tasks.returned_on(cpu, old, ReturnReason::Preempted),
            Err(TaskError::NotRunning)
        );
    }
    assert_eq!(tasks.current_on(0), Ok(Some(current)));
    assert_eq!(tasks.state(current), Ok(TaskState::Running));
    assert_eq!(
        tasks.returned_on(1, current, ReturnReason::Exit),
        Err(TaskError::NotRunning)
    );
    tasks.returned_on(0, current, ReturnReason::Exit).unwrap();
    assert!(!tasks.has_running());
}

#[test]
fn one_serialized_round_robin_cursor_remains_fair_across_cpu_batches() {
    let mut tasks = TaskTable::<(), 5, 4>::new();
    let ids = core::array::from_fn::<_, 5, _>(|_| tasks.spawn(()).unwrap());
    for batch in 0..5 {
        let expected = core::array::from_fn::<_, 4, _>(|cpu| ids[(batch * 4 + cpu) % 5]);
        for (cpu, task) in expected.into_iter().enumerate() {
            assert_eq!(tasks.dispatch_on(cpu), Ok(Some(task)));
        }
        for cpu in [3, 1, 2, 0] {
            tasks
                .returned_on(cpu, expected[cpu], ReturnReason::Preempted)
                .unwrap();
        }
        assert!(!tasks.has_running());
    }
}

#[test]
fn process_policy_must_wait_for_all_cpu_handoffs_before_stopping_peers() {
    let mut tasks = TaskTable::<(), 2, 2>::new();
    let faulted = tasks.spawn(()).unwrap();
    let peer = tasks.spawn(()).unwrap();
    tasks.dispatch_on(0).unwrap();
    tasks.dispatch_on(1).unwrap();
    tasks.returned_on(0, faulted, ReturnReason::Fault).unwrap();
    assert_eq!(tasks.stop(peer), Err(TaskError::Running));
    assert_eq!(tasks.state(peer), Ok(TaskState::Running));
    tasks.returned_on(1, peer, ReturnReason::Call).unwrap();
    assert!(!tasks.has_running());
    tasks.stop(peer).unwrap();
    assert_eq!(tasks.state(faulted), Ok(TaskState::Dying));
    assert_eq!(tasks.state(peer), Ok(TaskState::Dying));
}

#[test]
fn suspended_thread_cannot_dispatch_before_explicit_publication() {
    let mut tasks = TaskTable::<u64, 1>::new();
    let thread = tasks.spawn_suspended(41).unwrap();
    assert_eq!(tasks.state(thread), Ok(TaskState::Blocked));
    assert!(!tasks.has_runnable());
    assert_eq!(tasks.dispatch(), Ok(None));
    assert_eq!(tasks.current(), None);
    *tasks.resources_mut(thread).unwrap() = 42;
    assert_eq!(
        tasks.returned(thread, ReturnReason::Exit),
        Err(TaskError::NotRunning)
    );
    tasks.wake(thread).unwrap();
    assert_eq!(tasks.dispatch(), Ok(Some(thread)));
    assert_eq!(*tasks.resources_mut(thread).unwrap(), 42);
    tasks.returned(thread, ReturnReason::Exit).unwrap();
    assert_eq!(tasks.reap(thread, |_| Ok::<_, ()>(())), Ok(42));
}

#[test]
fn failed_suspended_spawn_returns_ownership_without_changing_existing_state() {
    let mut tasks = TaskTable::<u64, 1>::new();
    let thread = tasks.spawn_suspended(1).unwrap();
    let rejected = tasks.spawn_suspended(2).unwrap_err();
    assert_eq!(rejected.error, TaskError::Capacity);
    assert_eq!(rejected.resources, 2);
    assert_eq!(tasks.state(thread), Ok(TaskState::Blocked));
    assert_eq!(tasks.live_count(), 1);
    tasks.stop(thread).unwrap();
    assert_eq!(tasks.reap(thread, |_| Ok::<_, ()>(())), Ok(1));
}

#[test]
fn ordinary_spawn_remains_runnable_while_another_thread_is_suspended() {
    let mut tasks = TaskTable::<(), 2>::new();
    let suspended = tasks.spawn_suspended(()).unwrap();
    let runnable = tasks.spawn(()).unwrap();
    assert_eq!(tasks.dispatch(), Ok(Some(runnable)));
    tasks.returned(runnable, ReturnReason::Exit).unwrap();
    assert_eq!(tasks.dispatch(), Ok(None));
    tasks.wake(suspended).unwrap();
    assert_eq!(tasks.dispatch(), Ok(Some(suspended)));
}

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
