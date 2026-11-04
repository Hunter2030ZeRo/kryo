use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

pub(crate) struct Mutex<T> {
    state: AtomicU32,
    value: UnsafeCell<T>,
    #[cfg(test)]
    wait: std::sync::Mutex<()>,
    #[cfg(test)]
    changed: std::sync::Condvar,
}

// The state word exclusively hands access to value between threads. Waiting
// uses a private kernel compare-before-park, with no allocation or TLS access.
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            value: UnsafeCell::new(value),
            #[cfg(test)]
            wait: std::sync::Mutex::new(()),
            #[cfg(test)]
            changed: std::sync::Condvar::new(),
        }
    }

    pub(crate) fn lock(&self) -> Guard<'_, T> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // 0 = unlocked, 1 = owned, 2 = owned with possible waiters. Setting
            // 2 before compare-and-park makes unlock-before-registration safe.
            while self.state.swap(2, Ordering::Acquire) != 0 {
                self.park();
            }
        }
        Guard {
            mutex: self,
            borrow: PhantomData,
        }
    }

    fn park(&self) {
        #[cfg(not(test))]
        crate::native::park(&self.state);
        #[cfg(test)]
        {
            let guard = self.wait.lock().unwrap();
            if self.state.load(Ordering::Relaxed) == 2 {
                drop(self.changed.wait(guard).unwrap());
            }
        }
    }

    fn wake(&self) {
        #[cfg(not(test))]
        crate::native::wake(&self.state);
        #[cfg(test)]
        {
            let _guard = self.wait.lock().unwrap();
            self.changed.notify_one();
        }
    }
}

pub(crate) struct Guard<'a, T> {
    mutex: &'a Mutex<T>,
    // Deref exposes T, so sharing a guard requires T: Sync even though the
    // enclosing mutex only needs T: Send. Moving the guard still needs T: Send.
    borrow: PhantomData<&'a mut T>,
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        if self.mutex.state.swap(0, Ordering::Release) == 2 {
            self.mutex.wake();
        }
    }
}
