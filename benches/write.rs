use criterion::{criterion_group, criterion_main, Criterion};
use lib::access::AtomicAccessControl;
use lib::atomic::Atomic;
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
        b.iter(|| execute_u64(Atomic::new_cas(0, u16::MAX), 5, 5, 10000));
    });
}

fn lock_write(c: &mut Criterion) {
    c.bench_function("Atomic Ref - Lock Access Control", |b| {
        b.iter(|| execute_u64(Atomic::new_lock(0), 5, 5, 10000));
    });
}

criterion_group!(benches, cas_write, lock_write);
criterion_main!(benches);
