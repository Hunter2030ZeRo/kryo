use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity;

/// Inline tables keep deterministic bounds; growing tables allocate only when
/// insertion needs a new slot. Indices never change, including vacant slots.
pub enum Slots<T, const INLINE: usize> {
    Inline([T; INLINE]),
    Growing { values: Vec<T>, limit: usize },
}

impl<T, const INLINE: usize> Slots<T, INLINE> {
    pub const fn new(values: [T; INLINE]) -> Self {
        Self::Inline(values)
    }

    pub const fn growing() -> Self {
        Self::limited(usize::MAX)
    }

    pub const fn limited(limit: usize) -> Self {
        Self::Growing {
            values: Vec::new(),
            limit,
        }
    }

    pub const fn is_growing(&self) -> bool {
        matches!(self, Self::Growing { .. })
    }

    pub fn try_reserve(&mut self, additional: usize) -> Result<(), Capacity> {
        match self {
            Self::Inline(_) => (additional == 0).then_some(()).ok_or(Capacity),
            Self::Growing { values, limit } => {
                if additional > limit.saturating_sub(values.len()) {
                    return Err(Capacity);
                }
                values.try_reserve(additional).map_err(|_| Capacity)
            }
        }
    }

    pub fn try_push(&mut self, value: T) -> Result<usize, T> {
        if self.try_reserve(1).is_err() {
            return Err(value);
        }
        let Self::Growing { values, .. } = self else {
            unreachable!()
        };
        let index = values.len();
        values.push(value);
        Ok(index)
    }

    pub fn reserve_len(
        &mut self,
        length: usize,
        mut initialize: impl FnMut() -> T,
    ) -> Result<(), Capacity> {
        let additional = length.saturating_sub(self.len());
        self.try_reserve(additional)?;
        if let Self::Growing { values, .. } = self {
            while values.len() < length {
                values.push(initialize());
            }
        }
        Ok(())
    }

    /// Releases growing storage and all values. Generational registries must
    /// retain their slot histories until the entire registry retires instead.
    pub fn release(&mut self) {
        if let Self::Growing { values, .. } = self {
            drop(core::mem::take(values));
        }
    }

    pub fn vacant(
        &mut self,
        free: impl Fn(&T) -> bool,
        initialize: impl FnOnce() -> T,
    ) -> Result<usize, Capacity> {
        if let Some(index) = self.iter().position(free) {
            return Ok(index);
        }
        self.try_push(initialize()).map_err(|_| Capacity)
    }
}

impl<T, const INLINE: usize> Deref for Slots<T, INLINE> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match self {
            Self::Inline(values) => values,
            Self::Growing { values, .. } => values,
        }
    }
}

impl<T, const INLINE: usize> DerefMut for Slots<T, INLINE> {
    fn deref_mut(&mut self) -> &mut [T] {
        match self {
            Self::Inline(values) => values,
            Self::Growing { values, .. } => values,
        }
    }
}

impl<'a, T, const INLINE: usize> IntoIterator for &'a Slots<T, INLINE> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, const INLINE: usize> IntoIterator for &'a mut Slots<T, INLINE> {
    type Item = &'a mut T;
    type IntoIter = core::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
