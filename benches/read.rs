use std::{sync::Arc, time::Duration};

use arc_swap::ArcSwap;
use criterion::{Criterion, criterion_group, criterion_main};
use lib::{
    atomic::Atomic,
    tests::{ReadTask, WriteTask, runtime},
};

const READERS: usize = 5;
const WRITERS: usize = 0;

fn cas_read(c: &mut Criterion) {
    perform(c, "Read - AtomicRef CAS", Atomic::new_cas(0, u16::MAX));
}

#[cfg(feature = "benches")]
fn arc_swap_read(c: &mut Criterion) {
    perform(c, "Read - ArcSwap", ArcSwap::from_pointee(0));
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
            handle.read(ReadTask::TargetHits { hits: 1000000 });
            handle.recv_results(READERS + WRITERS, Duration::from_secs(25));
        });
    });
}

#[cfg(feature = "benches")]
criterion_group!(benches, cas_read, arc_swap_read);

#[cfg(not(feature = "benches"))]
criterion_group!(benches, cas_read);
criterion_main!(benches);
