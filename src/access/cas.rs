use crate::access::access::{AccessGuard, AtomicAccessControl};
use crate::sync::Contender;
use crate::sync::{AtomicU64, Ordering};
use crossbeam_utils::CachePadded;
use std::sync::atomic::{AtomicBool, AtomicU16};

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
            .read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let readers_flag = (old & 0x0000_0000_0000_FFFF) - 1;
                Some((old & !0x0000_0000_0000_FFFF) | readers_flag)
            })
            .expect("Always pending writers must be incremented");
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
        let old = self
            .access_control_ref
            .next_writer_id
            .fetch_sub(1, Ordering::Release);

        // Last Writer.
        if old == 1 {
            self.access_control_ref
                .is_writing
                .store(false, Ordering::Release);
        }
    }
}

pub struct CASAccessControl {
    // Holding Current Readers (first 16 bits) , Pending Readers (next 16 bits) and guaranteed read slots (most significant 32bits)
    read_flags: CachePadded<AtomicU64>,

    // Track how much writers can write in a row. The 1 slot is the initiator.
    // The initiator is the last executed, and put is_writing to false and initialize the force_read_slots.
    write_slots: CachePadded<AtomicU16>,
    next_writer_id: CachePadded<AtomicU16>,
    pending_writers: CachePadded<AtomicU16>,
    is_writing: CachePadded<AtomicBool>,
    max_write_line: u16,
}

impl Default for CASAccessControl {
    fn default() -> Self {
        Self {
            read_flags: Default::default(),
            write_slots: Default::default(),
            next_writer_id: Default::default(),
            pending_writers: Default::default(),
            is_writing: Default::default(),
            max_write_line: 1,
        }
    }
}

impl CASAccessControl {
    pub fn new(max_write_line: u16) -> Self {
        assert!(max_write_line > 0);
        Self {
            max_write_line,
            ..Default::default()
        }
    }

    fn inc_pending_writers(&self) {
        self.pending_writers.fetch_add(1, Ordering::Release);
    }

    fn inc_pending_readers(&self) {
        self.read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let pending_readers_flag = (((old & 0x0000_0000_FFFF_0000) >> 16) + 1) << 16;
                Some((old & !0x0000_0000_FFFF_0000) | pending_readers_flag)
            })
            .expect("Always pending writers must be incremented");
    }

    fn dec_pending_writers(&self) {
        self.pending_writers.fetch_sub(1, Ordering::Release);
    }

    fn try_reserve_write_slot(&self) -> Option<u16> {
        if let Ok(current_slot) =
            self.write_slots
                .fetch_update(Ordering::Release, Ordering::Acquire, |slots| {
                    if slots == 0 {
                        None
                    } else {
                        Some(slots - 1)
                    }
                })
        {
            Some(current_slot)
        } else {
            None
        }
    }

    fn try_reserve_read_slot(&self) -> bool {
        // Should increment readers, decrement pending readers and decrement slot if not zero

        let result = self
            .read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let readers_slots = (old & 0xFFFF_FFFF_0000_0000) >> 32;
                if readers_slots == 0 {
                    None
                } else {
                    let readers_slots_flag = (readers_slots - 1) << 32;
                    let pending_readers_flag = (((old & 0x0000_0000_FFFF_0000) >> 16) - 1) << 16;
                    let readers_flag = (old & 0x0000_0000_0000_FFFF) + 1;

                    Some(readers_slots_flag | pending_readers_flag | readers_flag)
                }
            });
        result.is_ok()
    }

    fn initialize_read_slots(&self, slots_size: u64) {
        self.read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                Some(old | (slots_size << 32))
            })
            .expect("Always read slots must be initialized");
    }

    fn initialize_read(&self) {
        self.read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let pending_readers_flag = (((old & 0x0000_0000_FFFF_0000) >> 16) - 1) << 16;
                let readers_flag = (old & 0x0000_0000_0000_FFFF) + 1;
                Some((old & !0x0000_0000_FFFF_FFFF) | pending_readers_flag | readers_flag)
            })
            .expect("Always pending writers must be incremented");
    }

    fn reset_read(&self) {
        self.read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let pending_readers_flag = (((old & 0x0000_0000_FFFF_0000) >> 16) + 1) << 16;
                let readers_flag = (old & 0x0000_0000_0000_FFFF) - 1;
                Some((old & !0x0000_0000_FFFF_FFFF) | pending_readers_flag | readers_flag)
            })
            .expect("Always pending writers must be incremented");
    }

    fn init_write_slot(&self, slots_size: u16) {
        self.write_slots.store(slots_size, Ordering::Release);
    }
}

impl AtomicAccessControl for CASAccessControl {
    fn write(&self) -> impl AccessGuard {
        self.inc_pending_writers();

        let mut slot_idx = 0;
        let mut initiator = false;
        let mut backoff: Option<Contender> = None;

        // Initialize Write Phase.
        loop {
            if self.is_writing.load(Ordering::Acquire) {
                //[1..=SLOTS_SIZE]
                if let Some(val) = self.try_reserve_write_slot() {
                    slot_idx = val;
                    break;
                }
            } else {
                initiator = !self.is_writing.swap(true, Ordering::Release);
                if initiator {
                    // Relaxed?
                    let pending_writers = self.pending_writers.load(Ordering::Acquire);
                    let slots_size = pending_writers.min(self.max_write_line);

                    // Minus 1 to guarantee the initiator slot.
                    self.init_write_slot(slots_size - 1);
                    slot_idx = slots_size;
                    self.next_writer_id.store(slot_idx, Ordering::Release);
                    break;
                } else if let Some(val) = self.try_reserve_write_slot() {
                    slot_idx = val;
                    break;
                }
            }

            if backoff.is_none() {
                backoff = Some(Contender::new());
            }

            let backoff_mut_ref = unsafe { backoff.as_mut().unwrap_unchecked() };
            backoff_mut_ref.snooze();
        }

        // Decrement pending writer here.
        self.dec_pending_writers();

        // Instead to initialize guaranteed read slots at the last write, initialize after writing flag to true to know how much time await to start writing. (Initiator).
        // Only initiator wait to read full finished, the others will wait until his turn.
        if initiator {
            // Relaxed?

            let pending_readers =
                (self.read_flags.load(Ordering::Relaxed) & 0x0000_0000_FFFF_0000) >> 16;
            if pending_readers > 0 {
                self.initialize_read_slots(pending_readers);
            }

            loop {
                let readers_flag = self.read_flags.load(Ordering::Relaxed);

                let readers = readers_flag & 0x0000_0000_0000_FFFF;
                let read_slots = (readers_flag & 0xFFFF_FFFF_0000_0000) >> 32;

                if readers == 0 && read_slots == 0 {
                    break;
                }

                if backoff.is_none() {
                    backoff = Some(Contender::new());
                }

                let backoff_mut_ref = unsafe { backoff.as_mut().unwrap_unchecked() };
                backoff_mut_ref.snooze();
            }

            // At what point i need to wait until reach.
        } else {
            // Waiting turn change
            while self.next_writer_id.load(Ordering::Acquire) != slot_idx {
                if backoff.is_none() {
                    backoff = Some(Contender::new());
                }

                let backoff_mut_ref = unsafe { backoff.as_mut().unwrap_unchecked() };
                backoff_mut_ref.snooze();
            }
        }

        CASWriteGuard::new(self)
    }

    fn read(&self) -> impl AccessGuard {
        self.inc_pending_readers();
        let mut backoff: Option<Contender> = None;
        loop {
            let is_writing = self.is_writing.load(Ordering::Acquire);
            if is_writing {
                if self.try_reserve_read_slot() {
                    break;
                }
            } else {
                self.initialize_read();
                // Stale Read due to change. Must to try get slot.
                if !self.is_writing.load(Ordering::Acquire) {
                    break;
                }

                // Reset and continue to try reserve slot. Avoid wait-free algorithm race conditions...
                self.reset_read();
            }

            if backoff.is_none() {
                backoff = Some(Contender::new());
            }

            let backoff_mut_ref = unsafe { backoff.as_mut().unwrap_unchecked() };
            backoff_mut_ref.snooze();
        }

        CASReadGuard::new(self)
    }
}
