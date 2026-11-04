use super::{Batch, KernelObject, Object, Objects, Tasks};
use kryo_core::ipc::{Completion, Delivery, IpcError};
use kryo_core::mm::vm::PrivateWordKey;
use kryo_core::process::ExitStatus;
use kryo_core::resource::Budget;
use kryo_core::sched::TaskState;
use kryo_core::storage::Slots;
use kryo_core::sync::WordWaiters;
use kryo_core::sync::wait::{WaitId, WaitTable};
use kryo_core::task::TaskId;
use kryo_interfaces::syscall::Error;

#[cfg(feature = "boot-tests")]
pub(crate) mod checks;

pub(crate) mod gates;

struct PendingTransfer<'objects> {
    sender: TaskId,
    wait: WaitId,
    batch: Batch<'objects>,
}

pub(crate) struct IpcRuntime<'objects> {
    pub(crate) waits: WaitTable<Completion, 4>,
    pub(crate) arena: Option<&'objects Objects>,
    pub(crate) device: Option<Object<'objects>>,
    pub(crate) device_signal: Option<Object<'objects>>,
    pub(crate) transport: Option<super::transport::Transport<'objects>>,
    channels: Slots<Option<Object<'objects>>, 4>,
    gates: Slots<Option<gates::GateRecord<'objects>>, 4>,
    gate_epoch: u64,
    gates_dirty: bool,
    #[cfg(feature = "boot-tests")]
    pub(crate) gate_scans: usize,
    transfers: Slots<Option<PendingTransfer<'objects>>, 4>,
    sleepers: Slots<Option<WaitId>, 4>,
    observations: Slots<Option<(WaitId, Object<'objects>)>, 4>,
    pub(crate) words: WordWaiters<PrivateWordKey>,
    pub(crate) accepted_failures: usize,
    pub(crate) transferred: usize,
    pub(crate) out_of_order: usize,
}

impl<'objects> IpcRuntime<'objects> {
    pub(crate) fn new() -> Self {
        Self {
            waits: WaitTable::new(),
            arena: None,
            device: None,
            device_signal: None,
            transport: None,
            channels: Slots::new([const { None }; 4]),
            gates: Slots::new([const { None }; 4]),
            gate_epoch: 0,
            gates_dirty: false,
            #[cfg(feature = "boot-tests")]
            gate_scans: 0,
            transfers: Slots::new([const { None }; 4]),
            sleepers: Slots::new([None; 4]),
            observations: Slots::new([const { None }; 4]),
            words: WordWaiters::new(),
            accepted_failures: 0,
            transferred: 0,
            out_of_order: 0,
        }
    }

    pub(crate) fn with_objects(arena: &'objects Objects) -> Self {
        let mut runtime = Self::new();
        runtime.arena = Some(arena);
        runtime
    }

    pub(crate) fn growing(arena: &'objects Objects) -> Self {
        Self {
            waits: WaitTable::growing(),
            arena: Some(arena),
            channels: Slots::growing(),
            gates: Slots::growing(),
            transfers: Slots::growing(),
            sleepers: Slots::growing(),
            observations: Slots::growing(),
            ..Self::new()
        }
    }

    #[cfg_attr(
        not(feature = "boot-tests"),
        expect(dead_code, reason = "Trusted unbudgeted fixture setup")
    )]
    pub(crate) fn begin_wait(
        &mut self,
        task: TaskId,
        deadline: Option<u64>,
    ) -> Result<WaitId, Error> {
        self.begin_wait_for(task, deadline, None)
    }

    pub(crate) fn begin_wait_for(
        &mut self,
        task: TaskId,
        deadline: Option<u64>,
        account: Option<&Budget>,
    ) -> Result<WaitId, Error> {
        // Completion and cancellation cannot allocate. Reserve side records
        // before publishing any wait or lending capabilities to an endpoint.
        let length = self
            .waits
            .live_count()
            .checked_add(1)
            .ok_or(Error::OutOfResources)?;
        self.transfers
            .reserve_len(length, || None)
            .map_err(|_| Error::OutOfResources)?;
        self.sleepers
            .reserve_len(length, || None)
            .map_err(|_| Error::OutOfResources)?;
        self.observations
            .reserve_len(length, || None)
            .map_err(|_| Error::OutOfResources)?;
        if let Some(account) = account {
            self.waits.begin_budgeted(task, deadline, account)
        } else {
            match deadline {
                Some(deadline) => self.waits.begin_until(task, deadline),
                None => self.waits.begin(task),
            }
        }
        .map_err(|_| Error::OutOfResources)
    }

    #[cfg_attr(
        not(feature = "boot-tests"),
        expect(dead_code, reason = "Bounded IPC fixture setup")
    )]
    pub(crate) fn register(&mut self, object: Object<'objects>) {
        self.try_register(object)
            .unwrap_or_else(|_| panic!("IPC fixture registry full"));
    }

    pub(crate) fn try_register(
        &mut self,
        object: Object<'objects>,
    ) -> Result<(), Object<'objects>> {
        let Ok(index) = self.channels.vacant(Option::is_none, || None) else {
            return Err(object);
        };
        self.channels[index] = Some(object);
        Ok(())
    }

    pub(crate) fn unregister(&mut self, id: kryo_core::object::ObjectId) {
        if let Some(slot) = self
            .channels
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|object| object.id() == id))
        {
            *slot = None;
        }
    }

    pub(crate) fn hold_transfer(&mut self, sender: TaskId, wait: WaitId, batch: Batch<'objects>) {
        assert!(!batch.is_empty());
        let slot = self
            .transfers
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("One transfer per active wait invariant");
        *slot = Some(PendingTransfer {
            sender,
            wait,
            batch,
        });
    }

    fn take_transfer(&mut self, wait: WaitId) -> Option<PendingTransfer<'objects>> {
        self.transfers
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|value| value.wait == wait))
            .and_then(Option::take)
    }

    pub(crate) fn observe(&mut self, wait: WaitId, control: Object<'objects>) {
        *self
            .observations
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("One observation per live wait invariant") = Some((wait, control));
    }

    fn observation(&self, wait: WaitId) -> Option<usize> {
        self.observations
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|(id, _)| *id == wait))
    }

    fn remove_observation(&mut self, wait: WaitId) {
        if let Some(index) = self.observation(wait) {
            self.observations[index] = None;
        }
    }

    pub(crate) fn cancel(&mut self, wait: WaitId) -> Result<Delivery, IpcError> {
        if self.words.remove(wait) {
            self.waits
                .complete(
                    wait,
                    Completion::Failed {
                        error: IpcError::Cancelled,
                        delivery: Delivery::NotDelivered,
                    },
                )
                .map_err(|_| IpcError::InvalidWait)?;
            return Ok(Delivery::NotDelivered);
        }
        if let Some(index) = self.observation(wait) {
            let result = self.observations[index].as_ref().unwrap().1.with(|object| {
                super::with_exit(object, |exit| exit.cancel(wait, &mut self.waits))
                    .expect("Observation object kind invariant")
            });
            if result.is_ok() {
                self.observations[index] = None;
            }
            return result.map(|()| Delivery::NotDelivered);
        }
        if self.remove_sleeper(wait) {
            self.waits
                .complete(
                    wait,
                    Completion::Failed {
                        error: IpcError::Cancelled,
                        delivery: Delivery::NotDelivered,
                    },
                )
                .map_err(|_| IpcError::InvalidWait)?;
            return Ok(Delivery::NotDelivered);
        }
        for object in self.channels.iter().flatten() {
            let result = object.with(|object| match object {
                KernelObject::Endpoint(endpoint) => {
                    endpoint.borrow_mut().cancel(wait, &mut self.waits)
                }
                KernelObject::Notification(notification) => notification
                    .borrow_mut()
                    .cancel(wait, &mut self.waits)
                    .map(|()| Delivery::NotDelivered),
                _ => Err(IpcError::InvalidWait),
            });
            if result.is_ok() {
                return result;
            }
        }
        Err(IpcError::InvalidWait)
    }

    pub(crate) fn sleep_for(
        &mut self,
        task: TaskId,
        deadline: u64,
        account: Option<&Budget>,
    ) -> Result<WaitId, Error> {
        let wait = self.begin_wait_for(task, Some(deadline), account)?;
        *self
            .sleepers
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("One sleep record per live wait invariant") = Some(wait);
        Ok(wait)
    }

    fn remove_sleeper(&mut self, wait: WaitId) -> bool {
        if let Some(slot) = self.sleepers.iter_mut().find(|slot| **slot == Some(wait)) {
            *slot = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn expire(&mut self, now: u64) {
        let mut expired = false;
        while let Some(wait) = self.waits.next_expired(now) {
            expired = true;
            if self.words.remove(wait) {
                self.waits
                    .complete(
                        wait,
                        Completion::Failed {
                            error: IpcError::TimedOut,
                            delivery: Delivery::NotDelivered,
                        },
                    )
                    .expect("Expired private word wait invariant");
                continue;
            }
            if let Some(index) = self.observation(wait) {
                self.observations[index].as_ref().unwrap().1.with(|object| {
                    super::with_exit(object, |exit| exit.timeout(wait, &mut self.waits))
                        .expect("Observation object kind invariant")
                        .expect("Expired observation owner invariant");
                });
                self.observations[index] = None;
                continue;
            }
            if self.remove_sleeper(wait) {
                self.waits
                    .complete(wait, Completion::Slept)
                    .expect("Missing sleep wait");
                continue;
            }
            let mut detached = false;
            for object in self.channels.iter().flatten() {
                let result = object.with(|object| match object {
                    KernelObject::Endpoint(endpoint) => endpoint
                        .borrow_mut()
                        .timeout(wait, &mut self.waits)
                        .map(|_| ()),
                    KernelObject::Notification(notification) => {
                        notification.borrow_mut().timeout(wait, &mut self.waits)
                    }
                    _ => Err(IpcError::InvalidWait),
                });
                if result.is_ok() {
                    detached = true;
                    break;
                }
            }
            assert!(detached, "Expired wait has no retained owner");
        }
        if expired {
            // Freeing an accepted reply slot must not admit another request
            // whose deadline was also due at this same clock sample.
            for object in self.channels.iter().flatten() {
                object.with(|object| {
                    if let KernelObject::Endpoint(endpoint) = object {
                        endpoint.borrow_mut().progress(&mut self.waits);
                    }
                });
            }
        }
    }

    fn commit_received(&mut self, tasks: &mut Tasks<'_, 'objects>) {
        loop {
            let next = self.waits.ready().find_map(|(wait, task, completion)| {
                (matches!(
                    completion,
                    Completion::Received { .. } | Completion::RepliedCaps { .. }
                ) && tasks.resources_mut(task).unwrap().received_wait != Some(wait))
                .then_some((wait, task, *completion))
            });
            let Some((wait, receiver, completion)) = next else {
                break;
            };
            let (sender, sender_wait, capabilities) = match completion {
                Completion::Received {
                    sender,
                    sender_wait,
                    capabilities,
                    ..
                } => (sender, sender_wait, capabilities),
                Completion::RepliedCaps {
                    sender,
                    capabilities,
                    ..
                } => (sender, wait, capabilities),
                _ => unreachable!(),
            };
            let transfer = self.take_transfer(sender_wait);
            let mut resources = tasks.resources_mut(receiver).unwrap();
            let batch = if let Some(transfer) = transfer {
                assert_eq!(transfer.sender, sender);
                transfer.batch
            } else {
                resources
                    .process
                    .handles
                    .prepare_transfer(&[])
                    .expect("Cannot prepare empty delivery")
            };
            assert_eq!(batch.len(), capabilities);
            self.out_of_order += usize::from(
                capabilities != 0 && (sender_wait.as_raw() as u32) < (wait.as_raw() as u32),
            );
            let reservation = resources
                .receive_slots
                .take()
                .expect("Receiver did not reserve handles");
            resources.received_handles = resources
                .process
                .handles
                .commit_transfer(reservation, batch)
                .unwrap_or_else(|_| panic!("Reserved receive commit invariant"));
            resources.received_wait = Some(wait);
            self.transferred += capabilities;
        }
    }

    pub(crate) fn settle(&mut self, tasks: &mut Tasks<'_, 'objects>) {
        // Acceptance owns its references even if the sender's completion sorts
        // first, or a task is about to die. Install grants before any cleanup.
        self.commit_received(tasks);
        let mut cursor = None;
        while let Some(id) = tasks.next_dying(cursor) {
            cursor = Some(id);
            self.task_died(id);
            tasks
                .end_thread(id)
                .expect("Ended thread membership invariant");
            let process_id = tasks.process_for(id).unwrap();
            let terminal = tasks.terminal(process_id);
            let resources = tasks.resources_mut(id).unwrap();
            let thread_status = ExitStatus {
                reason: resources.exit_reason,
                code: resources.exit_code,
            };
            if let Some(control) = resources.thread_control.as_ref() {
                control.with(|object| {
                    super::with_exit(object, |exit| {
                        if exit.result().is_none() {
                            exit.finish(thread_status, &mut self.waits)
                                .expect("Thread exit publication invariant");
                        }
                    })
                    .expect("Thread control kind invariant");
                });
            }
            if let Some(status) = terminal
                && let Some(control) = resources.process.control.as_ref()
            {
                control.with(|object| {
                    super::with_exit(object, |exit| {
                        if exit.result().is_none() {
                            exit.finish(status, &mut self.waits)
                                .expect("Process exit publication invariant");
                        }
                    })
                    .expect("Process control kind invariant");
                });
            }
        }
        self.commit_received(tasks);
        let mut cursor = None;
        while let Some(id) = tasks.next_dying(cursor) {
            cursor = Some(id);
            let mut resources = tasks.resources_mut(id).unwrap();
            if let Some(wait) = resources.wait.take() {
                self.words.remove(wait);
                self.remove_sleeper(wait);
                self.remove_observation(wait);
                if let Some(Completion::GateClosed { key, .. }) =
                    self.waits.abandon(wait).expect("Missing dying task wait")
                {
                    self.retire_gate_event(key);
                }
                drop(self.take_transfer(wait));
            }
            drop(resources.gate_loan.take());
            if let Some(reservation) = resources.receive_slots.take() {
                resources
                    .process
                    .handles
                    .release_reservation(reservation)
                    .unwrap_or_else(|_| panic!("Dying receive reservation invariant"));
            }
        }
        // Completing a transfer can release another channel's final grant.
        // Closing that channel may complete more waits in the same settlement.
        loop {
            // No arena or channel lock is held while publishing runnable tasks.
            while let Some((wait, id)) = self.waits.next_ready() {
                assert_eq!(tasks.state(id), Ok(TaskState::Blocked));
                let mut resources = tasks.resources_mut(id).unwrap();
                assert_eq!(resources.wait.take(), Some(wait));
                let completion = self.waits.take(wait).expect("Missing IPC completion");
                self.words.remove(wait);
                self.remove_sleeper(wait);
                self.remove_observation(wait);
                drop(self.take_transfer(wait));
                if let Some(reservation) = resources.receive_slots.take() {
                    resources
                        .process
                        .handles
                        .release_reservation(reservation)
                        .unwrap_or_else(|_| panic!("Failed receive reservation invariant"));
                }
                drop(resources.gate_loan.take());
                let thread = &mut resources.thread;
                let context = &mut thread.context;
                context.rax = 0;
                context.rdx = 0;
                match completion {
                    Completion::Sent | Completion::Notified | Completion::Slept => {}
                    Completion::ProcessExited(status) => {
                        context.rdx = status.code;
                        context.r8 = status.reason as u64;
                    }
                    Completion::Received {
                        message,
                        capabilities,
                        ..
                    }
                    | Completion::RepliedCaps {
                        message,
                        capabilities,
                        ..
                    } => {
                        assert_eq!(thread.received_wait.take(), Some(wait));
                        let reply = match completion {
                            Completion::Received { reply, .. } => reply,
                            _ => None,
                        };
                        context.rdx = reply.map_or(0, |reply| reply.as_raw());
                        [context.r8, context.r9, context.r10, context.r11] = message.0;
                        context.r12 =
                            thread.received_handles[0].map_or(0, |handle| handle.as_raw());
                        context.r13 =
                            thread.received_handles[1].map_or(0, |handle| handle.as_raw());
                        context.r14 = capabilities as u64;
                        if let Completion::Received { label, .. } = completion {
                            context.rcx = label;
                            context.r15 = kryo_interfaces::syscall::ReceiveEvent::Message as u64;
                        }
                        thread.received_handles = [None; 2];
                    }
                    Completion::GateClosed { label, key } => {
                        self.retire_gate_event(key);
                        [context.r8, context.r9, context.r10, context.r11] = [0; 4];
                        context.r12 = 0;
                        context.r13 = 0;
                        context.r14 = 0;
                        context.rcx = label;
                        context.r15 = kryo_interfaces::syscall::ReceiveEvent::GateClosed as u64;
                    }
                    Completion::Replied(message) => {
                        [context.r8, context.r9, context.r10, context.r11] = message.0;
                    }
                    Completion::Failed { error, delivery } => {
                        self.accepted_failures += usize::from(delivery == Delivery::Accepted);
                        context.rax = map_error(error).status();
                        context.rdx = u64::from(delivery == Delivery::Accepted);
                    }
                }
                tasks.wake(id).expect("Cannot wake IPC completion");
            }
            let channels_retired = self.retire_channels();
            let gates_changed = self.settle_gates();
            if !channels_retired && !gates_changed {
                break;
            }
        }
    }

    fn retire_channels(&mut self) -> bool {
        let mut retired = false;
        for slot in &mut self.channels {
            let Some(object) = slot.as_ref() else {
                continue;
            };
            let device_owned = self
                .device_signal
                .as_ref()
                .is_some_and(|signal| signal.id() == object.id());
            if device_owned {
                continue;
            }
            let unreferenced = object.reference_count() == 1;
            let closed = object.with(|object| match object {
                KernelObject::Endpoint(endpoint) => {
                    let mut endpoint = endpoint.borrow_mut();
                    if unreferenced {
                        endpoint.close(&mut self.waits);
                    }
                    endpoint.is_closed()
                }
                KernelObject::Notification(notification) => {
                    let mut notification = notification.borrow_mut();
                    if unreferenced {
                        notification.close(&mut self.waits);
                    }
                    notification.is_closed()
                }
                _ => false,
            });
            if closed {
                self.gates_dirty = true;
                // Never drop a final payload owner under ObjectRef::with.
                // A stale external grant keeps the closed payload charged.
                *slot = None;
                retired = true;
            }
        }
        retired
    }

    pub(crate) fn wake_words(&mut self, key: PrivateWordKey, maximum: usize) -> usize {
        self.words.wake(key, maximum, |wait| {
            self.waits
                .complete(wait, Completion::Notified)
                .expect("Private word wake invariant");
        })
    }

    /// Detach registrations before their backing can retire or be reused.
    pub(crate) fn invalidate_words(&mut self, space: u64, start: u64, end: u64) {
        self.words.remove_matching(
            |key| key.space() == space && start <= key.address() && key.address() < end,
            |wait| {
                self.waits
                    .complete(
                        wait,
                        Completion::Failed {
                            error: IpcError::Cancelled,
                            delivery: Delivery::NotDelivered,
                        },
                    )
                    .expect("Private word invalidation invariant");
            },
        );
    }

    fn task_died(&mut self, id: TaskId) {
        for slot in &mut self.observations {
            if let Some((wait, object)) = slot.as_ref()
                && self.waits.task(*wait) == Ok(id)
            {
                object.with(|object| {
                    super::with_exit(object, |exit| exit.task_died(id, &mut self.waits))
                        .expect("Observation object kind invariant");
                });
                *slot = None;
            }
        }
        for object in self.channels.iter().flatten() {
            object.with(|object| match object {
                KernelObject::Endpoint(endpoint) => {
                    endpoint.borrow_mut().task_died(id, &mut self.waits)
                }
                KernelObject::Notification(notification) => {
                    notification.borrow_mut().task_died(id, &mut self.waits)
                }
                _ => {}
            });
        }
    }

    #[cfg(feature = "boot-tests")]
    pub(crate) fn transfer_count(&self) -> usize {
        self.transfers.iter().flatten().count()
    }
}

pub(crate) fn map_error(error: IpcError) -> Error {
    match error {
        IpcError::Closed | IpcError::PeerClosed => Error::PeerClosed,
        IpcError::Capacity => Error::OutOfResources,
        IpcError::PermissionDenied => Error::PermissionDenied,
        IpcError::InvalidReply => Error::InvalidReply,
        IpcError::Cancelled => Error::Cancelled,
        IpcError::TimedOut => Error::TimedOut,
        IpcError::InvalidWait | IpcError::AlreadyWaiting => Error::InvalidArgument,
    }
}
