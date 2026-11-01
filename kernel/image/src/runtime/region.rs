use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};
use kryo_core::mm::frame::OwnedFrame;
use kryo_hal::{DirectMap, PagingError};

use crate::boot::memory::BootFramePool;

/// Retains every added table frame through branch retirement. The initial
/// three pages remain owned by their boot-image or caller allocation owners.
pub(crate) struct KernelRegion {
    inner: kryo_hal::KernelRegion,
    tables: Vec<OwnedFrame<'static>>,
    pool: Option<&'static BootFramePool>,
    direct: DirectMap,
    #[cfg_attr(
        not(feature = "boot-tests"),
        expect(dead_code, reason = "Normal boot retains the platform region")
    )]
    detached: bool,
}

impl KernelRegion {
    /// # Safety
    /// See HAL KernelRegion::new. Retain the initial table frames separately;
    /// this wrapper owns only the additional tables allocated by find_stack.
    pub(crate) unsafe fn new(
        direct: DirectMap,
        base: u64,
        tables: [u64; 3],
    ) -> Result<Self, PagingError> {
        Ok(Self {
            inner: unsafe { kryo_hal::KernelRegion::new(direct, base, tables) }?,
            tables: Vec::new(),
            pool: None,
            direct,
            detached: false,
        })
    }

    pub(crate) fn find_stack(
        &mut self,
        pool: &'static BootFramePool,
        direct: DirectMap,
    ) -> Result<usize, PagingError> {
        if direct.offset() != self.direct.offset()
            || self.pool.is_some_and(|owner| !core::ptr::eq(owner, pool))
        {
            return Err(PagingError::ContextChanged);
        }
        self.inner.retry_extension()?;
        self.pool = Some(pool);
        loop {
            for first in [2, 7, 17, 22] {
                if self.stack_free(first)? {
                    return Ok(first);
                }
            }
            // Fixed five-page cells preserve a live stack's lower guard even
            // when neighboring stacks are retired and holes are reused.
            for first in (27..self.inner.page_capacity().saturating_sub(3)).step_by(5) {
                if self.stack_free(first)? {
                    return Ok(first);
                }
            }
            if !self.inner.can_extend() {
                return Err(PagingError::InsufficientTables);
            }
            self.tables
                .try_reserve(1)
                .map_err(|_| PagingError::InsufficientTables)?;
            let table = pool
                .allocate()
                .map_err(|_| PagingError::InsufficientTables)?;
            let physical = table.address();
            // The owner is reachable before publication. Failed shootdowns keep
            // both the table and HAL's attached-leaf record for explicit retry.
            self.tables.push(table);
            let result = unsafe { self.inner.extend(physical) };
            if let Err(error) = result {
                if error != PagingError::ShootdownPending {
                    let owner = self.tables.pop().unwrap();
                    if let Err((_, owner)) = unsafe { pool.release(owner) } {
                        self.tables.push(owner);
                    }
                }
                return Err(error);
            }
        }
    }

    fn stack_free(&self, first: usize) -> Result<bool, PagingError> {
        let Some(guard) = first.checked_sub(1) else {
            return Ok(false);
        };
        let Some(last) = first.checked_add(3) else {
            return Ok(false);
        };
        if last >= self.inner.page_capacity() {
            return Ok(false);
        }
        for slot in guard..=last {
            // APIC and HPET slots remain reserved even before their mappings
            // are installed or after an optional device is unavailable.
            if slot == 0 || slot == 12 {
                return Ok(false);
            }
            match self.inner.permissions(slot) {
                Err(PagingError::NotMapped) => {}
                Ok(_) => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    /// # Safety
    /// Every user root sharing the branch and every mapping has retired. Keep
    /// this wrapper for retry if either the global flush or a frame return fails.
    #[cfg_attr(
        not(feature = "boot-tests"),
        expect(dead_code, reason = "Normal boot retains the platform region")
    )]
    pub(crate) unsafe fn detach(&mut self) -> Result<(), PagingError> {
        if !self.detached {
            unsafe { self.inner.detach() }?;
            self.detached = true;
        }
        while let Some(table) = self.tables.pop() {
            let pool = self.pool.expect("Extended region frame pool invariant");
            if let Err((_, table)) = unsafe { pool.release(table) } {
                self.tables.push(table);
                return Err(PagingError::ContextChanged);
            }
        }
        self.tables = Vec::new();
        self.pool = None;
        Ok(())
    }
}

impl Deref for KernelRegion {
    type Target = kryo_hal::KernelRegion;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for KernelRegion {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
