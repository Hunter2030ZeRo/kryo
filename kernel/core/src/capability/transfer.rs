use super::{CapabilityError, Entry, Handle, HandleTable, Rights, map_object_error};
use crate::object::ObjectRef;
use core::sync::atomic::{AtomicU64, Ordering};

const MAX_TRANSFERS: usize = 2;
static NEXT_DOMAIN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferSpec {
    pub handle: Handle,
    pub rights: Rights,
}

/// Staged reduced-rights strong references; source handles remain unchanged.
#[must_use]
pub struct TransferBatch<'objects, T, const OBJECTS: usize> {
    entries: [Option<Entry<'objects, T, OBJECTS>>; MAX_TRANSFERS],
    length: usize,
}

impl<'objects, T, const OBJECTS: usize> TransferBatch<'objects, T, OBJECTS> {
    /// Stages one trusted kernel-owned reference with newly granted rights.
    /// This mints authority; user-handle delegation must use prepare_transfer
    /// to validate source TRANSFER rights and attenuation instead.
    pub fn from_object(object: ObjectRef<'objects, T, OBJECTS>, rights: Rights) -> Self {
        Self {
            entries: [Some(Entry { object, rights }), None],
            length: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }
}

impl<T, const OBJECTS: usize> core::fmt::Debug for TransferBatch<'_, T, OBJECTS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransferBatch")
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

/// Reserved receiver slots. Dropping this token leaks capacity; explicitly
/// commit or release it before discarding a live receiver's wait.
/// A move-stable table identity prevents installing a token in another table.
#[must_use]
#[derive(Debug)]
pub struct HandleReservation {
    domain: u64,
    slots: [Handle; MAX_TRANSFERS],
    capacity: usize,
}

impl HandleReservation {
    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

#[derive(Debug)]
pub struct ReservationError {
    pub error: CapabilityError,
    pub reservation: HandleReservation,
}

pub struct TransferError<'objects, T, const OBJECTS: usize> {
    pub error: CapabilityError,
    pub reservation: HandleReservation,
    pub batch: TransferBatch<'objects, T, OBJECTS>,
}

impl<T, const OBJECTS: usize> core::fmt::Debug for TransferError<'_, T, OBJECTS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransferError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl<'objects, T, const OBJECTS: usize, const HANDLES: usize>
    HandleTable<'objects, T, OBJECTS, HANDLES>
{
    /// Copy delegation requires TRANSFER, not DUP. The prepared batch retains
    /// references independently of source close, cancellation, or task death.
    /// Call outside ObjectRef::with: cloning may lock the same object arena.
    pub fn prepare_transfer(
        &self,
        specs: &[TransferSpec],
    ) -> Result<TransferBatch<'objects, T, OBJECTS>, CapabilityError> {
        if specs.len() > MAX_TRANSFERS {
            return Err(CapabilityError::Capacity);
        }
        let mut batch = TransferBatch {
            entries: [const { None }; MAX_TRANSFERS],
            length: specs.len(),
        };
        for (index, spec) in specs.iter().enumerate() {
            let entry = self.entry(spec.handle)?;
            if !entry.rights.contains(Rights::TRANSFER)
                || !entry.rights.contains(spec.rights)
                || spec.rights.bits() & (Rights::RECEIVE | Rights::WAIT).bits() != 0
            {
                return Err(CapabilityError::PermissionDenied);
            }
            let object = entry.object.try_clone().map_err(map_object_error)?;
            batch.entries[index] = Some(Entry {
                object,
                rights: spec.rights,
            });
        }
        Ok(batch)
    }

    /// Reserve receiver capacity before publishing its IPC receive operation.
    pub fn reserve(&mut self, count: usize) -> Result<HandleReservation, CapabilityError> {
        if count > MAX_TRANSFERS {
            return Err(CapabilityError::Capacity);
        }
        self.ensure_free(count)?;
        let mut slots = [Handle::INVALID; MAX_TRANSFERS];
        let mut found = 0;
        for (index, slot) in self.slots.iter().enumerate() {
            if found == count {
                break;
            }
            if !slot.retired && !slot.reserved && slot.entry.is_none() {
                slots[found] =
                    Handle::new(index, slot.generation).ok_or(CapabilityError::Capacity)?;
                found += 1;
            }
        }
        if found != count {
            return Err(CapabilityError::Capacity);
        }
        let mut charges = [const { None }; MAX_TRANSFERS];
        for charge in charges.iter_mut().take(count) {
            *charge = self.charge_slot()?;
        }
        if self.domain == 0 {
            self.domain = claim_domain(&NEXT_DOMAIN)?;
        }
        for (index, handle) in slots.into_iter().take(count).enumerate() {
            let slot = &mut self.slots[handle.parts().unwrap().0];
            slot.reserved = true;
            slot.charge = charges[index].take();
        }
        Ok(HandleReservation {
            domain: self.domain,
            slots,
            capacity: count,
        })
    }

    /// Installs all staged references or returns both owners unchanged. This
    /// operation is infallible after reservation/count validation and never clones.
    pub fn commit_transfer(
        &mut self,
        reservation: HandleReservation,
        mut batch: TransferBatch<'objects, T, OBJECTS>,
    ) -> Result<[Option<Handle>; MAX_TRANSFERS], TransferError<'objects, T, OBJECTS>> {
        let valid = self.validate_reservation(&reservation).and_then(|()| {
            if batch.len() <= reservation.capacity {
                Ok(())
            } else {
                Err(CapabilityError::Capacity)
            }
        });
        if let Err(error) = valid {
            return Err(TransferError {
                error,
                reservation,
                batch,
            });
        }
        let mut received = [None; MAX_TRANSFERS];
        for index in 0..reservation.capacity {
            let handle = reservation.slots[index];
            let slot = &mut self.slots[handle.parts().unwrap().0];
            slot.reserved = false;
            if index < batch.len() {
                slot.entry = batch.entries[index].take();
                received[index] = Some(handle);
            } else {
                slot.charge = None;
                advance_generation(slot);
            }
        }
        Ok(received)
    }

    pub fn release_reservation(
        &mut self,
        reservation: HandleReservation,
    ) -> Result<(), ReservationError> {
        if let Err(error) = self.validate_reservation(&reservation) {
            return Err(ReservationError { error, reservation });
        }
        for handle in reservation.slots.into_iter().take(reservation.capacity) {
            let slot = &mut self.slots[handle.parts().unwrap().0];
            slot.reserved = false;
            slot.charge = None;
            advance_generation(slot);
        }
        Ok(())
    }

    pub fn reserved_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.reserved).count()
    }

    fn validate_reservation(&self, reservation: &HandleReservation) -> Result<(), CapabilityError> {
        if reservation.domain == 0 || reservation.domain != self.domain {
            return Err(CapabilityError::InvalidHandle);
        }
        for handle in reservation.slots.into_iter().take(reservation.capacity) {
            let (index, generation) = handle.parts().ok_or(CapabilityError::InvalidHandle)?;
            let slot = self
                .slots
                .get(index)
                .ok_or(CapabilityError::InvalidHandle)?;
            if slot.retired
                || !slot.reserved
                || slot.generation != generation
                || slot.entry.is_some()
            {
                return Err(CapabilityError::InvalidHandle);
            }
        }
        Ok(())
    }
}

fn claim_domain(counter: &AtomicU64) -> Result<u64, CapabilityError> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .map_err(|_| CapabilityError::Capacity)
}

fn advance_generation<T, const OBJECTS: usize>(slot: &mut super::Slot<'_, T, OBJECTS>) {
    if slot.generation == u32::MAX {
        slot.retired = true;
    } else {
        slot.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_domains_never_wrap_or_reuse_identity() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(claim_domain(&counter), Ok(u64::MAX - 1));
        assert_eq!(claim_domain(&counter), Err(CapabilityError::Capacity));
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn released_max_generation_reservation_retires_its_slot() {
        let mut handles = HandleTable::<(), 1, 1>::new();
        handles.slots[0].generation = u32::MAX;
        let reservation = handles.reserve(1).unwrap();
        handles.release_reservation(reservation).unwrap();
        assert_eq!(handles.reserve(1).unwrap_err(), CapabilityError::Capacity);
    }
}
