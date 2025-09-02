use std::{fmt::Debug, sync::Arc, time::Duration};

use arc_swap::ArcSwap;
use criterion::{Criterion, criterion_group, criterion_main};
use lib::{
    atomic::Atomic,
    tests::{ReadTask, WriteTask, runtime},
};

const READERS: usize = 0;
const WRITERS: usize = 5;
const WRITE_EXECS: usize = 200000;

fn cas_write(c: &mut Criterion) {
    perform(c, "Write - AtomicRef CAS", Atomic::new_cas(0, u16::MAX));
}

#[cfg(feature = "benches")]
fn arc_swap_write(c: &mut Criterion) {
    perform(c, "Write - ArcSwap", ArcSwap::from_pointee(0));
}

fn perform<'a, T: lib::tests::ReadWriteExt<usize> + 'static>(
    c: &'a mut Criterion,
    name: &'static str,
    target: T,
) {
    let target = Arc::new(target);
    c.bench_function(name, |b| {
        let handle = runtime(READERS, WRITERS, target.clone());

        b.iter(|| {
            handle.read(ReadTask::Simple {
                stop_fn: |val: &usize| {
                    if *val == (WRITERS * WRITE_EXECS) {
                        true
                    } else {
                        false
                    }
                },
            });

            handle.write(WriteTask::Simple {
                num_execs: WRITE_EXECS,
                task: |val: &usize| *val + 1,
            });

            handle.recv_results(READERS + WRITERS, Duration::from_secs(25));
            handle.write(WriteTask::Reset);
            handle.recv_results(WRITERS, Duration::from_secs(25));
        });
    });
}

#[cfg(feature = "benches")]
criterion_group!(benches, cas_write, arc_swap_write);

#[cfg(not(feature = "benches"))]
criterion_group!(benches, cas_write);
criterion_main!(benches);
