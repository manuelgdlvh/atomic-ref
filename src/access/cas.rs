use crate::access::{AccessGuard, AtomicAccessControl};
use crate::sync::Contender;
use crate::sync::{AtomicU64, Ordering};
use crossbeam_utils::CachePadded;
use std::sync::atomic::{AtomicBool, AtomicU16};

const ACTIVE_READERS_MASK: u64 = 0x0000_0000_0000_FFFF;
const READ_SLOTS_MASK: u64 = 0xFFFF_FFFF_0000_0000;
const PENDING_READERS_MASK: u64 = 0x0000_0000_FFFF_0000;
const NOT_ACTIVE_PENDING_READERS_MASK: u64 = !0x0000_0000_FFFF_FFFF;

const PENDING_READERS_SHIFT: u64 = 16;
const READ_SLOTS_SHIFT: u64 = 32;

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
                let readers_flag = (old & ACTIVE_READERS_MASK) - 1;
                Some((old & !ACTIVE_READERS_MASK) | readers_flag)
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

        if old == 1 {
            self.access_control_ref
                .is_writing
                .store(false, Ordering::Release);
        }
    }
}

#[derive(Default)]
pub struct BackOffStrategy {
    raw: Option<Contender>,
}

impl BackOffStrategy {
    pub fn wait(&mut self) {
        if self.raw.is_none() {
            self.raw = Some(Contender::new());
        }

        unsafe { self.raw.as_mut().unwrap_unchecked() }.snooze();
    }
}

pub struct CASAccessControl {
    // 0-16 bits hold current active readers
    // 16=32 bits hold pending registered readers
    // 32-64bits hold read slots. When writing phase starts read slots are initialized with the current pending readers to give them grace period to finish his reads.
    read_flags: CachePadded<AtomicU64>,

    // The initiator of write phase read current pending writers and assign max of sequential writes to the phase. This is to ensure convergence of reads and writes.
    write_slots: CachePadded<AtomicU16>,
    // Track the next writer to perform the action.
    next_writer_id: CachePadded<AtomicU16>,
    pending_writers: CachePadded<AtomicU16>,
    is_writing: CachePadded<AtomicBool>,

    // Maximum number of sequential writes that can happen in each write phase. Can be in/decreased to reduce contention in read or writes.
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
                let pending_readers_flag =
                    (((old & PENDING_READERS_MASK) >> PENDING_READERS_SHIFT) + 1)
                        << PENDING_READERS_SHIFT;
                Some((old & !PENDING_READERS_MASK) | pending_readers_flag)
            })
            .expect("Always pending writers must be incremented");
    }

    fn dec_pending_writers(&self) {
        self.pending_writers.fetch_sub(1, Ordering::Release);
    }

    fn try_reserve_write_slot(&self) -> Option<u16> {
        self.write_slots
            .fetch_update(Ordering::Release, Ordering::Acquire, |slots| {
                if slots == 0 { None } else { Some(slots - 1) }
            })
            .ok()
    }

    fn try_reserve_read_slot(&self) -> bool {
        // Should increment readers, decrement pending readers and decrement slot if not zero

        let result = self
            .read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let readers_slots = (old & READ_SLOTS_MASK) >> READ_SLOTS_SHIFT;
                if readers_slots == 0 {
                    None
                } else {
                    let readers_slots_flag = (readers_slots - 1) << READ_SLOTS_SHIFT;
                    let pending_readers_flag =
                        (((old & PENDING_READERS_MASK) >> PENDING_READERS_SHIFT) - 1)
                            << PENDING_READERS_SHIFT;
                    let readers_flag = (old & ACTIVE_READERS_MASK) + 1;

                    Some(readers_slots_flag | pending_readers_flag | readers_flag)
                }
            });
        result.is_ok()
    }

    fn initialize_read_slots(&self, slots_size: u64) {
        self.read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                Some(old | (slots_size << READ_SLOTS_SHIFT))
            })
            .expect("Always read slots must be initialized");
    }

    fn initialize_read(&self) {
        self.read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let pending_readers_flag =
                    (((old & PENDING_READERS_MASK) >> PENDING_READERS_SHIFT) - 1)
                        << PENDING_READERS_SHIFT;
                let readers_flag = (old & ACTIVE_READERS_MASK) + 1;
                Some((old & NOT_ACTIVE_PENDING_READERS_MASK) | pending_readers_flag | readers_flag)
            })
            .expect("Always pending writers must be incremented");
    }

    fn try_reserve_read_slot_or_reset(&self) -> bool {
        let old_read_flags = self
            .read_flags
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                let readers_slots = (old & READ_SLOTS_MASK) >> READ_SLOTS_SHIFT;
                if readers_slots == 0 {
                    let pending_readers_flag =
                        (((old & PENDING_READERS_MASK) >> PENDING_READERS_SHIFT) + 1)
                            << PENDING_READERS_SHIFT;
                    let readers_flag = (old & ACTIVE_READERS_MASK) - 1;
                    Some(
                        (old & NOT_ACTIVE_PENDING_READERS_MASK)
                            | pending_readers_flag
                            | readers_flag,
                    )
                } else {
                    Some((old & !READ_SLOTS_MASK) | ((readers_slots - 1) << READ_SLOTS_SHIFT))
                }
            })
            .expect("Always pending writers must be incremented");

        (old_read_flags & READ_SLOTS_MASK) >> READ_SLOTS_SHIFT != 0
    }

    fn init_write_slot(&self, slots_size: u16) {
        self.write_slots.store(slots_size, Ordering::Release);
    }
}

impl AtomicAccessControl for CASAccessControl {
    fn write(&self) -> impl AccessGuard {
        self.inc_pending_writers();

        let slot_idx;
        let initiator;
        let mut backoff = BackOffStrategy::default();

        // Initialize Write Phase.
        loop {
            if self.is_writing.load(Ordering::Acquire) {
                // [1..=SLOTS_SIZE]
                if let Some(val) = self.try_reserve_write_slot() {
                    slot_idx = val;
                    initiator = false;
                    break;
                }
            } else if !self.is_writing.swap(true, Ordering::Acquire) {
                initiator = true;
                let pending_writers = self.pending_writers.load(Ordering::Acquire);
                let slots_size = pending_writers.min(self.max_write_line);

                self.init_write_slot(slots_size - 1);
                slot_idx = slots_size;
                self.next_writer_id.store(slot_idx, Ordering::Release);
                break;
            } else if let Some(val) = self.try_reserve_write_slot() {
                slot_idx = val;
                initiator = false;
                break;
            }

            backoff.wait();
        }

        self.dec_pending_writers();

        // Instead to initialize guaranteed read slots at the last write, initialize after writing flag to true to know how much time await to start writing. (Initiator).
        // Only initiator wait to read full finished, the others will wait until his turn.
        if initiator {
            let pending_readers = (self.read_flags.load(Ordering::Acquire) & PENDING_READERS_MASK)
                >> PENDING_READERS_SHIFT;
            if pending_readers > 0 {
                self.initialize_read_slots(pending_readers);
            }

            loop {
                let readers_flag = self.read_flags.load(Ordering::Acquire);

                let readers = readers_flag & ACTIVE_READERS_MASK;
                let read_slots = (readers_flag & READ_SLOTS_MASK) >> READ_SLOTS_SHIFT;

                if readers == 0 && read_slots == 0 {
                    break;
                }

                backoff.wait();
            }
        } else {
            while self.next_writer_id.load(Ordering::Acquire) != slot_idx {
                backoff.wait();
            }
        }

        CASWriteGuard::new(self)
    }

    fn read(&self) -> impl AccessGuard {
        self.inc_pending_readers();
        let mut backoff = BackOffStrategy::default();
        loop {
            if self.is_writing.load(Ordering::Acquire) {
                if self.try_reserve_read_slot() {
                    break;
                }
            } else {
                self.initialize_read();

                // Stale Read due to change. Must try get slot.
                if !self.is_writing.load(Ordering::Acquire) || self.try_reserve_read_slot_or_reset()
                {
                    break;
                }
            }

            backoff.wait();
        }

        CASReadGuard::new(self)
    }
}
