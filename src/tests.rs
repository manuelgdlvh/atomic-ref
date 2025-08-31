// This module exposes functions to easily perform performance and correctness tests maintaining consistency across all tests.
// This must decouple the Runtime which will execute all actions and is able to be reused (very useful for performance tests).
// Also must be decoupled of the inner types used, for example can be used to compare against other crates.

use std::{
    fmt::Debug,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

use arc_swap::ArcSwap;

use crate::{access::AtomicAccessControl, atomic::Atomic};

#[derive(Clone)]
pub enum ReadTask<I: Clone + Debug> {
    Simple { stop_fn: fn(&I) -> bool },
    Stop,
}

#[derive(Clone, Copy)]
pub enum WriteTask<I: Clone + Debug> {
    Simple { num_execs: usize, task: fn(&I) -> I },
    Reset,
    Stop,
}

pub enum TaskResult {
    SimpleReadDone,
    SimpleWriteDone,
}

pub struct RuntimeHandle<I: Clone + Debug> {
    readers: Vec<SyncSender<ReadTask<I>>>,
    writers: Vec<SyncSender<WriteTask<I>>>,
    res_recv: Receiver<TaskResult>,
}

impl<I: Clone + Debug> RuntimeHandle<I> {
    pub fn new(num_readers: usize, num_writers: usize) -> (Self, SyncSender<TaskResult>) {
        let (res_tx, res_rx) = mpsc::sync_channel(num_readers + num_writers);

        let self_ = Self {
            readers: vec![],
            writers: vec![],
            res_recv: res_rx,
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

    pub fn recv_results(&self, expected: usize, timeout: std::time::Duration) -> Vec<TaskResult> {
        (0..expected)
            .map(|_| {
                self.res_recv
                    .recv_timeout(timeout)
                    .expect("Should retrieve results before defined time")
            })
            .collect()
    }
}

impl<I: Clone + Debug> Drop for RuntimeHandle<I> {
    fn drop(&mut self) {
        self.readers.iter().for_each(|channel| {
            channel.send(ReadTask::Stop).expect("");
        });

        self.writers.iter().for_each(|channel| {
            channel.send(WriteTask::Stop).expect("");
        });
    }
}

pub fn runtime<
    I: Send + Default + Clone + Debug + 'static,
    T: ReadWriteExt<I> + Send + Sync + 'static,
>(
    num_readers: usize,
    num_writers: usize,
    target: Arc<T>,
) -> RuntimeHandle<I> {
    let (mut r_handle, res_tx) = RuntimeHandle::<I>::new(num_readers, num_writers);

    (0..num_readers).for_each(|_| {
        let task_rx = r_handle.register_reader();
        let res_tx = res_tx.clone();
        let target = target.clone();
        thread::spawn(move || {
            loop {
                match task_rx
                    .recv()
                    .expect("Should receive stop before handle be dropped")
                {
                    ReadTask::Stop => {
                        break;
                    }
                    ReadTask::Simple { stop_fn } => {
                        while !stop_fn(&target.read()) {
                            thread::yield_now();
                        }

                        res_tx.send(TaskResult::SimpleReadDone).expect("");
                    }
                }
            }
        });
    });

    (0..num_writers).for_each(|_| {
        let task_rx = r_handle.register_writer();
        let res_tx = res_tx.clone();
        let target = target.clone();

        thread::spawn(move || {
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

                        res_tx.send(TaskResult::SimpleWriteDone).expect("");
                    }
                    WriteTask::Reset => {
                        target.write_fn(|_| I::default());

                        res_tx.send(TaskResult::SimpleWriteDone).expect("");
                    }
                }
            }
        });
    });

    r_handle
}

pub trait ReadWriteExt<I: Debug> {
    fn read(&self) -> Arc<I>;
    fn write_fn(&self, fn_ptr: fn(&I) -> I);
}

impl<I: Debug> ReadWriteExt<I> for ArcSwap<I> {
    fn read(&self) -> Arc<I> {
        self.load_full()
    }

    fn write_fn(&self, fn_ptr: fn(&I) -> I) {
        self.rcu(|inner| fn_ptr(&*inner));
    }
}

impl<A: AtomicAccessControl, I: Debug> ReadWriteExt<I> for Atomic<I, A> {
    fn read(&self) -> Arc<I> {
        self.read()
    }
    fn write_fn(&self, fn_ptr: fn(&I) -> I) {
        self.write(fn_ptr);
    }
}

// Should allow to send actions to readers and writers and get in other channel when finished the task itself.
// For writers, pass some function and how many times this function must to be executed. And for readers, some stop_fn and perform without stop until reach the goal and send the finish result.
// Explain here how compose both actions to reach whatever desired goal. For the cases at the moment, we need to support current performance and correctness checks developed.
