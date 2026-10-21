//! Fair ticket spinlock.

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

/// A fair (FIFO) ticket lock protecting a value of type `T`.
pub struct SpinLock<T> {
    next: AtomicU32,
    serving: AtomicU32,
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is serialised by the ticket protocol.
unsafe impl<T: Send> Send for SpinLock<T> {}
// SAFETY: as above; guards hand out `&mut T` only to the unique holder.
unsafe impl<T: Send> Sync for SpinLock<T> {}

/// RAII guard; the lock is released on drop.
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> SpinLock<T> {
    /// Creates an unlocked lock.
    pub const fn new(value: T) -> Self {
        Self { next: AtomicU32::new(0), serving: AtomicU32::new(0), value: UnsafeCell::new(value) }
    }

    /// Spins until the lock is acquired.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        while self.serving.load(Ordering::Acquire) != ticket {
            spin_loop();
        }
        SpinLockGuard { lock: self }
    }

    /// Acquires the lock only if it is free right now.
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let serving = self.serving.load(Ordering::Relaxed);
        self.next
            .compare_exchange(serving, serving.wrapping_add(1), Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinLockGuard { lock: self })
    }

    /// Whether some holder or waiter currently exists.
    pub fn is_locked(&self) -> bool {
        self.next.load(Ordering::Relaxed) != self.serving.load(Ordering::Relaxed)
    }

    /// Consumes the lock, returning the protected value.
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard proves unique ownership of the lock.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard proves unique ownership of the lock.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.serving.fetch_add(1, Ordering::Release);
    }
}
