use super::*;
use crate::runtime::{Batch, Handles};
use kryo_core::capability::{Handle, Rights};
use kryo_core::ipc::GateKey;
use kryo_core::object::ObjectKind;

pub(super) struct GateRecord<'objects> {
    gate: Object<'objects>,
    target: Object<'objects>,
    label: u64,
    event: usize,
    closing: bool,
}

impl<'objects> IpcRuntime<'objects> {
    pub(crate) fn create_gate(
        &mut self,
        owner: TaskId,
        root: Object<'objects>,
        label: u64,
        handles: &mut Handles<'objects>,
    ) -> Result<Handle, Error> {
        if label == 0 {
            return Err(Error::InvalidArgument);
        }
        root.with(|object| match object {
            KernelObject::Endpoint(endpoint) => {
                endpoint.borrow().check_gate_owner(owner).map_err(map_error)
            }
            _ => Err(Error::WrongType),
        })?;
        // Active and pending labels remain reserved until delivery or root closure.
        if self
            .gates
            .iter()
            .flatten()
            .any(|record| record.target.id() == root.id() && record.label == label)
        {
            return Err(Error::Busy);
        }
        let lease = root.with(|object| match object {
            KernelObject::Endpoint(endpoint) => {
                endpoint.borrow().gate_lease(owner).map_err(map_error)
            }
            _ => Err(Error::WrongType),
        })?;
        let arena = self.arena.ok_or(Error::InvalidCall)?;
        let index = self
            .gates
            .vacant(Option::is_none, || None)
            .map_err(|_| Error::OutOfResources)?;
        let reservation = handles
            .reserve(1)
            .map_err(super::super::native::map_error)?;
        let prepared = (|| {
            let gate = arena
                .create(
                    ObjectKind::EndpointGate,
                    KernelObject::EndpointGate {
                        registry: index,
                        label,
                        _lease: lease,
                    },
                )
                .map_err(|_| Error::OutOfResources)?;
            let registered = gate.try_clone().map_err(|_| Error::OutOfResources)?;
            let key = GateKey {
                registry: index,
                object: gate.id().as_raw(),
            };
            let event = root.with(|object| {
                let KernelObject::Endpoint(endpoint) = object else {
                    unreachable!()
                };
                endpoint
                    .borrow_mut()
                    .reserve_gate(owner, label, key)
                    .map_err(map_error)
            })?;
            self.gates[index] = Some(GateRecord {
                gate: registered,
                target: root,
                label,
                event,
                closing: false,
            });
            Ok(gate)
        })();
        let gate = match prepared {
            Ok(gate) => gate,
            Err(error) => {
                handles
                    .release_reservation(reservation)
                    .expect("Gate creation reservation invariant");
                return Err(error);
            }
        };
        let key = GateKey {
            registry: index,
            object: gate.id().as_raw(),
        };
        match handles.commit_transfer(
            reservation,
            Batch::from_object(gate, Rights::SEND | Rights::DUP | Rights::TRANSFER),
        ) {
            Ok(result) => Ok(result[0].expect("Gate handle invariant")),
            Err(failure) => {
                let record = self.gates[index].take().unwrap();
                record.target.with(|object| {
                    let KernelObject::Endpoint(endpoint) = object else {
                        unreachable!()
                    };
                    endpoint.borrow_mut().discard_gate(record.event, key);
                });
                handles
                    .release_reservation(failure.reservation)
                    .expect("Gate rollback invariant");
                Err(super::super::native::map_error(failure.error))
            }
        }
    }

    pub(crate) fn gate_target(
        &self,
        gate: &Object<'objects>,
    ) -> Result<(Object<'objects>, u64), Error> {
        let (index, label) = gate.with(|object| match object {
            KernelObject::EndpointGate {
                registry, label, ..
            } => (*registry, *label),
            _ => unreachable!("Gate object kind invariant"),
        });
        let record = self
            .gates
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::PeerClosed)?;
        if record.gate.id() != gate.id() || record.label != label || record.closing {
            return Err(Error::PeerClosed);
        }
        record
            .target
            .try_clone()
            .map(|root| (root, label))
            .map_err(|_| Error::OutOfResources)
    }

    pub(super) fn retire_gate_event(&mut self, key: GateKey) {
        if let Some(record) = self.gates.get_mut(key.registry)
            && record
                .as_ref()
                .is_some_and(|record| record.gate.id().as_raw() == key.object)
        {
            assert!(record.as_ref().unwrap().closing);
            *record = None;
        }
    }

    pub(super) fn settle_gates(&mut self) -> bool {
        let Some(arena) = self.arena else {
            return false;
        };
        let epoch = arena.gate_epoch();
        if !self.gates_dirty && epoch != u64::MAX && self.gate_epoch == epoch {
            return false;
        }
        // Drops during the scan must remain visible to the next fixed-point pass.
        self.gate_epoch = epoch;
        self.gates_dirty = false;
        #[cfg(feature = "boot-tests")]
        {
            self.gate_scans += 1;
        }
        let mut changed = false;
        for (index, slot) in self.gates.iter_mut().enumerate() {
            let Some(record) = slot.as_mut() else {
                continue;
            };
            let closed = record.target.with(|object| {
                let KernelObject::Endpoint(endpoint) = object else {
                    unreachable!()
                };
                endpoint.borrow().is_closed()
            });
            if closed {
                *slot = None;
                changed = true;
            } else if !record.closing && record.gate.reference_count() == 1 {
                record.closing = true;
                let key = GateKey {
                    registry: index,
                    object: record.gate.id().as_raw(),
                };
                record.target.with(|object| {
                    let KernelObject::Endpoint(endpoint) = object else {
                        unreachable!()
                    };
                    endpoint
                        .borrow_mut()
                        .close_gate(record.event, key, &mut self.waits);
                });
                changed = true;
            }
        }
        changed || (epoch != u64::MAX && arena.gate_epoch() != epoch)
    }

    #[cfg(feature = "boot-tests")]
    pub(crate) fn gate_count(&self) -> usize {
        self.gates.iter().flatten().count()
    }
}

#[cfg(feature = "boot-tests")]
pub(crate) mod checks;
