use super::{BootFramePool, KernelRegion, ProcessResources, Resources, ThreadResources};
use core::ops::{Deref, DerefMut};
use kryo_core::process::{ExitStatus, ProcessId, ProcessState, ProcessTable};
use kryo_core::sched::{ReturnReason, TaskState};
use kryo_core::task::{ReapError, SpawnError, TaskError, TaskId, TaskTable};

pub(crate) struct Tasks<'kernel, 'objects> {
    threads: TaskTable<ThreadResources<'kernel, 'objects>, 4, { kryo_hal::MAX_CPUS }>,
    processes: ProcessTable<ProcessResources<'kernel, 'objects>, 2, 4>,
    thread_limit: Option<usize>,
}

pub(crate) struct ResourcesView<'borrow, 'kernel, 'objects> {
    pub(crate) process: &'borrow mut ProcessResources<'kernel, 'objects>,
    pub(crate) thread: &'borrow mut ThreadResources<'kernel, 'objects>,
    final_member: bool,
}

impl<'kernel, 'objects> Deref for ResourcesView<'_, 'kernel, 'objects> {
    type Target = ThreadResources<'kernel, 'objects>;

    fn deref(&self) -> &Self::Target {
        self.thread
    }
}

impl DerefMut for ResourcesView<'_, '_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.thread
    }
}

impl ResourcesView<'_, '_, '_> {
    pub(crate) fn close(
        &mut self,
        pool: &BootFramePool,
        region: &mut KernelRegion,
    ) -> Result<(), &'static str> {
        self.thread.close(pool, region, &mut self.process.space)?;
        if self.final_member {
            self.process.close(pool, region)?;
        }
        Ok(())
    }
}

impl<'kernel, 'objects> Tasks<'kernel, 'objects> {
    pub(crate) const fn new() -> Self {
        Self {
            threads: TaskTable::new(),
            processes: ProcessTable::new(),
            thread_limit: Some(2),
        }
    }

    pub(crate) const fn growing() -> Self {
        Self {
            threads: TaskTable::growing(),
            processes: ProcessTable::growing(),
            thread_limit: None,
        }
    }

    pub(crate) fn reserve_process(&mut self) -> Result<(), TaskError> {
        self.threads.reserve_slot()?;
        self.processes
            .reserve_slot()
            .map_err(|_| TaskError::Capacity)
    }

    pub(crate) fn reserve_thread(&mut self, process: ProcessId) -> Result<(), TaskError> {
        if self.thread_limit.is_some_and(|limit| {
            self.processes
                .thread_count(process)
                .is_ok_and(|count| count >= limit)
        }) {
            return Err(TaskError::Capacity);
        }
        self.threads.reserve_slot()?;
        self.processes
            .reserve_thread(process)
            .map_err(|_| TaskError::Capacity)
    }

    pub(crate) fn next_dying(&self, after: Option<TaskId>) -> Option<TaskId> {
        self.ids().find(|id| {
            after.is_none_or(|last| (id.as_raw() as u32) > (last.as_raw() as u32))
                && self.state(*id) == Ok(TaskState::Dying)
        })
    }

    pub(crate) fn thread(
        &self,
        id: TaskId,
    ) -> Result<&ThreadResources<'kernel, 'objects>, TaskError> {
        self.threads.resources(id)
    }

    #[expect(
        clippy::result_large_err,
        reason = "Unpublished process owners return to the creation transaction"
    )]
    pub(crate) fn spawn(
        &mut self,
        resources: Resources<'kernel, 'objects>,
    ) -> Result<TaskId, SpawnError<Resources<'kernel, 'objects>>> {
        if self.reserve_process().is_err() {
            return Err(SpawnError {
                error: TaskError::Capacity,
                resources,
            });
        }
        let (process, thread) = resources.into_parts();
        let process = match self.processes.insert(process) {
            Ok(process) => process,
            Err(failure) => {
                return Err(SpawnError {
                    error: TaskError::Capacity,
                    resources: Resources::from_parts(failure.resources, thread),
                });
            }
        };
        if self.processes.reserve_thread(process).is_err() {
            let process = self
                .processes
                .abort(process, |_| Ok::<_, ()>(()))
                .expect("Unpublished process rollback invariant");
            return Err(SpawnError {
                error: TaskError::Capacity,
                resources: Resources::from_parts(process, thread),
            });
        }
        let task = match self.threads.spawn_suspended(thread) {
            Ok(task) => task,
            Err(failure) => {
                let process = self
                    .processes
                    .abort(process, |_| Ok::<_, ()>(()))
                    .expect("Unpublished process rollback invariant");
                return Err(SpawnError {
                    error: failure.error,
                    resources: Resources::from_parts(process, failure.resources),
                });
            }
        };
        self.processes
            .attach(process, task)
            .expect("New process membership capacity invariant");
        if let Some(control) = self
            .processes
            .resources_mut(process)
            .unwrap()
            .control
            .as_ref()
        {
            super::process::bind(control, task, process);
        }
        self.threads
            .wake(task)
            .expect("Initial thread publication invariant");
        Ok(task)
    }

    #[expect(
        clippy::result_large_err,
        reason = "Unpublished thread owners return to the creation transaction"
    )]
    pub(crate) fn spawn_thread(
        &mut self,
        process: ProcessId,
        resources: ThreadResources<'kernel, 'objects>,
    ) -> Result<TaskId, SpawnError<ThreadResources<'kernel, 'objects>>> {
        if self.processes.state(process) != Ok(ProcessState::Active)
            || self.reserve_thread(process).is_err()
        {
            return Err(SpawnError {
                error: TaskError::Capacity,
                resources,
            });
        }
        let task = self.threads.spawn_suspended(resources)?;
        self.processes
            .attach(process, task)
            .expect("Validated thread membership capacity invariant");
        if let Some(control) = self
            .threads
            .resources_mut(task)
            .unwrap()
            .thread_control
            .as_ref()
        {
            control.with(|object| {
                super::with_exit(object, |exit| {
                    exit.bind(task)
                        .expect("New thread control binding invariant")
                })
                .expect("Thread control kind invariant");
            });
        }
        self.threads
            .wake(task)
            .expect("Thread publication invariant");
        Ok(task)
    }

    pub(crate) fn resources_mut(
        &mut self,
        task: TaskId,
    ) -> Result<ResourcesView<'_, 'kernel, 'objects>, TaskError> {
        let process = self.process_for(task).ok_or(TaskError::InvalidId)?;
        let final_member = self
            .processes
            .thread_count(process)
            .map_err(|_| TaskError::InvalidId)?
            == 1
            && self.processes.state(process) == Ok(ProcessState::Exiting);
        Ok(ResourcesView {
            thread: self.threads.resources_mut(task)?,
            process: self
                .processes
                .resources_mut(process)
                .map_err(|_| TaskError::InvalidId)?,
            final_member,
        })
    }

    pub(crate) fn thread_mut(
        &mut self,
        task: TaskId,
    ) -> Result<&mut ThreadResources<'kernel, 'objects>, TaskError> {
        self.threads.resources_mut(task)
    }
    pub(crate) fn process_for(&self, task: TaskId) -> Option<ProcessId> {
        self.processes.process_for(task)
    }
    pub(crate) fn process_mut(
        &mut self,
        process: ProcessId,
    ) -> Result<&mut ProcessResources<'kernel, 'objects>, TaskError> {
        self.processes
            .resources_mut(process)
            .map_err(|_| TaskError::InvalidId)
    }
    pub(crate) fn process_threads(
        &self,
        process: ProcessId,
    ) -> Result<impl Iterator<Item = TaskId> + '_, TaskError> {
        self.processes
            .threads(process)
            .map_err(|_| TaskError::InvalidId)
    }
    pub(crate) fn process_thread_count(&self, process: ProcessId) -> Result<usize, TaskError> {
        self.processes
            .thread_count(process)
            .map_err(|_| TaskError::InvalidId)
    }
    #[cfg(feature = "boot-tests")]
    pub(crate) fn process_count(&self) -> usize {
        self.processes.live_count()
    }
    pub(crate) fn terminal(&self, process: ProcessId) -> Option<ExitStatus> {
        self.processes.terminal(process).ok().flatten()
    }

    pub(crate) fn end_thread(&mut self, task: TaskId) -> Result<(), TaskError> {
        if self.state(task)? != TaskState::Dying {
            return Err(TaskError::NotDying);
        }
        let process = self.process_for(task).ok_or(TaskError::InvalidId)?;
        let thread = self.threads.resources_mut(task)?;
        let status = ExitStatus {
            reason: thread.exit_reason,
            code: thread.exit_code,
        };
        self.processes
            .end_thread(process, task, status)
            .map_err(|_| TaskError::InvalidId)?;
        Ok(())
    }

    pub(crate) fn stop_process(
        &mut self,
        process: ProcessId,
        status: ExitStatus,
    ) -> Result<(), TaskError> {
        self.processes
            .request_exit(process, status)
            .map_err(|_| TaskError::InvalidId)?;
        loop {
            let next = self
                .process_threads(process)?
                .find(|task| self.state(*task) != Ok(TaskState::Dying));
            let Some(task) = next else { break };
            let thread = self.threads.resources_mut(task)?;
            thread.exit_reason = status.reason;
            thread.exit_code = status.code;
            self.threads.stop(task)?;
        }
        Ok(())
    }

    pub(crate) fn dispatch(&mut self) -> Result<Option<TaskId>, TaskError> {
        self.threads.dispatch()
    }
    pub(crate) fn dispatch_on(&mut self, cpu: usize) -> Result<Option<TaskId>, TaskError> {
        self.threads.dispatch_on(cpu)
    }
    pub(crate) fn returned_on(
        &mut self,
        cpu: usize,
        task: TaskId,
        reason: ReturnReason,
    ) -> Result<TaskState, TaskError> {
        self.threads.returned_on(cpu, task, reason)
    }
    pub(crate) fn apply_return_policy(
        &mut self,
        task: TaskId,
        reason: ReturnReason,
    ) -> Result<(), TaskError> {
        if matches!(reason, ReturnReason::Fault | ReturnReason::Exit) {
            let process = self.process_for(task).ok_or(TaskError::InvalidId)?;
            let thread = self.threads.resources_mut(task)?;
            let status = ExitStatus {
                reason: thread.exit_reason,
                code: thread.exit_code,
            };
            self.stop_process(process, status)?;
        }
        Ok(())
    }
    pub(crate) fn has_runnable(&self) -> bool {
        self.threads.has_runnable()
    }
    pub(crate) fn ids(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.threads.ids()
    }
    pub(crate) fn state(&self, task: TaskId) -> Result<TaskState, TaskError> {
        self.threads.state(task)
    }
    pub(crate) fn wake(&mut self, task: TaskId) -> Result<(), TaskError> {
        self.threads.wake(task)
    }
    pub(crate) fn stop(&mut self, task: TaskId) -> Result<(), TaskError> {
        self.threads.stop(task)
    }
    pub(crate) fn live_count(&self) -> usize {
        self.threads.live_count()
    }

    pub(crate) fn returned(
        &mut self,
        task: TaskId,
        reason: ReturnReason,
    ) -> Result<TaskState, TaskError> {
        let state = self.threads.returned(task, reason)?;
        self.apply_return_policy(task, reason)?;
        Ok(state)
    }

    pub(crate) fn reap<E>(
        &mut self,
        task: TaskId,
        cleanup: impl FnOnce(&mut ResourcesView<'_, 'kernel, 'objects>) -> Result<(), E>,
    ) -> Result<ThreadResources<'kernel, 'objects>, ReapError<E>> {
        self.end_thread(task).map_err(ReapError::Task)?;
        let process = self
            .process_for(task)
            .ok_or(ReapError::Task(TaskError::InvalidId))?;
        cleanup(&mut self.resources_mut(task).map_err(ReapError::Task)?)
            .map_err(ReapError::Cleanup)?;
        let thread = self.threads.reap(task, |_| Ok::<_, E>(()))?;
        self.processes
            .detach(process, task)
            .expect("Retired thread membership invariant");
        if self.processes.thread_count(process) == Ok(0) {
            let owner = self
                .processes
                .reap(process, |owner| {
                    assert!(owner.space_closed, "Final process cleanup invariant");
                    Ok::<_, ()>(())
                })
                .expect("Final process retirement invariant");
            drop(owner);
        }
        Ok(thread)
    }
}
