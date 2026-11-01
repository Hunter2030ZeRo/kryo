use crate::object::{ObjectError, ObjectKind, ObjectRef};
use crate::storage::Slots;
use core::ops::BitOr;

mod transfer;
pub use transfer::{
    HandleReservation, ReservationError, TransferBatch, TransferError, TransferSpec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(u64);

impl Handle {
    pub const INVALID: Self = Self(0);
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    fn new(index: usize, generation: u32) -> Option<Self> {
        let slot = u32::try_from(index).ok()?.checked_add(1)?;
        Some(Self((u64::from(generation) << 32) | u64::from(slot)))
    }

    fn parts(self) -> Option<(usize, u32)> {
        let index = (self.0 as u32).checked_sub(1)? as usize;
        let generation = (self.0 >> 32) as u32;
        (generation != 0).then_some((index, generation))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rights(u64);

impl Rights {
    pub const INSPECT: Self = Self(1);
    pub const CONTROL: Self = Self(2);
    pub const DUP: Self = Self(4);
    pub const SEND: Self = Self(8);
    pub const RECEIVE: Self = Self(16);
    pub const WAIT: Self = Self(32);
    pub const SIGNAL: Self = Self(64);
    pub const READ: Self = Self(128);
    pub const WRITE: Self = Self(256);
    pub const MAP: Self = Self(512);
    pub const TRANSFER: Self = Self(1024);
    pub const EXECUTE: Self = Self(2048);
    pub const CREATE: Self = Self(4096);

    pub const fn from_bits(bits: u64) -> Option<Self> {
        if bits & !8191 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u64 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Rights {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityError {
    InvalidHandle,
    WrongKind,
    PermissionDenied,
    Capacity,
    ReferenceLimit,
}

pub struct InsertError<T> {
    pub error: CapabilityError,
    pub object: T,
}

impl<T> core::fmt::Debug for InsertError<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

struct Entry<'objects, T, const OBJECTS: usize> {
    object: ObjectRef<'objects, T, OBJECTS>,
    rights: Rights,
}

struct Slot<'objects, T, const OBJECTS: usize> {
    generation: u32,
    retired: bool,
    reserved: bool,
    entry: Option<Entry<'objects, T, OBJECTS>>,
}

/// Process-local handles. Exclusive mutation serializes insert, close, and duplication.
pub struct HandleTable<'objects, T, const OBJECTS: usize, const HANDLES: usize> {
    domain: u64,
    slots: Slots<Slot<'objects, T, OBJECTS>, HANDLES>,
}

impl<T, const OBJECTS: usize, const HANDLES: usize> Default
    for HandleTable<'_, T, OBJECTS, HANDLES>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'objects, T, const OBJECTS: usize, const HANDLES: usize>
    HandleTable<'objects, T, OBJECTS, HANDLES>
{
    pub const fn new() -> Self {
        Self {
            domain: 0,
            slots: Slots::new(
                [const {
                    Slot {
                        generation: 1,
                        retired: false,
                        reserved: false,
                        entry: None,
                    }
                }; HANDLES],
            ),
        }
    }

    pub const fn growing() -> Self {
        Self::limited(u32::MAX as usize)
    }

    pub const fn limited(limit: usize) -> Self {
        Self {
            domain: 0,
            slots: Slots::limited(limit),
        }
    }

    fn ensure_free(&mut self, count: usize) -> Result<(), CapabilityError> {
        let free = self
            .slots
            .iter()
            .filter(|slot| !slot.retired && !slot.reserved && slot.entry.is_none())
            .count();
        let length = self
            .slots
            .len()
            .checked_add(count.saturating_sub(free))
            .ok_or(CapabilityError::Capacity)?;
        self.slots
            .reserve_len(length, || Slot {
                generation: 1,
                retired: false,
                reserved: false,
                entry: None,
            })
            .map_err(|_| CapabilityError::Capacity)
    }

    pub fn insert(
        &mut self,
        object: ObjectRef<'objects, T, OBJECTS>,
        rights: Rights,
    ) -> Result<Handle, InsertError<ObjectRef<'objects, T, OBJECTS>>> {
        if self.ensure_free(1).is_err() {
            return Err(InsertError {
                error: CapabilityError::Capacity,
                object,
            });
        }
        let Some(index) = self.free_slot() else {
            return Err(InsertError {
                error: CapabilityError::Capacity,
                object,
            });
        };
        let Some(handle) = Handle::new(index, self.slots[index].generation) else {
            return Err(InsertError {
                error: CapabilityError::Capacity,
                object,
            });
        };
        self.slots[index].entry = Some(Entry { object, rights });
        Ok(handle)
    }

    pub fn lookup(
        &self,
        handle: Handle,
        kind: ObjectKind,
        rights: Rights,
    ) -> Result<ObjectRef<'objects, T, OBJECTS>, CapabilityError> {
        let entry = self.entry(handle)?;
        if entry.object.kind() != kind {
            return Err(CapabilityError::WrongKind);
        }
        if !entry.rights.contains(rights) {
            return Err(CapabilityError::PermissionDenied);
        }
        entry.object.try_clone().map_err(map_object_error)
    }

    /// Describes only this table's live handle; this grants no object reference.
    pub fn describe(&self, handle: Handle) -> Result<(ObjectKind, Rights), CapabilityError> {
        let entry = self.entry(handle)?;
        Ok((entry.object.kind(), entry.rights))
    }

    pub fn duplicate(&mut self, handle: Handle, rights: Rights) -> Result<Handle, CapabilityError> {
        let entry = self.entry(handle)?;
        if !entry.rights.contains(Rights::DUP) || !entry.rights.contains(rights) {
            return Err(CapabilityError::PermissionDenied);
        }
        self.ensure_free(1)?;
        let index = self.free_slot().ok_or(CapabilityError::Capacity)?;
        let object = self
            .entry(handle)?
            .object
            .try_clone()
            .map_err(map_object_error)?;
        let result =
            Handle::new(index, self.slots[index].generation).ok_or(CapabilityError::Capacity)?;
        self.slots[index].entry = Some(Entry { object, rights });
        Ok(result)
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), CapabilityError> {
        self.entry(handle)?;
        let (index, _) = handle.parts().ok_or(CapabilityError::InvalidHandle)?;
        let slot = &mut self.slots[index];
        let removed = slot.entry.take();
        if slot.generation == u32::MAX {
            slot.retired = true;
        } else {
            slot.generation += 1;
        }
        drop(removed);
        Ok(())
    }

    pub fn live_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.entry.is_some())
            .count()
    }

    fn free_slot(&self) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| !slot.retired && !slot.reserved && slot.entry.is_none())
    }

    fn entry(&self, handle: Handle) -> Result<&Entry<'objects, T, OBJECTS>, CapabilityError> {
        let (index, generation) = handle.parts().ok_or(CapabilityError::InvalidHandle)?;
        let slot = self
            .slots
            .get(index)
            .ok_or(CapabilityError::InvalidHandle)?;
        if slot.retired || slot.generation != generation {
            return Err(CapabilityError::InvalidHandle);
        }
        slot.entry.as_ref().ok_or(CapabilityError::InvalidHandle)
    }
}

fn map_object_error(error: ObjectError) -> CapabilityError {
    match error {
        ObjectError::Capacity => CapabilityError::Capacity,
        ObjectError::ReferenceLimit => CapabilityError::ReferenceLimit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ObjectTable;

    #[test]
    fn generation_wrap_retires_handle_slots() {
        let objects = ObjectTable::<(), 2>::new();
        let mut handles = HandleTable::<(), 2, 1>::new();
        handles.slots[0].generation = u32::MAX;
        let object = objects.create(ObjectKind::TaskControl, ()).unwrap();
        let handle = handles.insert(object, Rights::INSPECT).unwrap();
        handles.close(handle).unwrap();
        let object = objects.create(ObjectKind::TaskControl, ()).unwrap();
        let error = handles.insert(object, Rights::INSPECT).unwrap_err();
        assert_eq!(error.error, CapabilityError::Capacity);
        drop(error.object);
        assert_eq!(objects.live_count(), 0);
    }
}
