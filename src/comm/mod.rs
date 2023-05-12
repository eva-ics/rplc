use parking_lot::MutexGuard;
use std::sync::Arc;

#[cfg(feature = "serial")]
pub mod serial;
pub mod tcp;

pub type Communicator = Arc<dyn Comm + Send + Sync>;

pub trait Comm {
    fn lock(&self) -> MutexGuard<()>;
    fn reconnect(&self);
    fn write(&self, buf: &[u8]) -> Result<(), std::io::Error>;
    fn read_exact(&self, buf: &mut [u8]) -> Result<(), std::io::Error>;
}
