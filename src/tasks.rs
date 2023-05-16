use crate::cpus;
use crate::interval::Loop;
use bmart_derive::EnumStr;
use eva_common::{EResult, Error};
use log::{debug, error, info, warn};
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use std::collections::{btree_map, BTreeMap};
use std::env;
use std::str::FromStr;
use std::sync::atomic;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

static CONTROLLER_STATS: Lazy<Mutex<ControllerStats>> = Lazy::new(<_>::default);
static WAIT_HANDLES: Lazy<Mutex<Option<Vec<thread::JoinHandle<()>>>>> = Lazy::new(<_>::default);
static STATS_TX: OnceCell<Mutex<mpsc::SyncSender<(String, u16)>>> = OnceCell::new();
static SHUTDOWN_FN: OnceCell<Box<dyn Fn() + Send + Sync>> = OnceCell::new();
static STATUS_CHANGED: Condvar = Condvar::new();
static STATUS_MUTEX: Mutex<()> = Mutex::new(());

static STATUS: atomic::AtomicI16 = atomic::AtomicI16::new(Status::Inactive as i16);

pub fn controller_stats() -> &'static Mutex<ControllerStats> {
    &CONTROLLER_STATS
}

pub const WAIT_STEP: Duration = Duration::from_secs(1);
pub const SLEEP_STEP: Duration = Duration::from_millis(500);
pub const SLEEP_STEP_ERR: Duration = Duration::from_secs(2);

const STATS_CHANNEL_SIZE: usize = 100_000;

pub(crate) fn init() {
    WAIT_HANDLES.lock().replace(<_>::default());
    let (tx, rx) = mpsc::sync_channel::<(String, u16)>(STATS_CHANNEL_SIZE);
    STATS_TX.set(Mutex::new(tx)).unwrap();
    spawn_service("stats", move || {
        while let Ok((name, jitter)) = rx.recv() {
            if let Some(entry) = CONTROLLER_STATS.lock().thread_stats.get_mut(&name) {
                entry.report_jitter(jitter);
            }
        }
    });
}

/// # Panics
///
/// Will panic if set twice
pub fn on_shutdown<F>(f: F)
where
    F: Fn() + Send + Sync + 'static,
{
    assert!(
        SHUTDOWN_FN.set(Box::new(f)).is_ok(),
        "Unable to set shutdown function. Has it been already set?"
    );
}

pub(crate) fn shutdown() {
    set_status(Status::Stopping);
    if let Some(wait_handles) = WAIT_HANDLES.lock().take() {
        if let Some(f) = SHUTDOWN_FN.get() {
            for handle in wait_handles {
                let _ = handle.join();
            }
            f();
        }
    } else {
        warn!("no wait handles, is shutdown called twice?");
    }
    set_status(Status::StopSyncing);
}

pub(crate) trait ConvX {
    fn as_u16_max(&self) -> u16;
}

macro_rules! impl_convx {
    ($t: ty) => {
        impl ConvX for $t {
            fn as_u16_max(&self) -> u16 {
                let val = *self;
                if val > <$t>::from(u16::MAX) {
                    u16::MAX
                } else {
                    val as u16
                }
            }
        }
    };
}

impl_convx!(u32);
impl_convx!(u64);

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, EnumStr)]
#[repr(i16)]
#[enumstr(rename_all = "UPPERCASE")]
pub enum Status {
    Inactive = 0,     // plc is launched
    Starting = 1,     // plc is starting
    Syncing = 2,      // inputs can run
    Preparing = 3,    // programs can run
    Active = 100,     // outputs can run
    Stopping = -1,    // plc started shutdown, inputs and programs must quit
    StopSyncing = -2, // final data sync
    Stopped = -100,   // outputs completed, PLC is stopped
    Unknown = -200,
}

#[inline]
pub fn step_sleep() {
    sleep(SLEEP_STEP);
}

#[inline]
pub fn sleep(duration: Duration) {
    thread::sleep(duration);
}

#[inline]
pub fn step_sleep_err() {
    sleep(SLEEP_STEP_ERR);
}

pub fn thread_name() -> String {
    let th = thread::current();
    if let Some(name) = th.name() {
        name.to_owned()
    } else {
        format!("{:?}", th.id())
    }
}

#[derive(Eq, PartialEq, Copy, Clone, EnumStr)]
#[enumstr(rename_all = "lowercase")]
pub enum Kind {
    Input,
    Output,
    Program,
    Service,
}

impl Kind {
    fn thread_prefix(self) -> &'static str {
        match self {
            Kind::Input => "I",
            Kind::Output => "O",
            Kind::Program => "P",
            Kind::Service => "S",
        }
    }
}

pub enum Period {
    Interval(Duration),
    Trigger(triggered::Listener),
}

pub(crate) fn set_preparing_if_no_inputs() {
    if CONTROLLER_STATS.lock().input_threads_ready.is_empty() {
        set_status(Status::Preparing);
    }
}

pub(crate) fn set_active_if_no_inputs_and_programs() {
    let cs = CONTROLLER_STATS.lock();
    if cs.program_threads_ready.is_empty() && cs.input_threads_ready.is_empty() {
        set_status(Status::Active);
    }
}

pub fn stop_if_no_output_or_sfn() {
    if CONTROLLER_STATS.lock().output_threads_stopped.is_empty() || SHUTDOWN_FN.get().is_none() {
        set_status(Status::Stopped);
    }
}

#[allow(clippy::struct_excessive_bools)]
pub struct ControllerStats {
    input_threads_ready: BTreeMap<String, bool>,
    program_threads_ready: BTreeMap<String, bool>,
    output_threads_stopped: BTreeMap<String, bool>,
    inputs_ready: bool,
    programs_ready: bool,
    outputs_stopped: bool,
    pub(crate) thread_stats: BTreeMap<String, ThreadStats>,
}

#[derive(Default, Debug)]
pub(crate) struct ThreadStats {
    iters: u32,
    jitter: Option<JitterStats>,
}

impl ThreadStats {
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn info(&self) -> Option<ThreadInfo> {
        self.jitter.as_ref().map(|jitter| ThreadInfo {
            iters: self.iters,
            jitter_min: jitter.min,
            jitter_max: jitter.max,
            jitter_last: jitter.last,
            jitter_avg: (jitter.total / self.iters).as_u16_max(),
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ThreadInfo {
    pub iters: u32,
    pub jitter_min: u16,
    pub jitter_max: u16,
    pub jitter_last: u16,
    pub jitter_avg: u16,
}

#[derive(Default, Debug, Serialize)]
struct JitterStats {
    min: u16,
    max: u16,
    last: u16,
    total: u32,
}

impl JitterStats {
    #[inline]
    fn new(jitter: u16) -> Self {
        Self {
            min: jitter,
            max: jitter,
            last: jitter,
            total: u32::from(jitter),
        }
    }
}

impl ThreadStats {
    #[inline]
    fn report_jitter(&mut self, jitter: u16) {
        let was_reset = if self.iters == u32::MAX {
            self.iters = 1;
            true
        } else {
            self.iters += 1;
            false
        };
        if let Some(ref mut j_stats) = self.jitter {
            if j_stats.min > jitter {
                j_stats.min = jitter;
            }
            if j_stats.max < jitter {
                j_stats.max = jitter;
            }
            j_stats.last = jitter;
            let j32 = u32::from(jitter);
            if was_reset {
                j_stats.total = j32;
            } else if j_stats.total > u32::MAX - j32 {
                self.iters = 1;
                j_stats.total = j32;
            } else {
                j_stats.total += j32;
            }
        } else {
            self.jitter.replace(JitterStats::new(jitter));
        }
    }
    pub(crate) fn reset(&mut self) {
        self.iters = 0;
        self.jitter.take();
    }
}

#[inline]
pub(crate) fn report_jitter(jitter: u16) {
    if STATS_TX
        .get()
        .unwrap()
        .lock()
        .try_send((thread_name(), jitter))
        .is_err()
    {
        error!("CRITICAL: stats channel full");
    }
}

impl Default for ControllerStats {
    fn default() -> Self {
        Self {
            input_threads_ready: <_>::default(),
            program_threads_ready: <_>::default(),
            output_threads_stopped: <_>::default(),
            inputs_ready: true,
            programs_ready: true,
            outputs_stopped: true,
            thread_stats: <_>::default(),
        }
    }
}

impl ControllerStats {
    fn register_thread_stats(&mut self, name: &str) -> EResult<()> {
        if let btree_map::Entry::Vacant(v) = self.thread_stats.entry(name.to_owned()) {
            v.insert(ThreadStats::default());
            Ok(())
        } else {
            Err(Error::busy(format!(
                "thread {} is already registered",
                name
            )))
        }
    }
    fn register_input_thread(&mut self, name: &str) -> EResult<()> {
        self.register_thread_stats(name)?;
        self.input_threads_ready.insert(name.to_owned(), false);
        self.inputs_ready = false;
        Ok(())
    }
    fn register_output_thread(&mut self, name: &str) -> EResult<()> {
        self.output_threads_stopped.insert(name.to_owned(), false);
        self.outputs_stopped = false;
        self.register_thread_stats(name)
    }
    fn register_program_thread(&mut self, name: &str) -> EResult<()> {
        self.register_thread_stats(name)?;
        self.program_threads_ready.insert(name.to_owned(), false);
        self.programs_ready = false;
        Ok(())
    }
    fn register_service_thread(&mut self, name: &str) -> EResult<()> {
        self.register_thread_stats(name)
    }
    fn mark_input_thread_ready(&mut self) {
        if let Some(name) = thread::current().name() {
            if !self.inputs_ready && status() >= Status::Syncing {
                if self
                    .input_threads_ready
                    .insert(name.to_owned(), true)
                    .is_none()
                {
                    warn!("input thread {name} not registered");
                }
                for v in self.input_threads_ready.values() {
                    if !v {
                        return;
                    }
                }
                self.inputs_ready = true;
                set_status(Status::Preparing);
                if self.program_threads_ready.is_empty() {
                    set_status(Status::Active);
                }
            }
        }
    }
    fn mark_program_thread_ready(&mut self) {
        if let Some(name) = thread::current().name() {
            if !self.programs_ready && status() >= Status::Preparing {
                if self
                    .program_threads_ready
                    .insert(name.to_owned(), true)
                    .is_none()
                {
                    warn!("program thread {name} not registered");
                }
                for v in self.program_threads_ready.values() {
                    if !v {
                        return;
                    }
                }
                self.programs_ready = true;
                set_status(Status::Active);
            }
        }
    }
    fn mark_output_thread_stopped(&mut self) {
        if !self.outputs_stopped {
            let name = thread_name();
            if self
                .output_threads_stopped
                .insert(name.clone(), true)
                .is_none()
            {
                warn!("output thread {name} not registered");
            } else {
                debug!("output thread {} stopped", name);
            }
            for v in self.output_threads_stopped.values() {
                if !v {
                    return;
                }
            }
            self.outputs_stopped = true;
            set_status(Status::Stopped);
        }
    }
    pub fn current_thread_info(&self) -> Option<ThreadInfo> {
        if let Some(name) = thread::current().name() {
            self.thread_info(name)
        } else {
            None
        }
    }
    pub fn thread_info(&self, name: &str) -> Option<ThreadInfo> {
        if let Some(thread_stats) = self.thread_stats.get(name) {
            thread_stats.info()
        } else {
            None
        }
    }
}

#[inline]
fn set_status(status: Status) {
    let _lock = STATUS_MUTEX.lock();
    STATUS.store(status as i16, atomic::Ordering::Relaxed);
    info!("controller status: {}", status);
    STATUS_CHANGED.notify_all();
}

#[inline]
pub(crate) fn set_starting() {
    if status() != Status::Stopping {
        set_status(Status::Starting);
    }
}

#[inline]
pub(crate) fn set_syncing() {
    if status() != Status::Stopping {
        set_status(Status::Syncing);
    }
}

#[inline]
pub fn set_stopped() {
    set_status(Status::Stopped);
}

impl From<i16> for Status {
    fn from(s: i16) -> Status {
        match s {
            x if x == Status::Inactive as i16 => Status::Inactive,
            x if x == Status::Starting as i16 => Status::Starting,
            x if x == Status::Syncing as i16 => Status::Syncing,
            x if x == Status::Preparing as i16 => Status::Preparing,
            x if x == Status::Active as i16 => Status::Active,
            x if x == Status::Stopping as i16 => Status::Stopping,
            x if x == Status::StopSyncing as i16 => Status::StopSyncing,
            x if x == Status::Stopped as i16 => Status::Stopped,
            _ => Status::Unknown,
        }
    }
}

#[inline]
pub fn status() -> Status {
    STATUS.load(atomic::Ordering::Relaxed).into()
}

#[inline]
pub fn is_active() -> bool {
    status() == Status::Active
}

#[inline]
fn mark_input_thread_ready() {
    CONTROLLER_STATS.lock().mark_input_thread_ready();
}

#[inline]
fn mark_program_thread_ready() {
    CONTROLLER_STATS.lock().mark_program_thread_ready();
}

#[inline]
fn mark_output_thread_stopped() {
    CONTROLLER_STATS.lock().mark_output_thread_stopped();
}

#[inline]
pub(crate) fn mark_thread_ready(kind: Kind) {
    match kind {
        Kind::Input => mark_input_thread_ready(),
        Kind::Program => mark_program_thread_ready(),
        _ => {}
    }
}

#[inline]
fn can_run_inputs() -> bool {
    status() >= Status::Syncing
}

#[inline]
fn can_run_programs() -> bool {
    status() >= Status::Preparing
}

#[inline]
fn can_run_outputs() -> bool {
    let status = status();
    status >= Status::Preparing || status <= Status::Stopping
}

pub(crate) fn wait_can_run_input() {
    while !can_run_inputs() {
        let mut lock = STATUS_MUTEX.lock();
        let _ = STATUS_CHANGED.wait_for(&mut lock, WAIT_STEP);
    }
}

pub(crate) fn wait_can_run_output() {
    while !can_run_outputs() {
        let mut lock = STATUS_MUTEX.lock();
        let _ = STATUS_CHANGED.wait_for(&mut lock, WAIT_STEP);
    }
}

pub(crate) fn wait_can_run_program() {
    while !can_run_programs() {
        let mut lock = STATUS_MUTEX.lock();
        let _ = STATUS_CHANGED.wait_for(&mut lock, WAIT_STEP);
    }
}

/// On Linux alias for std::thread::spawn
#[inline]
pub fn spawn0<F, T>(f: F) -> thread::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    thread::spawn(f)
}

/// Spawns a new thread/task
///
/// If the PLC is already started, all tasks except service ones are ignored
///
/// # Panics
///
/// The function will panic if
///
/// - the thread with such name is already registered
///
/// - the thread name is more than 14 characters
///
/// - the OS is unable to spawn the thread
///
/// - the thread has invalid CPU id or priority if specified
pub fn spawn<F>(name: &str, kind: Kind, f: F)
where
    F: FnOnce() + Send + 'static,
{
    let status = status();
    if status != Status::Inactive && status != Status::Starting && kind != Kind::Service {
        error!("can not spawn {}, the PLC is already running", name);
        return;
    }
    if let Some(wait_handles) = WAIT_HANDLES.lock().as_mut() {
        assert!(
            name.len() < 15,
            "task name MUST be LESS than 15 characters ({})",
            name
        );
        let name = format!("{}{}", kind.thread_prefix(), name);
        match kind {
            Kind::Input => CONTROLLER_STATS
                .lock()
                .register_input_thread(&name)
                .unwrap(),
            Kind::Program => CONTROLLER_STATS
                .lock()
                .register_program_thread(&name)
                .unwrap(),
            Kind::Output => CONTROLLER_STATS
                .lock()
                .register_output_thread(&name)
                .unwrap(),
            Kind::Service => CONTROLLER_STATS
                .lock()
                .register_service_thread(&name)
                .unwrap(),
        }
        let var = format!("PLC_THREAD_AFFINITY_{}", name.replace('.', "__"));
        let affinity = env::var(var)
            .map(|aff| {
                aff.parse::<Affinity>()
                    .unwrap_or_else(|e| panic!("UNABLE TO SET THREAD {} AFFINITY: {}", name, e))
            })
            .ok();
        let mut builder = thread::Builder::new();
        if let Some(ss) = crate::STACK_SIZE.get() {
            builder = builder.stack_size(*ss);
        }
        let handle = builder
            .name(name)
            .spawn(move || {
                if let Some(affinity) = affinity {
                    let name = thread_name();
                    info!(
                        "setting {} affinity to CPU {}, priority: {}",
                        name, affinity.cpu_id, affinity.sched_priority
                    );
                    core_affinity::set_for_current(core_affinity::CoreId {
                        id: affinity.cpu_id,
                    });
                    let res = unsafe {
                        libc::sched_setscheduler(
                            0,
                            libc::SCHED_RR,
                            &libc::sched_param {
                                sched_priority: affinity.sched_priority,
                            },
                        )
                    };
                    assert!(
                        res == 0,
                        "UNABLE TO SET THREAD {} AFFINITY, error code: {}",
                        name,
                        res
                    );
                }
                f();
            })
            .unwrap();
        if kind == Kind::Input || kind == Kind::Program {
            wait_handles.push(handle);
        }
    }
}

pub struct Affinity {
    pub cpu_id: usize,
    pub sched_priority: libc::c_int,
}

impl FromStr for Affinity {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut sp = s.split(',');
        let cpu_id: usize = sp
            .next()
            .unwrap()
            .parse()
            .map_err(|e| Error::invalid_params(format!("invalid task cpu id: {e}")))?;
        let sched_priority: libc::c_int = sp
            .next()
            .ok_or_else(|| Error::invalid_params("no priority specified"))?
            .parse()
            .map_err(|e| Error::invalid_params(format!("invalid task priority: {e}")))?;
        if let Some(s) = sp.next() {
            return Err(Error::invalid_params(format!(
                "extra affinity params not supported: {}",
                s
            )));
        }
        if cpu_id >= cpus() {
            return Err(Error::invalid_params(format!("CPU not found: {}", cpu_id)));
        }
        if !(1..=99).contains(&sched_priority) {
            return Err(Error::invalid_params(format!(
                "invalid scheduler priority: {}",
                sched_priority
            )));
        }
        Ok(Self {
            cpu_id,
            sched_priority,
        })
    }
}

#[inline]
pub fn spawn_input_loop<F>(name: &str, interval: Duration, f: F)
where
    F: FnMut() + Send + 'static,
{
    spawn_loop(name, interval, Kind::Input, f);
}

#[inline]
pub fn spawn_output_loop<F>(name: &str, interval: Duration, f: F)
where
    F: FnMut() + Send + 'static,
{
    spawn_loop(name, interval, Kind::Output, f);
}

pub fn spawn_loop<F>(name: &str, interval: Duration, kind: Kind, mut f: F)
where
    F: FnMut() + Send + 'static,
{
    if kind == Kind::Output {
        spawn(name, Kind::Output, move || {
            let mut int = Loop::prepare_reported(interval);
            loop {
                let last_sync = output_last_sync();
                f();
                if last_sync {
                    break;
                }
                int.tick();
            }
            mark_output_thread_stopped();
            log_finished();
        });
    } else {
        spawn(name, kind, move || {
            let mut int = Loop::prepare_reported(interval);
            loop {
                log_running();
                f();
                if need_stop(kind) {
                    break;
                }
                int.tick();
            }
            log_finished();
        });
    }
}

#[inline]
fn log_running() {
    debug!("loop {} running", thread_name());
}

#[inline]
fn log_finished() {
    debug!("loop {} finished", thread_name());
}

#[inline]
fn need_stop(kind: Kind) -> bool {
    match kind {
        Kind::Input | Kind::Program => status() <= Status::Stopping,
        Kind::Output => status() <= Status::StopSyncing,
        Kind::Service => false,
    }
}

#[inline]
fn output_last_sync() -> bool {
    status() == Status::StopSyncing
}

/// # Panics
///
/// The function will panic if
///
/// - the thread with such name is already registered
///
/// - the thread name is more than 14 characters
///
/// - the OS is unable to spawn the thread
pub fn spawn_service<F>(name: &str, f: F)
where
    F: FnOnce() + Send + 'static,
{
    spawn(name, Kind::Service, f);
}

/// # Panics
///
/// The function will panic if
///
/// - the thread with such name is already registered
///
/// - the thread name is more than 14 characters
///
/// - the OS is unable to spawn the thread
pub fn spawn_program_loop<F>(name: &str, prog: F, interval: Duration)
where
    F: Fn() + Send + 'static,
{
    spawn(name, Kind::Program, move || {
        let mut int = Loop::prepare_reported(interval);
        loop {
            log_running();
            {
                if status() >= Status::Preparing {
                    prog();
                }
            }
            if need_stop(Kind::Program) {
                break;
            }
            int.tick();
        }
        log_finished();
    });
}

pub fn spawn_stats_log(int: Duration) {
    spawn_service("stlog", move || {
        let mut stats_interval = Loop::prepare0(int);
        loop {
            stats_interval.tick();
            let stats = CONTROLLER_STATS.lock();
            for (name, t_stats) in &stats.thread_stats {
                log_thread_stats(name, t_stats);
            }
        }
    });
}

fn log_thread_stats(name: &str, t_stats: &ThreadStats) {
    if let Some(info) = t_stats.info() {
        info!(
            "thread {} iters {}, jitter min: {}, max: {}, last: {}, avg: {}",
            name, info.iters, info.jitter_min, info.jitter_max, info.jitter_last, info.jitter_avg
        );
    }
}

pub(crate) fn reset_thread_stats() {
    CONTROLLER_STATS
        .lock()
        .thread_stats
        .values_mut()
        .for_each(ThreadStats::reset);
}
