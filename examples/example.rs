use lib::atomic::Atomic;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

// TODO: Add concurrency tests
// TODO: Add performance tests

const WRITER_WORKERS: u32 = 16;
const READER_WORKERS: u32 = 16;
const TOTAL_WRITES: u32 = 1000000;
const WRITES_PER_WORKER: u32 = TOTAL_WRITES / WRITER_WORKERS;

fn main() {
    {
        let atomic = Arc::new(Atomic::new_cas(0));

        if TOTAL_WRITES % WRITER_WORKERS != 0 {
            panic!("WRITES_PER_WORKER must be integer number");
        }

        // Add global registry to park/unpark when events happens to avoid resource starvation

        let writers = (0..WRITER_WORKERS)
            .map(|idx| {
                let atomic = atomic.clone();
                thread::spawn(move || {
                    let mut i = 0;
                    while i < WRITES_PER_WORKER {
                        atomic.write(|current| current + 1);
                        i += 1;
                    }

                    println!("#{} Write Worker finished!", idx);
                })
            })
            .collect::<Vec<_>>();

        let readers = (0..READER_WORKERS)
            .map(|idx| {
                let atomic = atomic.clone();
                thread::spawn(move || {
                    loop {
                        let value = *atomic.read().get();
                        if value == TOTAL_WRITES {
                            println!("#{} Read Worker finished!. Value: {}", idx, value);
                            break;
                        }

                        thread::sleep(Duration::from_millis(50));
                    }
                })
            })
            .collect::<Vec<_>>();

        readers
            .into_iter()
            .for_each(|handle| handle.join().unwrap());
        writers
            .into_iter()
            .for_each(|handle| handle.join().unwrap());

        println!("#{:?} VALUE.", atomic.read().get());
        println!("#{:?} VERSION.", atomic.read().version());
    }

    println!("#{:?} Done!", GLOBAL_ALLOCATOR);
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator::new();
#[derive(Debug)]
pub struct CountingAllocator {
    pub allocs: AtomicUsize,
    pub deallocs: AtomicUsize,
    pub bytes_allocated: AtomicUsize,
    pub bytes_deallocated: AtomicUsize,
}

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
