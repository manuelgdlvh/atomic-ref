use crate::access::access::{AccessGuard, AtomicAccessControl};
use crate::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Default)]
pub struct LockAccessControl {
    lock: RwLock<()>,
}

impl AtomicAccessControl for LockAccessControl {
    fn write(&self) -> impl AccessGuard {
        self.lock.write().expect("Always lock is locked")
    }

    fn read(&self) -> impl AccessGuard {
        self.lock.read().expect("Always lock is locked")
    }
}

impl AccessGuard for RwLockWriteGuard<'_, ()> {}
impl AccessGuard for RwLockReadGuard<'_, ()> {}
