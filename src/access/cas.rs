use crate::access::access::{AccessGuard, AtomicAccessControl};

use crate::sync::Contender;
use crate::sync::{AtomicU64, Ordering};

impl AccessGuard for CASReadGuard<'_> {}
impl AccessGuard for CASWriteGuard<'_> {}
pub struct CASReadGuard<'a> {
    access_control_ref: &'a CASAccessControl,
}

impl<'a> CASReadGuard<'a> {
    pub fn new(access_control_ref: &'a CASAccessControl) -> Self {
        Self { access_control_ref }
    }
}
impl Drop for CASReadGuard<'_> {
    fn drop(&mut self) {
        self.access_control_ref
            .flags
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let readers_flag = (current & CASAccessControl::NUM_READERS_MASK) - 1;
                let current = current & !CASAccessControl::NUM_READERS_MASK;
                let result = current | readers_flag;
                Some(result)
            })
            .expect("Always readers must be decremented");
    }
}

// Write
pub struct CASWriteGuard<'a> {
    access_control_ref: &'a CASAccessControl,
}

impl<'a> CASWriteGuard<'a> {
    pub fn new(access_control_ref: &'a CASAccessControl) -> Self {
        Self { access_control_ref }
    }
}
impl Drop for CASWriteGuard<'_> {
    fn drop(&mut self) {
        self.access_control_ref
            .flags
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let writers_flag = (((current & CASAccessControl::NUM_WRITERS_MASK)
                    >> CASAccessControl::WRITERS_BITS_SHIFT)
                    - 1)
                    << CASAccessControl::WRITERS_BITS_SHIFT;
                let current = current & !CASAccessControl::NUM_WRITERS_MASK;
                let result = current | writers_flag;
                Some(result)
            })
            .expect("Always writers must be decremented");
    }
}

#[derive(Default)]
pub struct CASAccessControl {
    flags: AtomicU64,
}

// TODO: Change the Ordering modes.

// TODO: Hybrid mechanism with blocking in case of high contention
impl AtomicAccessControl for CASAccessControl {
    fn write(&self) -> impl AccessGuard {
        let mut flags = self.flags.load(Ordering::SeqCst);
        let mut backoff: Option<Contender> = None;

        loop {
            let readers = flags & Self::NUM_READERS_MASK;
            let writers = (flags & Self::NUM_WRITERS_MASK) >> Self::WRITERS_BITS_SHIFT;

            if readers > 0 || writers > 0 {
                if backoff.is_none() {
                    backoff = Some(Contender::new());
                }


                let backoff_mut_ref = unsafe { backoff.as_mut().unwrap_unchecked() };
                if backoff_mut_ref.is_completed() {
                    //println!("locked in write");
                }
                backoff_mut_ref.snooze();

                flags = self.flags.load(Ordering::SeqCst);
            } else {
                let writers = (((flags & CASAccessControl::NUM_WRITERS_MASK)
                    >> Self::WRITERS_BITS_SHIFT)
                    + 1)
                    << Self::WRITERS_BITS_SHIFT;
                let new_flags = (flags & !CASAccessControl::NUM_WRITERS_MASK) | writers;

                if let Err(err_flags) = self.flags.compare_exchange(
                    flags,
                    new_flags,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    flags = err_flags;
                } else {
                    break;
                }
            }
        }

        CASWriteGuard::new(self)
    }

    // TODO: Algorithm for concurrency control (delay reads or writes)
    // Control how much pending writes could be when to reduce reads
    // Control how much pending reads could be when to reduce writes

    fn read(&self) -> impl AccessGuard {
        let mut flags = self.flags.load(Ordering::SeqCst);
        let mut backoff: Option<Contender> = None;

        // Remove from pending when successful (else branch)
        loop {
            let writers = (flags & Self::NUM_WRITERS_MASK) >> Self::WRITERS_BITS_SHIFT;
            if writers > 0 {
                if backoff.is_none() {
                    backoff = Some(Contender::new());
                }

                let backoff_mut_ref = unsafe { backoff.as_mut().unwrap_unchecked() };
                if backoff_mut_ref.is_completed() {
                    //println!("locked in write");
                }
                backoff_mut_ref.snooze();

                flags = self.flags.load(Ordering::SeqCst);
            } else {
                let readers = (flags & Self::NUM_READERS_MASK) + 1;
                let new_flags = (flags & !CASAccessControl::NUM_READERS_MASK) | readers;

                if let Err(err_flags) = self.flags.compare_exchange(
                    flags,
                    new_flags,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    flags = err_flags;
                } else {
                    break;
                }
            }
        }

        CASReadGuard::new(self)
    }

    fn increment_version(&self) -> u32 {
        1
    }
}

impl CASAccessControl {
    const NUM_READERS_MASK: u64 = 0x0000_0000_0000_FFFF;

    pub(crate) const NUM_WRITERS_MASK: u64 = 0x0000_0000_FFFF_0000;

    const PENDING_NUM_READERS_MASK: u64 = 0x0000_FFFF_0000_0000;

    pub(crate) const PENDING_NUM_WRITERS_MASK: u64 = 0xFFFF_0000_0000_0000;

    pub(crate) const WRITERS_BITS_SHIFT: u64 = 16;
    pub(crate) const PENDING_READERS_BITS_SHIFT: u64 = 32;
    pub(crate) const PENDING_WRITERS_BITS_SHIFT: u64 = 48;
}
