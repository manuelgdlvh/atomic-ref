use lib::access::access::AtomicAccessControl;
use lib::atomic::Atomic;
use lib::value_ref::ValueRef;
use std::alloc::{GlobalAlloc, System};
use std::fmt::Debug;

#[cfg(not(loom))]
pub(crate) use std::alloc::Layout;
#[cfg(not(loom))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::sync::Arc;
#[cfg(not(loom))]
use std::thread;

#[cfg(not(loom))]
use std::thread::JoinHandle;

#[cfg(loom)]
pub(crate) use loom::alloc::Layout;
#[cfg(loom)]
use loom::sync::atomic::{AtomicUsize, Ordering};
#[cfg(loom)]
use loom::sync::Arc;
#[cfg(loom)]
use loom::thread;
#[cfg(loom)]
use loom::thread::JoinHandle;

// Due to proptest allocations
const EXTRA_ALLOCS: usize = 1;

// NOTE: At the moment this tests only can be executed with cargo test -- --test-threads=1

/*
proptest! {

    #[cfg(not(loom))]
    #[test]
    fn test_atomic_lock_memory_free(num_readers in 1u8..6, num_writers in 1u8..6, num_worker_writes in 100u64..10000) {
        execute_u64(Atomic::new_lock(0), num_readers, num_writers, num_worker_writes)

    }

    #[cfg(not(loom))]
    #[test]
    fn test_atomic_cas_memory_free(num_readers in 1u8..6, num_writers in 1u8..6, num_worker_writes in 100u64..10000) {
    execute_u64(Atomic::new_cas(0, num_writers as u16), num_readers, num_writers, num_worker_writes)
    }

}
 */

/*



#[test]
fn test_atomic_lock_memory_free() {
    execute_u64(Atomic::new_lock(0), 12, 8, 1000000)
}


 */

#[test]
fn test_atomic_cas_memory_free() {
    execute_u64(Atomic::new_cas(0, 10000), 12, 8, 1000000)
}

// TODO: Test with ArcSwap
// TODO: Check overall algorithm
// TODO: Relax ordering constraints
// TODO: Track also in the test how much reads are done to see the real performance benefit also at read time.

#[cfg(loom)]
#[test]
fn test_atomic_lock_memory_free() {
    loom::model(|| {
        execute_u64(Atomic::new_lock(0), 0, 1, 100);
    });
}

fn execute_u64<A: AtomicAccessControl + Send + Sync + 'static>(
    target: Atomic<u64, A>,
    num_readers: u8,
    num_writers: u8,
    num_worker_writes: u64,
) {
    let stop_fn = |val: ValueRef<u64>, total_writes: u64| total_writes.eq(val.get());
    let write_fn = |val: &u64| val + 1;

    let result = execute(
        target,
        num_readers,
        num_writers,
        num_worker_writes,
        stop_fn,
        write_fn,
    );

    assert_eq!(num_writers as u64 * num_worker_writes, result)
}

fn execute<T: Clone + Debug + 'static, A: AtomicAccessControl + Send + Sync + 'static>(
    target: Atomic<T, A>,
    num_readers: u8,
    num_writers: u8,
    num_worker_writes: u64,
    stop_fn: fn(ValueRef<T>, u64) -> bool,
    write_fn: fn(&T) -> T,
) -> T {
    #[cfg(not(loom))]
    GLOBAL_ALLOCATOR.reset();

    let target = Arc::new(target);
    let total_writes: u64 = num_writers as u64 * num_worker_writes;
    let writers = init_writers(
        Arc::clone(&target),
        num_writers,
        num_worker_writes,
        write_fn,
    );
    let readers = init_readers(Arc::clone(&target), num_readers, total_writes, stop_fn);
    readers.into_iter().for_each(|handle| {
        let _ = handle.join();
    });
    writers.into_iter().for_each(|handle| {
        let _ = handle.join();
    });

    let result = target.read().get().clone();
    drop(target);

    #[cfg(not(loom))]
    assert_eq!(
        GLOBAL_ALLOCATOR.allocs.load(Ordering::Relaxed) + EXTRA_ALLOCS,
        GLOBAL_ALLOCATOR.deallocs.load(Ordering::Relaxed)
    );

    result
}

fn init_writers<T: Debug + 'static, A: AtomicAccessControl + Send + Sync + 'static>(
    target: Arc<Atomic<T, A>>,
    num: u8,
    num_worker_writes: u64,
    write_fn: fn(&T) -> T,
) -> Vec<JoinHandle<()>> {
    (0..num)
        .map(|idx| {
            let target = Arc::clone(&target);
            thread::spawn(move || {
                let mut i = 0;
                while i < num_worker_writes {
                    target.write(write_fn);
                    i += 1;
                }

                println!("#{} Write Worker finished!", idx);
            })
        })
        .collect::<Vec<_>>()
}

fn init_readers<T: Debug + 'static, A: AtomicAccessControl + Send + Sync + 'static>(
    target: Arc<Atomic<T, A>>,
    num: u8,
    total_writes: u64,
    stop_fn: fn(ValueRef<T>, u64) -> bool,
) -> Vec<JoinHandle<()>> {
    (0..num)
        .map(|_| {
            let target = Arc::clone(&target);
            thread::spawn(move || {
                loop {
                    if stop_fn(target.read(), total_writes) {
                        break;
                    }

                    thread::yield_now();
                }
            })
        })
        .collect::<Vec<_>>()
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
