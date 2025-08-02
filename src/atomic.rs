use std::alloc::{Layout, alloc, dealloc};
use std::error::Error;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, AtomicU16, AtomicU64, Ordering};

pub struct WriteGuard<'a> {
    control_flags: &'a AtomicControlBlock,
}

impl<'a> WriteGuard<'a> {
    pub fn new(atomic_flags: &'a AtomicControlBlock) -> Self {
        Self {
            control_flags: atomic_flags,
        }
    }
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        self.control_flags
            .flags
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let writers_flag =
                    (((current & AtomicControlBlock::NUM_WRITERS_MASK) >> 16) - 1) << 16;
                let current = current & !AtomicControlBlock::NUM_WRITERS_MASK;
                let result = current | writers_flag;
                Some(result)
            })
            .expect("Always writers must be decremented");
    }
}

pub struct ReadGuard<'a> {
    control_flags: &'a AtomicControlBlock,
}

impl<'a> ReadGuard<'a> {
    pub fn new(atomic_flags: &'a AtomicControlBlock) -> Self {
        Self {
            control_flags: atomic_flags,
        }
    }
}
impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        self.control_flags
            .flags
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let readers_flag = (current & AtomicControlBlock::NUM_READERS_MASK) - 1;
                let current = current & !AtomicControlBlock::NUM_READERS_MASK;
                let result = current | readers_flag;
                Some(result)
            })
            .expect("Always writers must be decremented");
    }
}

#[derive(Default)]
pub struct AtomicControlBlock {
    flags: AtomicU64,
}

impl AtomicControlBlock {
    // First 2
    const NUM_READERS_MASK: u64 = 0x0000_0000_0000_FFFF;

    // Second 2
    const NUM_WRITERS_MASK: u64 = 0x0000_0000_FFFF_0000;

    // Last 4
    const VERSION_MASK: u64 = 0xFFFF_FFFF_0000_0000;

    const MIDDLE_BITS_POINTS: u64 = 32;

    // Add writer if no reader
    // TODO: Add mechanism to avoid starvation

    // TODO: Add acquire timeout
    pub fn write(&self) -> WriteGuard {
        let mut flags = self.flags.load(Ordering::Acquire);
        loop {
            let readers = flags & Self::NUM_READERS_MASK;
            if readers != 0 {
                //TODO: Spin loop with backoff

                flags = self.flags.load(Ordering::Acquire);
                continue;
            }

            let writers = (((flags & AtomicControlBlock::NUM_WRITERS_MASK) >> 16) + 1) << 16;
            let new_flags = (flags & !AtomicControlBlock::NUM_WRITERS_MASK) | writers;

            if let Err(err_flags) =
                self.flags
                    .compare_exchange(flags, new_flags, Ordering::Relaxed, Ordering::Acquire)
            {
                flags = err_flags;
            } else {
                break;
            }
        }

        WriteGuard::new(self)
    }

    pub fn read(&self) -> ReadGuard {
        let mut flags = self.flags.load(Ordering::Acquire);
        loop {
            let writers = (flags & Self::NUM_WRITERS_MASK) >> 16;
            if writers != 0 {
                //TODO: Spin loop with backoff

                flags = self.flags.load(Ordering::Acquire);
                continue;
            }

            let readers = (flags & Self::NUM_READERS_MASK) + 1;
            let new_flags = (flags & !AtomicControlBlock::NUM_READERS_MASK) | readers;
            if let Err(err_flags) =
                self.flags
                    .compare_exchange(flags, new_flags, Ordering::Relaxed, Ordering::Acquire)
            {
                flags = err_flags;
            } else {
                break;
            }
        }

        ReadGuard::new(self)
    }

    pub fn increment_version(&self) -> u32 {
        let flags = self
            .flags
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let version = (((current & AtomicControlBlock::VERSION_MASK)
                    >> Self::MIDDLE_BITS_POINTS)
                    + 1)
                    << Self::MIDDLE_BITS_POINTS;
                let current = current & !AtomicControlBlock::VERSION_MASK;
                Some(current | version)
            })
            .expect("Always version must be incremented");

        ((flags & Self::VERSION_MASK) >> Self::MIDDLE_BITS_POINTS) as u32 + 1
    }
}
pub struct Atomic<T> {
    // Initialized refs in 1. When write happens is reduced by 1 to only in flight current reads
    current: AtomicPtr<ValueRef<T>>,
    // Masks for readers, writers, version
    control: AtomicControlBlock,
}

// If drop, reduce the active references to current and if zero.
impl<T> Drop for Atomic<T> {
    fn drop(&mut self) {
        unsafe { ptr::drop_in_place(self.current.load(Ordering::Acquire)) }
    }
}

impl<T: std::fmt::Debug> Atomic<T> {
    pub fn new(value: T) -> Atomic<T> {
        Atomic {
            current: AtomicPtr::new(allocate(ValueRef::new(value, 0))),
            control: AtomicControlBlock::default(),
        }
    }

    pub fn read(&self) -> ValueRef<T> {
        let guard_ = self.control.read();

        let ptr = self.current.load(Ordering::Acquire);
        unsafe { (*ptr).clone() }
    }

    // Sequential writes (fair order using SeqCst) and maximum in a row to allow complement with reads
    pub fn write(&self, value: T) {
        let guard_ = self.control.write();
        let version = self.control.increment_version();

        // Instead of swap linerized functions with retry (like update())
        let old_ref = self
            .current
            .swap(allocate(ValueRef::new(value, version)), Ordering::SeqCst);

        unsafe { ptr::drop_in_place(old_ref) }
    }
}

// Flags for InFlightRefs and version

#[derive(Debug)]
pub struct ValueRefInner<T> {
    data: T,
    refs: AtomicU16,
}

impl<T> ValueRefInner<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            refs: AtomicU16::new(1),
        }
    }
}

#[derive(Debug)]
pub struct ValueRef<T> {
    inner: NonNull<ValueRefInner<T>>,
    version: u32,
}

impl<T> Clone for ValueRef<T> {
    fn clone(&self) -> Self {
        let inner_ref = unsafe { self.inner.as_ref() };
        inner_ref.refs.fetch_add(1, Ordering::AcqRel);

        Self {
            inner: self.inner,
            version: self.version,
        }
    }
}

impl<T> Drop for ValueRef<T> {
    fn drop(&mut self) {
        let old_refs = unsafe { self.inner.as_ref().refs.fetch_sub(1, Ordering::Acquire) };

        if old_refs == 1 {
            let layout = Layout::new::<ValueRefInner<T>>();
            unsafe {
                ptr::drop_in_place(self.inner.as_ptr());
                dealloc(self.inner.as_ptr() as *mut u8, layout);
            }
        }
    }
}

impl<T: std::fmt::Debug> ValueRef<T> {
    fn new(value: T, version: u32) -> ValueRef<T> {
        unsafe {
            ValueRef::<T> {
                inner: NonNull::new_unchecked(allocate(ValueRefInner::new(value))),
                version,
            }
        }
    }

    pub fn get(&self) -> &T {
        unsafe { &self.inner.as_ref().data }
    }
}

fn allocate<I>(value: I) -> *mut I {
    let layout = Layout::new::<I>();
    let raw_ptr = unsafe { alloc(layout) } as *mut I;
    unsafe {
        ptr::write(raw_ptr, value);
    }
    raw_ptr
}
