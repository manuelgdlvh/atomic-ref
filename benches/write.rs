use arc_swap::ArcSwap;
use criterion::{Criterion, criterion_group, criterion_main};
use lib::access::AtomicAccessControl;
use lib::atomic::Atomic;
use proptest::prelude::Strategy;
use std::fmt::Debug;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;

fn execute_u64<A: AtomicAccessControl + Send + Sync + 'static>(
    target: Atomic<u64, A>,
    num_readers: u8,
    num_writers: u8,
    num_worker_writes: u64,
) {
    let stop_fn = |val: Arc<u64>, total_writes: u64| total_writes.eq(val.as_ref());
    let write_fn = |val: &u64| val + 1;

    execute(
        target,
        num_readers,
        num_writers,
        num_worker_writes,
        stop_fn,
        write_fn,
    );
}

fn execute<T: Clone + Debug + 'static, A: AtomicAccessControl + Send + Sync + 'static>(
    target: Atomic<T, A>,
    num_readers: u8,
    num_writers: u8,
    num_worker_writes: u64,
    stop_fn: fn(Arc<T>, u64) -> bool,
    write_fn: fn(&T) -> T,
) {
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
            })
        })
        .collect::<Vec<_>>()
}

fn init_readers<T: Debug + 'static, A: AtomicAccessControl + Send + Sync + 'static>(
    target: Arc<Atomic<T, A>>,
    num: u8,
    total_writes: u64,
    stop_fn: fn(Arc<T>, u64) -> bool,
) -> Vec<JoinHandle<()>> {
    (0..num)
        .map(|idx| {
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

fn cas_write(c: &mut Criterion) {
    c.bench_function("Atomic Ref - CAS Access Control", |b| {
        b.iter(|| execute_u64(Atomic::new_cas(0, u16::MAX), 20, 8, 50000));
    });
}

fn lock_write(c: &mut Criterion) {
    c.bench_function("Atomic Ref - Lock Access Control", |b| {
        b.iter(|| execute_u64(Atomic::new_lock(0), 20, 8, 50000));
    });
}

fn arc_swap_write(c: &mut Criterion) {
    c.bench_function("Arc Swap", |b| {
        let readers = 20;
        let writers = 8;
        let writes_per_worker = 50000;

        b.iter(|| {
            let target = Arc::new(ArcSwap::from_pointee(0));
            let readers: Vec<JoinHandle<()>> = (0..readers)
                .map(|idx| {
                    let target = Arc::clone(&target);
                    thread::spawn(move || {
                        loop {
                            if *target.load_full() == (writes_per_worker * writers) {
                                break;
                            }

                            thread::yield_now();
                        }
                    })
                })
                .collect();

            let writers: Vec<JoinHandle<()>> = (0..writers)
                .map(|idx| {
                    let target = Arc::clone(&target);
                    thread::spawn(move || {
                        let mut i = 0;

                        while (i < writes_per_worker) {
                            target.rcu(|val| **val + 1);

                            i += 1;
                        }
                    })
                })
                .collect();

            readers.into_iter().for_each(|reader| {
                reader.join().unwrap();
            });

            writers.into_iter().for_each(|writer| {
                writer.join().unwrap();
            });
        });
    });
}

criterion_group!(benches, cas_write, arc_swap_write, lock_write);
criterion_main!(benches);
