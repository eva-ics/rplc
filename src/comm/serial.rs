use super::Comm;
use parking_lot::{Mutex, MutexGuard};
use serial::prelude::*;
use serial::SystemPort;
use std::error::Error;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

fn parse_path(
    path: &str,
) -> (
    &str,
    serial::BaudRate,
    serial::CharSize,
    serial::Parity,
    serial::StopBits,
) {
    let mut sp = path.split(':');
    let port_dev = sp.next().unwrap();
    let s_baud_rate = sp
        .next()
        .unwrap_or_else(|| panic!("serial baud rate not specified: {}", path));
    let s_char_size = sp
        .next()
        .unwrap_or_else(|| panic!("serial char size not specified: {}", path));
    let s_parity = sp
        .next()
        .unwrap_or_else(|| panic!("serial parity not specified: {}", path));
    let s_stop_bits = sp
        .next()
        .unwrap_or_else(|| panic!("serial stopbits not specified: {}", path));
    let baud_rate = match s_baud_rate {
        "110" => serial::Baud110,
        "300" => serial::Baud300,
        "600" => serial::Baud600,
        "1200" => serial::Baud1200,
        "2400" => serial::Baud2400,
        "4800" => serial::Baud4800,
        "9600" => serial::Baud9600,
        "19200" => serial::Baud19200,
        "38400" => serial::Baud38400,
        "57600" => serial::Baud57600,
        "115200" => serial::Baud115200,
        v => panic!("specified serial baud rate not supported: {}", v),
    };
    let char_size = match s_char_size {
        "5" => serial::Bits5,
        "6" => serial::Bits6,
        "7" => serial::Bits7,
        "8" => serial::Bits8,
        v => panic!("specified serial char size not supported: {}", v),
    };
    let parity = match s_parity {
        "N" => serial::ParityNone,
        "E" => serial::ParityEven,
        "O" => serial::ParityOdd,
        v => panic!("specified serial parity not supported: {}", v),
    };
    let stop_bits = match s_stop_bits {
        "1" => serial::Stop1,
        "2" => serial::Stop2,
        v => unimplemented!("specified serial stop bits not supported: {}", v),
    };
    (port_dev, baud_rate, char_size, parity, stop_bits)
}

/// # Panics
///
/// Will panic on misconfigured listen string
pub fn check_path(path: &str) {
    let _ = parse_path(path);
}

/// # Panics
///
/// Will panic on misconfigured listen string
pub fn open(listen: &str, timeout: Duration) -> Result<SystemPort, serial::Error> {
    let (port_dev, baud_rate, char_size, parity, stop_bits) = parse_path(listen);
    let mut port = serial::open(&port_dev)?;
    port.reconfigure(&|settings| {
        (settings.set_baud_rate(baud_rate).unwrap());
        settings.set_char_size(char_size);
        settings.set_parity(parity);
        settings.set_stop_bits(stop_bits);
        settings.set_flow_control(serial::FlowNone);
        Ok(())
    })?;
    port.set_timeout(timeout)?;
    Ok(port)
}

#[allow(clippy::module_name_repetitions)]
pub struct SerialComm {
    path: String,
    port: Mutex<Option<SystemPort>>,
    timeout: Duration,
    busy: Mutex<()>,
}

#[allow(clippy::module_name_repetitions)]
pub type SerialCommunicator = Arc<SerialComm>;

impl Comm for SerialComm {
    fn lock(&self) -> MutexGuard<()> {
        self.busy.lock()
    }
    fn reconnect(&self) {
        self.port.lock().take();
    }
    fn write(&self, buf: &[u8]) -> Result<(), std::io::Error> {
        let mut port = self.get_port()?;
        port.as_mut().unwrap().write_all(buf).map_err(|e| {
            self.reconnect();
            e
        })
    }
    fn read_exact(&self, buf: &mut [u8]) -> Result<(), std::io::Error> {
        let mut port = self.get_port()?;
        port.as_mut().unwrap().read_exact(buf).map_err(|e| {
            self.reconnect();
            e
        })
    }
}

impl SerialComm {
    /// # Panics
    ///
    /// Will panic on misconfigured path string
    pub fn create(path: &str, timeout: Duration) -> Result<Self, Box<dyn Error>> {
        check_path(path);
        Ok(Self {
            path: path.to_owned(),
            port: <_>::default(),
            busy: <_>::default(),
            timeout,
        })
    }
    fn get_port(&self) -> Result<MutexGuard<Option<SystemPort>>, std::io::Error> {
        let mut lock = self.port.lock();
        if lock.as_mut().is_none() {
            let port = open(&self.path, self.timeout)?;
            lock.replace(port);
        }
        Ok(lock)
    }
}
