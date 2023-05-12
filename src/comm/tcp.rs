use super::Comm;
use parking_lot::{Mutex, MutexGuard};
use std::error::Error;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

#[allow(clippy::module_name_repetitions)]
pub struct TcpComm {
    addr: SocketAddr,
    stream: Mutex<Option<TcpStream>>,
    timeout: Duration,
    busy: Mutex<()>,
}

#[allow(clippy::module_name_repetitions)]
pub type TcpCommunicator = Arc<TcpComm>;

macro_rules! handle_tcp_stream_error {
    ($stream: expr, $err: expr, $any: expr) => {{
        if $any || $err.kind() == std::io::ErrorKind::TimedOut {
            $stream.take();
        }
        $err
    }};
}

impl Comm for TcpComm {
    fn lock(&self) -> MutexGuard<()> {
        self.busy.lock()
    }
    fn reconnect(&self) {
        self.stream.lock().take();
    }
    fn write(&self, buf: &[u8]) -> Result<(), std::io::Error> {
        let mut stream = self.get_stream()?;
        stream
            .as_mut()
            .unwrap()
            .write_all(buf)
            .map_err(|e| handle_tcp_stream_error!(stream, e, true))
    }
    fn read_exact(&self, buf: &mut [u8]) -> Result<(), std::io::Error> {
        let mut stream = self.get_stream()?;
        stream
            .as_mut()
            .unwrap()
            .read_exact(buf)
            .map_err(|e| handle_tcp_stream_error!(stream, e, false))
    }
}

impl TcpComm {
    pub fn create(path: &str, timeout: Duration) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            addr: path.parse()?,
            stream: <_>::default(),
            busy: <_>::default(),
            timeout,
        })
    }
    fn get_stream(&self) -> Result<MutexGuard<Option<TcpStream>>, std::io::Error> {
        let mut lock = self.stream.lock();
        if lock.as_mut().is_none() {
            let stream = TcpStream::connect_timeout(&self.addr, self.timeout)?;
            stream.set_read_timeout(Some(self.timeout))?;
            stream.set_write_timeout(Some(self.timeout))?;
            stream.set_nodelay(true)?;
            lock.replace(stream);
        }
        Ok(lock)
    }
}
