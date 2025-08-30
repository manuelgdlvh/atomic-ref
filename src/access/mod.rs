pub mod cas;
pub mod lock;

pub trait AccessGuard {}

pub trait AtomicAccessControl {
    fn write(&self) -> impl AccessGuard;
    fn read(&self) -> impl AccessGuard;
}
