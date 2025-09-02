pub mod cas;
pub mod lock;

pub trait AccessGuard {}

pub trait AtomicAccessControl: Send + Sync {
    fn write(&self) -> impl AccessGuard;
    fn read(&self) -> impl AccessGuard;
}
