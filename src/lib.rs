use log::{debug, info};
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod api;
pub mod builder;
#[cfg(feature = "client")]
pub mod client;
pub mod comm;
#[cfg(feature = "eva")]
pub mod eapi;
pub mod interval;
pub mod io;
pub mod server;
pub mod tasks;

pub mod prelude {
    pub use super::{init_plc, plc_context, plc_context_mut, run_plc};
    pub use log::{debug, error, info, trace, warn};
    pub use rplc_derive::plc_program;
}

pub mod export {
    pub use eva_common;
    #[cfg(feature = "eva")]
    pub use eva_sdk;
    pub use log;
    pub use once_cell;
    #[cfg(feature = "opcua")]
    pub use opcua;
    pub use parking_lot;
    #[cfg(feature = "modbus")]
    pub use rmodbus;
    pub use serde;
}

pub type LockedContext<C> = RwLock<C>;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);
pub const DEFAULT_STOP_TIMEOUT: f64 = 30.0;
pub static NAME: OnceCell<String> = OnceCell::new();
pub static DESCRIPTION: OnceCell<String> = OnceCell::new();
pub static VERSION: OnceCell<String> = OnceCell::new();
pub static CPUS: OnceCell<usize> = OnceCell::new();

static HOSTNAME: OnceCell<String> = OnceCell::new();
static STARTUP_TIME: OnceCell<Instant> = OnceCell::new();

static TERM_FLAG: Lazy<Arc<atomic::AtomicBool>> =
    Lazy::new(|| Arc::new(atomic::AtomicBool::new(false)));

fn sigterm_received() -> bool {
    TERM_FLAG.load(atomic::Ordering::SeqCst)
}

#[derive(Serialize, Deserialize, Default)]
pub struct PlcInfo {
    pub system_name: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub status: i16,
    pub pid: u32,
    pub uptime: f64,
}

pub(crate) fn plc_info() -> PlcInfo {
    PlcInfo {
        system_name: HOSTNAME.get().unwrap().clone(),
        name: NAME.get().unwrap().clone(),
        description: DESCRIPTION.get().unwrap().clone(),
        version: VERSION.get().unwrap().clone(),
        status: tasks::status() as i16,
        pid: process::id(),
        uptime: uptime().as_secs_f64(),
    }
}

/// # Panics
///
/// Will panic if PLC is not initialized
#[inline]
pub fn hostname() -> &'static str {
    HOSTNAME.get().unwrap()
}

/// # Panics
///
/// Will panic if PLC is not initialized
#[inline]
pub fn uptime() -> Duration {
    STARTUP_TIME.get().unwrap().elapsed()
}

/// use init_plc!() macro to init the PLC
///
/// # Panics
///
/// Will panic if syslog is selected but can not be connected
pub fn init(name: &str, description: &str, version: &str) {
    panic::set_hook(Box::new(|s| {
        println!("PANIC: {}", s);
        std::process::exit(1);
    }));
    HOSTNAME
        .set(hostname::get().unwrap().to_string_lossy().to_string())
        .unwrap();
    STARTUP_TIME.set(Instant::now()).unwrap();
    NAME.set(name.to_owned()).unwrap();
    DESCRIPTION.set(description.to_owned()).unwrap();
    VERSION.set(version.to_owned()).unwrap();
    let verbose: bool = env::var("VERBOSE").ok().map_or(false, |v| v == "1");
    let syslog: bool = env::var("SYSLOG").ok().map_or(false, |v| v == "1");
    if syslog {
        let formatter = syslog::Formatter3164 {
            facility: syslog::Facility::LOG_USER,
            hostname: None,
            process: name.to_owned(),
            pid: std::process::id(),
        };
        log::set_boxed_logger(Box::new(syslog::BasicLogger::new(
            syslog::unix(formatter).unwrap(),
        )))
        .unwrap();
        log::set_max_level(if verbose {
            log::LevelFilter::Trace
        } else {
            log::LevelFilter::Info
        });
    } else {
        env_logger::Builder::new()
            .target(env_logger::Target::Stdout)
            .filter_level(if verbose {
                log::LevelFilter::Trace
            } else {
                log::LevelFilter::Info
            })
            .init();
    }
    debug!("log initialization completed");
    tasks::init();
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! init_plc {
    () => {
        ::rplc::init(
            crate::plc::NAME,
            crate::plc::DESCRIPTION,
            crate::plc::VERSION,
        );
    };
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! plc_context {
    () => {
        crate::plc::context::CONTEXT.read()
    };
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! plc_context_mut {
    () => {
        crate::plc::context::CONTEXT.write()
    };
}

/// # Panics
///
/// Will panic if unable to register SIGTERM/SIGINT handler
fn register_signals() {
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&TERM_FLAG)).unwrap();
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&TERM_FLAG)).unwrap();
}

pub fn var_dir() -> PathBuf {
    env::var("PLC_VAR_DIR").map_or_else(|_| env::temp_dir(), |p| Path::new(&p).to_owned())
}

pub(crate) fn name() -> &'static str {
    NAME.get().map(String::as_str).unwrap()
}

/// use run_plc!() macro to run the PLC
///
/// # Panics
///
/// Will panic if unable to write/remove the pid file/api socket or if PLC is not intialized
pub fn run<F: Fn(), F2: Fn()>(
    launch_datasync: &F,
    stop_datasync: &F2,
    stop_timeout: Duration,
    _eapi_action_pool_size: usize,
) {
    tasks::set_starting();
    let name = NAME.get().expect("PLC not initialized");
    let description = DESCRIPTION.get().unwrap();
    let version = VERSION.get().unwrap();
    let mut msg = format!("{} {}", name, version);
    if !description.is_empty() {
        let _ = write!(msg, " ({})", description);
    }
    info!("system: {}, cpus: {}", HOSTNAME.get().unwrap(), cpus());
    info!("{}", msg);
    register_signals();
    #[cfg(feature = "eva")]
    eapi::launch(_eapi_action_pool_size);
    launch_datasync();
    tasks::set_syncing();
    tasks::set_preparing_if_no_inputs();
    tasks::set_active_if_no_inputs_and_programs();
    let pid = process::id();
    let mut pid_file = var_dir();
    pid_file.push(format!("{}.pid", name));
    fs::write(&pid_file, pid.to_string()).unwrap();
    let socket_path = api::spawn_api();
    while !sigterm_received() {
        tasks::step_sleep();
        check_health();
    }
    tasks::spawn0(move || {
        tasks::sleep(stop_timeout);
        panic!("timeout has been reached, FORCE STOP");
    });
    if tasks::status() == tasks::Status::Active {
        tasks::shutdown();
        if tasks::status() != tasks::Status::Stopped {
            stop_datasync();
        }
        while tasks::status() != tasks::Status::Stopped {
            tasks::step_sleep();
            check_health();
        }
    } else {
        tasks::set_stopped();
    }
    fs::remove_file(pid_file).unwrap();
    fs::remove_file(socket_path).unwrap();
}

fn check_health() {
    //tasks::check_health();
}

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! run_plc {
    () => {
        ::rplc::run(
            &crate::plc::io::launch_datasync,
            &crate::plc::io::stop_datasync,
            crate::plc::STOP_TIMEOUT,
            crate::plc::EAPI_ACTION_POOL_SIZE,
        );
    };
}

pub fn cpus() -> usize {
    if let Some(cpus) = CPUS.get() {
        *cpus
    } else {
        let cpus = if let Ok(s) = std::fs::read_to_string("/proc/cpuinfo") {
            let mut c = 0;
            for line in s.split('\n') {
                if line.starts_with("processor\t") {
                    c += 1;
                }
            }
            c
        } else {
            0
        };
        let _ = CPUS.set(cpus);
        cpus
    }
}

// TODO custom action handlers, long action examples, kill and terminate support
// TODO allows programs which are called via BUS/RT as lmacros

// TODO hostmaster daemon (BUS/RT)
// TODO hostmaster daemon (web ui, plc logs)
// TODO BUS/RT fieldbus (direct context exchange between PLCs)

// TODO vs code plugins for context and mappings
// TODO freertos support
// TODO sync context with EthernetIP structures
// TODO sync context with CANOpen registers
// TODO sync context with TwinCAT registers
// TODO chains: inputs, programs, outputs
