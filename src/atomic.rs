use crate::access::cas::CASAccessControl;
use crate::access::lock::LockAccessControl;
use crate::access::AtomicAccessControl;
use crate::mem::deallocate;
use crate::sync::{AtomicPtr, Ordering};
use crate::value_ref::{ValueRef, ValueRefInner};
use std::fmt::Debug;
use std::sync::atomic::AtomicU64;

static ATOMIC_ID_GEN: AtomicU64 = AtomicU64::new(0);

pub struct Atomic<T, A>
where
    A: AtomicAccessControl,
{
    id: u64,
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
            id: ATOMIC_ID_GEN.fetch_add(1, Ordering::Release),
            current: AtomicPtr::new(ValueRefInner::raw(value)),
            control: CASAccessControl::new(max_write_line),
        }
    }
}

impl<T: Debug> Atomic<T, LockAccessControl> {
    pub fn new_lock(value: T) -> Atomic<T, LockAccessControl> {
        Atomic {
            id: ATOMIC_ID_GEN.fetch_add(1, Ordering::Release),
            current: AtomicPtr::new(ValueRefInner::raw(value)),
            control: LockAccessControl::default(),
        }
    }
}

impl<T: Debug, A: AtomicAccessControl> Atomic<T, A> {
    pub fn read(&self) -> ValueRef<T> {
        let _guard = self.control.read();

        let ptr = self.current.load(Ordering::Acquire);
        ValueRef::from(ptr)
    }

    pub fn write<F>(&self, update_fn: F)
    where
        F: Fn(&T) -> T,
    {
        let guard_ = self.control.write();

        let current_ptr = self.current.load(Ordering::Acquire);
        let current_val = unsafe { &current_ptr.as_ref().unwrap_unchecked().data };
        let new_val = update_fn(current_val);
        let new_ptr = ValueRefInner::raw(new_val);

        let old_ptr = self.current.swap(new_ptr, Ordering::AcqRel);

        drop(guard_);

        let old_refs = unsafe { &old_ptr.as_ref().unwrap_unchecked().refs };
        if old_refs.fetch_sub(1, Ordering::SeqCst) == 1 {
            deallocate(old_ptr, true);
        }
    }
}

type TxFunc<T> = Box<dyn Fn(&T) -> Option<T>>;
pub struct Transaction<'a, T, A>
where
    T: Debug,
    A: AtomicAccessControl,
{
    context: Vec<(&'a Atomic<T, A>, TxFunc<T>)>,
}

impl<'a, T, A> Transaction<'a, T, A>
where
    T: Debug,
    A: AtomicAccessControl,
{
    pub fn new(capacity: usize) -> Self {
        Self {
            context: Vec::with_capacity(capacity),
        }
    }

    pub fn add<F>(mut self, atomic: &'a Atomic<T, A>, fn_: F) -> Self
    where
        F: Fn(&T) -> Option<T> + 'static,
    {
        self.context.push((atomic, Box::new(fn_)));
        self
    }

    pub fn execute(mut self) -> bool {
        self.context.sort_unstable_by_key(|(a, _)| a.id);

        let mut guards: Vec<_> = Vec::with_capacity(self.context.len());
        for (atomic, _) in self.context.iter() {
            guards.push(atomic.control.write());
        }

        let mut rollback = false;
        let mut ptrs = Vec::with_capacity(guards.len());
        for (atomic, f) in self.context.iter() {
            let current_ptr = atomic.current.load(Ordering::Acquire);
            let current_val = unsafe { &current_ptr.as_ref().unwrap_unchecked().data };
            if let Some(new_val) = f(current_val) {
                let new_ptr = ValueRefInner::raw(new_val);
                let old_ptr = atomic.current.swap(new_ptr, Ordering::AcqRel);
                ptrs.push((old_ptr, new_ptr));
            } else {
                rollback = true;
                break;
            }
        }

        if rollback {
            let i = 0;
            while i < ptrs.len() {
                deallocate(ptrs[i].1, true);
                self.context[i]
                    .0
                    .current
                    .store(ptrs[i].0, Ordering::Release);
            }
        } else {
            for (old_ptr, _) in ptrs {
                let old_refs = unsafe { &old_ptr.as_ref().unwrap_unchecked().refs };
                if old_refs.fetch_sub(1, Ordering::SeqCst) == 1 {
                    deallocate(old_ptr, true);
                }
            }
        }

        !rollback
    }
}
