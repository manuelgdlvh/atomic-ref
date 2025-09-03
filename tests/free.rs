use lib::atomic::Atomic;
use std::alloc::{GlobalAlloc, System};
use std::fmt::Debug;

#[cfg(not(loom))]
pub(crate) use std::alloc::Layout;
#[cfg(not(loom))]
use std::sync::Arc;
#[cfg(not(loom))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::thread;
use std::time::Duration;

use lib::access::AtomicAccessControl;
#[cfg(loom)]
pub(crate) use loom::alloc::Layout;
#[cfg(loom)]
use loom::sync::Arc;
#[cfg(loom)]
use loom::sync::atomic::{AtomicUsize, Ordering};
#[cfg(loom)]
use loom::thread;
#[cfg(loom)]
use loom::thread::JoinHandle;
use proptest::proptest;
#[cfg(not(loom))]
use std::thread::JoinHandle;

proptest! {

    #[cfg(not(loom))]
    #[test]
    fn test_atomic_cas_memory_free(num_readers in 4usize..6, num_writers in 4usize..6, num_worker_writes in 1000usize..10000) {
        perform(num_readers, num_writers, num_worker_writes, Atomic::new_cas(0, u16::MAX));

}


    #[cfg(not(loom))]
    #[test]
    fn test_atomic_lock_memory_free(num_readers in 4usize..6, num_writers in 4usize..6, num_worker_writes in 1000usize..10000) {
        perform(num_readers, num_writers, num_worker_writes, Atomic::new_lock(0));

}

}

fn perform<T: lib::tests::ReadWriteExt<usize> + 'static>(
    num_readers: usize,
    num_writers: usize,
    num_worker_writes: usize,
    target: T,
) {
    GLOBAL_ALLOCATOR.reset();

    let handle = lib::tests::runtime(num_readers, num_writers, Arc::new(target));

    handle.read(lib::tests::ReadTask::ReadUntil {
        stop_fn: Arc::new(move |val: &usize| {
            if *val == (num_writers * num_worker_writes) {
                true
            } else {
                false
            }
        }),
    });

    handle.write(lib::tests::WriteTask::Simple {
        num_execs: num_worker_writes,
        task: |val: &usize| *val + 1,
    });

    let results = handle.recv_results(num_readers + num_writers, Duration::from_secs(15));

    for result in results {
        match result {
            lib::tests::TaskResult::ReadUntil(last_read) => {
                assert_eq!((num_writers * num_worker_writes), *last_read);
            }
            _ => {}
        }
    }

    drop(handle);

    // assert_eq!(
    //     GLOBAL_ALLOCATOR.allocs.load(Ordering::Acquire),
    //     GLOBAL_ALLOCATOR.deallocs.load(Ordering::Acquire)
    // );
}

#[cfg(not(loom))]
#[derive(Debug)]
pub struct CountingAllocator {
    pub allocs: AtomicUsize,
    pub deallocs: AtomicUsize,
    pub bytes_allocated: AtomicUsize,
    pub bytes_deallocated: AtomicUsize,
}

#[cfg(not(loom))]
#[global_allocator]
pub static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator::new();

#[cfg(not(loom))]
impl CountingAllocator {
    pub const fn new() -> Self {
        CountingAllocator {
            allocs: AtomicUsize::new(0),
            deallocs: AtomicUsize::new(0),
            bytes_allocated: AtomicUsize::new(0),
            bytes_deallocated: AtomicUsize::new(0),
        }
    }

    pub fn reset(&self) {
        self.allocs.store(0, Ordering::SeqCst);
        self.deallocs.store(0, Ordering::SeqCst);
        self.bytes_allocated.store(0, Ordering::SeqCst);
        self.bytes_deallocated.store(0, Ordering::SeqCst);
    }
}

#[cfg(not(loom))]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocs.fetch_add(1, Ordering::SeqCst);
        self.bytes_allocated
            .fetch_add(layout.size(), Ordering::SeqCst);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.deallocs.fetch_add(1, Ordering::SeqCst);
        self.bytes_deallocated
            .fetch_add(layout.size(), Ordering::SeqCst);
        System.dealloc(ptr, layout)
    }
}
