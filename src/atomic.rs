use crate::access::access::AtomicAccessControl;
use crate::access::cas::CASAccessControl;
use crate::mem::deallocate;
use crate::sync::{AtomicPtr, Ordering};
use crate::value_ref::{ValueRef, ValueRefInner};
use std::fmt::Debug;

use crate::access::lock::LockAccessControl;

pub struct Atomic<T, A>
where
    A: AtomicAccessControl,
{
    // Initialized refs in 1. When write happens is reduced by 1 to only in flight current reads
    current: AtomicPtr<ValueRefInner<T>>,
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
            let inner_ref = self.current.load(Ordering::SeqCst);
            let current_refs = inner_ref
                .as_ref()
                .unwrap_unchecked()
                .refs
                .fetch_sub(1, Ordering::SeqCst);
            if current_refs == 1 {
                deallocate(inner_ref, true);
            }
        }
    }
}

impl<T: Debug> Atomic<T, CASAccessControl> {
    pub fn new_cas(value: T, max_write_line: u16) -> Atomic<T, CASAccessControl> {
        Atomic {
            current: AtomicPtr::new(ValueRefInner::raw(value)),
            control: CASAccessControl::new(max_write_line),
        }
    }
}

impl<T: Debug> Atomic<T, LockAccessControl> {
    pub fn new_lock(value: T) -> Atomic<T, LockAccessControl> {
        Atomic {
            current: AtomicPtr::new(ValueRefInner::raw(value)),
            control: LockAccessControl::default(),
        }
    }
}

impl<T: Debug, A: AtomicAccessControl> Atomic<T, A> {
    pub fn read(&self) -> ValueRef<T> {
        let guard_ = self.control.read();

        let ptr = self.current.load(Ordering::SeqCst);
        ValueRef::from(ptr)
    }

    pub fn write<F>(&self, update_fn: F)
    where
        F: Fn(&T) -> T,
    {
        let guard_ = self.control.write();

        let current_ptr = self.current.load(Ordering::SeqCst);
        let current_val = unsafe { &current_ptr.as_ref().unwrap_unchecked().data };
        let new_val = update_fn(current_val);
        let new_ptr = ValueRefInner::raw(new_val);

        let old_ptr = self.current.swap(new_ptr, Ordering::SeqCst);

        drop(guard_);

        let old_refs = unsafe { &old_ptr.as_ref().unwrap_unchecked().refs };
        if old_refs.fetch_sub(1, Ordering::SeqCst) == 1 {
            deallocate(old_ptr, true);
        }
    }
}
