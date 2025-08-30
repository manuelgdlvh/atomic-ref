use crate::access::AtomicAccessControl;
use crate::access::cas::CASAccessControl;
use crate::access::lock::LockAccessControl;
use crate::sync::{AtomicPtr, Ordering};
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

static ATOMIC_ID_GEN: AtomicU64 = AtomicU64::new(0);

pub struct Atomic<T, A>
where
    A: AtomicAccessControl,
{
    id: u64,
    // Initialized refs in 1. When write happens is reduced by 1 to only in flight current reads
    current: AtomicPtr<T>,
    // Masks for readers, writers, version
    control: A,
}

// If drop, reduce the active references to current and if zero.
impl<T, A> Drop for Atomic<T, A>
where
    A: AtomicAccessControl,
{
    fn drop(&mut self) {
        unsafe {
            Arc::from_raw(self.current.load(Ordering::Acquire));
        }
    }
}

impl<T: Debug> Atomic<T, CASAccessControl> {
    pub fn new_cas(value: T, max_write_line: u16) -> Atomic<T, CASAccessControl> {
        let raw = Arc::into_raw(Arc::new(value)) as *mut T;

        Atomic {
            id: ATOMIC_ID_GEN.fetch_add(1, Ordering::Release),
            current: AtomicPtr::new(raw),
            control: CASAccessControl::new(max_write_line),
        }
    }
}

impl<T: Debug> Atomic<T, LockAccessControl> {
    pub fn new_lock(value: T) -> Atomic<T, LockAccessControl> {
        let raw = Arc::into_raw(Arc::new(value)) as *mut T;

        Atomic {
            id: ATOMIC_ID_GEN.fetch_add(1, Ordering::Release),
            current: AtomicPtr::new(raw),
            control: LockAccessControl::default(),
        }
    }
}

impl<T: Debug, A: AtomicAccessControl> Atomic<T, A> {
    pub fn read(&self) -> Arc<T> {
        let _guard = self.control.read();

        let p = self.current.load(Ordering::Acquire) as *const T;
        unsafe {
            let tmp = Arc::from_raw(p);
            let out = Arc::clone(&tmp);
            std::mem::forget(tmp);
            out
        }
    }

    // TODO: To think about it (Improve reads in the case references are used-dropped entering in the new access_control model):
    // Writes like a continuous sequential writers handling fairness with writer_id
    // Writers do best effort to clean unstable references from the bucket (not reachable by readers and with refCount = 1).
    // If tracked references reach some limit, current read / write model must be used.

    pub fn write<F>(&self, update_fn: F)
    where
        F: Fn(&T) -> T,
    {
        let guard_ = self.control.write();

        let new_val = unsafe {
            let current_arc = Arc::from_raw(self.current.load(Ordering::Acquire));
            let v = update_fn(&current_arc);
            std::mem::forget(current_arc);
            v
        };

        let new_raw = Arc::into_raw(Arc::new(new_val)) as *mut T;
        let old_raw = self.current.swap(new_raw, Ordering::AcqRel);

        drop(guard_);

        unsafe {
            drop(Arc::from_raw(old_raw));
        }
    }
}
