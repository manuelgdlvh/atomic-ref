mod atomic;

use crate::atomic::Atomic;

// TODO: Add concurrency tests
// TODO: Add performance tests
// TODO: Add starvation mechanism
// TODO: Change store to update mechanism using functions to allow disjoint concurrency

fn main() {
    let atomic = Atomic::new("test".to_string());
    let result = atomic.read();
    atomic.write("test2".to_string());
    let result2 = atomic.read();

    println!("{:?}", result.get());
    println!("{:?}", result2.get());
}
