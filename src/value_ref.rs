use std::fmt::Debug;

use crate::sync::{AtomicU16, Ordering};

#[derive(Debug)]
pub struct ValueRefInner<T> {
    pub(crate) data: T,
    pub(crate) refs: AtomicU16,
}

impl<T> ValueRefInner<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            refs: AtomicU16::new(1),
        }
    }

    pub fn raw(value: T) -> *mut ValueRefInner<T> {
        let result = ValueRefInner::new(value);
        let raw_ptr = crate::mem::allocate(result);
        raw_ptr
    }
}

#[derive(Debug)]
pub struct ValueRef<T: Debug> {
    pub(crate) inner: *mut ValueRefInner<T>,
}

impl<T: Debug> Clone for ValueRef<T> {
    fn clone(&self) -> Self {
        self.as_ref().refs.fetch_add(1, Ordering::SeqCst);

        Self { inner: self.inner }
    }
}

impl<T: Debug> Drop for ValueRef<T> {
    fn drop(&mut self) {
        if self.as_ref().refs.fetch_sub(1, Ordering::SeqCst) == 1 {
            crate::mem::deallocate(self.inner, true);
        }
    }
}

impl<T: Debug> ValueRef<T> {
    pub(crate) fn from(inner: *mut ValueRefInner<T>) -> ValueRef<T> {
        let ref_ = unsafe { inner.as_ref().unwrap_unchecked() };
        ref_.refs.fetch_add(1, Ordering::SeqCst);
        ValueRef { inner }
    }

    pub(crate) fn as_ref(&self) -> &ValueRefInner<T> {
        unsafe { self.inner.as_ref().unwrap_unchecked() }
    }
    pub fn get(&self) -> &T {
        &self.as_ref().data
    }
}
