// This module exposes functions to easily perform performance and correctness tests maintaining consistency across all tests.

use std::{
    fmt::Debug,
    mem,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};

use crate::{access::AtomicAccessControl, atomic::Atomic};

pub enum ReadTask<I: Debug + Send> {
    ReadUntil {
        stop_fn: Arc<dyn Fn(&I) -> bool + Send + Sync>,
    },
    TargetHits {
        hits: usize,
    },
    Stop,
}

impl<I: Debug + Send> Clone for ReadTask<I> {
    fn clone(&self) -> Self {
        match self {
            ReadTask::ReadUntil { stop_fn } => ReadTask::ReadUntil {
                stop_fn: stop_fn.clone(),
            },
            ReadTask::TargetHits { hits } => ReadTask::TargetHits { hits: *hits },
            ReadTask::Stop => ReadTask::Stop,
        }
    }
}

pub enum WriteTask<I: Debug + Send> {
    Simple { num_execs: usize, task: fn(&I) -> I },
    Reset,
    Stop,
}

impl<I: Debug + Send> Clone for WriteTask<I> {
    fn clone(&self) -> Self {
        match self {
            WriteTask::Reset => WriteTask::Reset,
            WriteTask::Simple { num_execs, task } => WriteTask::Simple {
                num_execs: *num_execs,
                task: *task,
            },
            WriteTask::Stop => WriteTask::Stop,
        }
    }
}

pub enum TaskResult<I: Debug + Send> {
    ReadUntil(Arc<I>),
    Done,
}

pub struct RuntimeHandle<I: Debug + Send> {
    readers: Vec<SyncSender<ReadTask<I>>>,
    writers: Vec<SyncSender<WriteTask<I>>>,
    res_recv: Receiver<TaskResult<I>>,
    workers: Vec<JoinHandle<()>>,
}

impl<I: Debug + Send> RuntimeHandle<I> {
    pub fn new(num_readers: usize, num_writers: usize) -> (Self, SyncSender<TaskResult<I>>) {
        let (res_tx, res_rx) = mpsc::sync_channel(num_readers + num_writers);

        let self_ = Self {
            readers: vec![],
            writers: vec![],
            res_recv: res_rx,
            workers: vec![],
        };

        (self_, res_tx)
    }

    pub fn register_reader(&mut self) -> Receiver<ReadTask<I>> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.readers.push(tx);
        rx
    }

    pub fn register_writer(&mut self) -> Receiver<WriteTask<I>> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.writers.push(tx);
        rx
    }

    pub fn write(&self, task: WriteTask<I>) {
        self.writers
            .iter()
            .for_each(|channel| channel.send(task.clone()).expect(""));
    }

    pub fn read(&self, task: ReadTask<I>) {
        self.readers.iter().for_each(|channel| {
            channel.send(task.clone()).expect("");
        });
    }

    pub fn recv_results(
        &self,
        expected: usize,
        timeout: std::time::Duration,
    ) -> Vec<TaskResult<I>> {
        (0..expected)
            .map(|_| {
                self.res_recv
                    .recv_timeout(timeout)
                    .expect("Should retrieve results before defined time")
            })
            .collect()
    }
}

impl<I: Debug + Send> Drop for RuntimeHandle<I> {
    fn drop(&mut self) {
        self.readers.iter().for_each(|channel| {
            channel.send(ReadTask::Stop).expect("");
        });

        self.writers.iter().for_each(|channel| {
            channel.send(WriteTask::Stop).expect("");
        });

        let workers = mem::take(&mut self.workers);
        for worker in workers {
            worker.join().expect("");
        }
    }
}

pub fn runtime<I: Send + Sync + Default + Debug + 'static, T: ReadWriteExt<I> + 'static>(
    num_readers: usize,
    num_writers: usize,
    target: Arc<T>,
) -> RuntimeHandle<I> {
    let (mut r_handle, res_tx) = RuntimeHandle::<I>::new(num_readers, num_writers);

    (0..num_readers).for_each(|_| {
        let task_rx = r_handle.register_reader();
        let res_tx = res_tx.clone();
        let target = target.clone();
        let worker = thread::spawn(move || {
            loop {
                match task_rx
                    .recv()
                    .expect("Should receive stop before handle be dropped")
                {
                    ReadTask::Stop => {
                        break;
                    }
                    ReadTask::ReadUntil { stop_fn } => {
                        let mut last_read;
                        loop {
                            last_read = target.read();

                            if stop_fn(&last_read) {
                                break;
                            }
                            thread::yield_now();
                        }

                        res_tx.send(TaskResult::ReadUntil(last_read)).expect("");
                    }

                    ReadTask::TargetHits { hits } => {
                        let mut i = 0;
                        while i < hits {
                            std::hint::black_box({
                                target.read();
                            });
                            i += 1;
                        }

                        res_tx.send(TaskResult::Done).expect("");
                    }
                }
            }
        });
        r_handle.workers.push(worker);
    });

    (0..num_writers).for_each(|_| {
        let task_rx = r_handle.register_writer();
        let res_tx = res_tx.clone();
        let target = target.clone();

        let worker = thread::spawn(move || {
            loop {
                match task_rx
                    .recv()
                    .expect("Should receive stop before handle be dropped")
                {
                    WriteTask::Stop => {
                        break;
                    }
                    WriteTask::Simple { num_execs, task } => {
                        let mut iter = 0;

                        while iter < num_execs {
                            target.write_fn(task);
                            iter += 1;
                        }

                        res_tx.send(TaskResult::Done).expect("");
                    }
                    WriteTask::Reset => {
                        target.write_fn(|_| I::default());

                        res_tx.send(TaskResult::Done).expect("");
                    }
                }
            }
        });

        r_handle.workers.push(worker);
    });

    r_handle
}

pub trait ReadWriteExt<I: Debug + Send + Sync>: Send + Sync {
    fn read(&self) -> Arc<I>;
    fn write_fn(&self, fn_ptr: fn(&I) -> I);
}

#[cfg(feature = "benches")]
use arc_swap::ArcSwap;

#[cfg(feature = "benches")]
impl<I: Debug + Send + Sync> ReadWriteExt<I> for ArcSwap<I> {
    fn read(&self) -> Arc<I> {
        self.load_full()
    }

    fn write_fn(&self, fn_ptr: fn(&I) -> I) {
        self.rcu(|inner| fn_ptr(&*inner));
    }
}

impl<A: AtomicAccessControl, I: Debug + Send + Sync> ReadWriteExt<I> for Atomic<I, A> {
    fn read(&self) -> Arc<I> {
        self.read()
    }
    fn write_fn(&self, fn_ptr: fn(&I) -> I) {
        self.write(fn_ptr);
    }
}
