use std::{sync::Arc, time::Duration};

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
    c.bench_function("Atomic Ref - CAS Access Control", |b| {
        let handle = runtime(READERS, WRITERS, Arc::new(Atomic::new_cas(0, u16::MAX)));

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

fn arc_swap_write(c: &mut Criterion) {
    c.bench_function("Arc Swap", |b| {
        let handle = runtime(READERS, WRITERS, Arc::new(ArcSwap::from_pointee(0)));

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
criterion_group!(benches, cas_write, arc_swap_write);
criterion_main!(benches);
