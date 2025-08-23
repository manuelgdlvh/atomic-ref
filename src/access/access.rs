pub trait AccessGuard: Drop {}

pub trait AtomicAccessControl {
    fn write(&self) -> impl AccessGuard;
    fn read(&self) -> impl AccessGuard;
}
