use crate::storage::Slots;
use spin::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    ResourceBudget,
    LaunchManifest,
    StartupRecord,
    ChannelFactory,
    TaskControl,
    Memory,
    Endpoint,
    Notification,
    ProcessControl,
    ThreadControl,
    Image,
    DebugConsole,
    Device,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectId(u64);

impl ObjectId {
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    fn new(slot: usize, generation: u32) -> Option<Self> {
        let slot = u32::try_from(slot).ok()?.checked_add(1)?;
        Some(Self((u64::from(generation) << 32) | u64::from(slot)))
    }

    fn parts(self) -> (usize, u32) {
        ((self.0 as u32 - 1) as usize, (self.0 >> 32) as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectError {
    Capacity,
    ReferenceLimit,
}

pub struct ObjectCreateError<T> {
    pub error: ObjectError,
    pub value: T,
}

impl<T> core::fmt::Debug for ObjectCreateError<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

struct Slot<T> {
    generation: u32,
    retired: bool,
    references: usize,
    kind: ObjectKind,
    value: Option<T>,
}

/// Arena locking requires callers to exclude same-CPU IRQ reentry; NMI use is unsupported.
pub struct ObjectTable<T, const OBJECTS: usize> {
    slots: Mutex<Slots<Slot<T>, OBJECTS>>,
}

impl<T, const OBJECTS: usize> Drop for ObjectTable<T, OBJECTS> {
    fn drop(&mut self) {
        // Forgotten references may still retain hardware mappings. Losing the
        // arena must not destroy their backing memory.
        for slot in self.slots.get_mut() {
            if slot.references != 0 {
                core::mem::forget(slot.value.take());
            }
        }
    }
}

impl<T, const OBJECTS: usize> Default for ObjectTable<T, OBJECTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const OBJECTS: usize> ObjectTable<T, OBJECTS> {
    pub const fn new() -> Self {
        Self {
            slots: Mutex::new(Slots::new(
                [const {
                    Slot {
                        generation: 1,
                        retired: false,
                        references: 0,
                        kind: ObjectKind::TaskControl,
                        value: None,
                    }
                }; OBJECTS],
            )),
        }
    }

    pub const fn growing() -> Self {
        Self::limited(u32::MAX as usize)
    }

    pub const fn limited(limit: usize) -> Self {
        Self {
            slots: Mutex::new(Slots::limited(limit)),
        }
    }

    pub fn create(
        &self,
        kind: ObjectKind,
        value: T,
    ) -> Result<ObjectRef<'_, T, OBJECTS>, ObjectCreateError<T>> {
        let mut slots = self.slots.lock();
        let Ok(index) = slots.vacant(
            |slot| !slot.retired && slot.value.is_none(),
            || Slot {
                generation: 1,
                retired: false,
                references: 0,
                kind: ObjectKind::TaskControl,
                value: None,
            },
        ) else {
            return Err(ObjectCreateError {
                error: ObjectError::Capacity,
                value,
            });
        };
        let slot = &mut slots[index];
        let Some(id) = ObjectId::new(index, slot.generation) else {
            return Err(ObjectCreateError {
                error: ObjectError::Capacity,
                value,
            });
        };
        slot.kind = kind;
        slot.references = 1;
        slot.value = Some(value);
        Ok(ObjectRef {
            table: self,
            id,
            kind,
        })
    }

    pub fn live_count(&self) -> usize {
        self.slots
            .lock()
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
    }
}

/// A strong arena reference. Its ID is descriptive; the reference retains ownership.
pub struct ObjectRef<'objects, T, const OBJECTS: usize> {
    table: &'objects ObjectTable<T, OBJECTS>,
    id: ObjectId,
    kind: ObjectKind,
}

impl<T, const OBJECTS: usize> core::fmt::Debug for ObjectRef<'_, T, OBJECTS> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ObjectRef")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .finish()
    }
}

impl<T, const OBJECTS: usize> ObjectRef<'_, T, OBJECTS> {
    pub const fn id(&self) -> ObjectId {
        self.id
    }
    pub const fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub fn try_clone(&self) -> Result<Self, ObjectError> {
        let mut slots = self.table.slots.lock();
        let (index, generation) = self.id.parts();
        let slot = &mut slots[index];
        assert_eq!(slot.generation, generation, "Object generation invariant");
        assert!(slot.value.is_some(), "Object reference invariant");
        slot.references = slot
            .references
            .checked_add(1)
            .ok_or(ObjectError::ReferenceLimit)?;
        Ok(Self {
            table: self.table,
            id: self.id,
            kind: self.kind,
        })
    }

    /// The callback runs under the object lock; it must not reenter this arena or block.
    pub fn with<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        let slots = self.table.slots.lock();
        let (index, generation) = self.id.parts();
        let slot = &slots[index];
        assert_eq!(slot.generation, generation, "Object generation invariant");
        read(slot.value.as_ref().expect("Object reference invariant"))
    }

    pub fn reference_count(&self) -> usize {
        let slots = self.table.slots.lock();
        let (index, _) = self.id.parts();
        slots[index].references
    }
}

impl<T, const OBJECTS: usize> Drop for ObjectRef<'_, T, OBJECTS> {
    fn drop(&mut self) {
        let removed = {
            let mut slots = self.table.slots.lock();
            let (index, generation) = self.id.parts();
            let slot = &mut slots[index];
            assert_eq!(slot.generation, generation, "Object generation invariant");
            assert!(slot.references > 0, "Object reference invariant");
            slot.references -= 1;
            if slot.references != 0 {
                return;
            }
            let removed = slot.value.take();
            if slot.generation == u32::MAX {
                slot.retired = true;
            } else {
                slot.generation += 1;
            }
            removed
        };
        // Payload destruction may need other locks; the arena lock is no longer held.
        drop(removed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_wrap_retires_object_slots() {
        let objects = ObjectTable::<(), 1>::new();
        objects.slots.lock()[0].generation = u32::MAX;
        let object = objects.create(ObjectKind::TaskControl, ()).unwrap();
        let id = object.id();
        drop(object);
        assert_eq!(
            objects
                .create(ObjectKind::TaskControl, ())
                .unwrap_err()
                .error,
            ObjectError::Capacity
        );
        assert_eq!(id.as_raw() >> 32, u64::from(u32::MAX));
    }

    #[test]
    fn reference_overflow_preserves_the_original_reference() {
        let objects = ObjectTable::<(), 1>::new();
        let object = objects.create(ObjectKind::TaskControl, ()).unwrap();
        objects.slots.lock()[0].references = usize::MAX;
        assert_eq!(object.try_clone().unwrap_err(), ObjectError::ReferenceLimit);
        objects.slots.lock()[0].references = 1;
        drop(object);
        assert_eq!(objects.live_count(), 0);
    }
}
