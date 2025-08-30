#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicPtr, AtomicU16, AtomicU64, Ordering};

#[cfg(not(loom))]
pub(crate) use std::alloc::Layout;

#[cfg(not(loom))]
pub(crate) use std::alloc::{alloc, dealloc};

#[cfg(not(loom))]
pub(crate) use std::sync::Arc;

#[cfg(not(loom))]
pub(crate) type Contender = crossbeam_utils::Backoff;

#[cfg(not(loom))]
pub(crate) use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(loom)]
pub(crate) use loom::sync::atomic::fence;

#[cfg(loom)]
pub(crate) use loom::alloc::Layout;
#[cfg(loom)]
pub(crate) use loom::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
#[cfg(loom)]
pub(crate) type Contender = CustomBackoff;

#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicPtr, AtomicU16, AtomicU64, Ordering};

#[cfg(loom)]
pub(crate) use loom::sync::Arc;
#[cfg(loom)]
pub(crate) struct CustomBackoff;

#[cfg(loom)]
pub(crate) use loom::alloc::{alloc, dealloc};

#[cfg(loom)]
impl CustomBackoff {
    pub fn new() -> Self {
        Self {}
    }

    pub fn is_completed(&self) -> bool {
        true
    }

    pub fn snooze(&self) {
        loom::thread::yield_now();
    }
}
