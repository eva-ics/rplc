use crate::tasks;
use busrt::async_trait;
use busrt::ipc::{Client, Config};
use busrt::rpc::{Rpc, RpcClient, RpcError, RpcEvent, RpcHandlers, RpcResult};
use busrt::{Frame, QoS};
use eva_common::events::RAW_STATE_TOPIC;
use eva_common::payload::{pack, unpack};
use eva_common::value::Value;
use eva_common::{EResult, Error, OID};
use eva_sdk::controller::{format_action_topic, Action, RawStateEventPreparedOwned};
use log::{error, info, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt;
use std::str::FromStr;
use std::sync::atomic;
use std::sync::Arc;
use std::time::Duration;

eva_common::err_logger!();

#[derive(Debug)]
pub struct Params {
    pub path: String,
    pub timeout: Option<Duration>,
    pub buf_size: Option<usize>,
    pub queue_size: Option<usize>,
    pub buf_ttl: Option<Duration>,
}

impl FromStr for Params {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut sp = s.split(',');
        let path = sp.next().unwrap().to_owned();
        let mut timeout = None;
        let mut buf_size = None;
        let mut queue_size = None;
        let mut buf_ttl = None;
        for param in sp {
            let mut sp_param = param.split('=');
            let key = sp_param.next().unwrap();
            let value = sp_param
                .next()
                .ok_or_else(|| Error::invalid_params("no value"))?;
            match key {
                "timeout" => timeout = Some(Duration::from_secs_f64(value.parse()?)),
                "buf_size" => buf_size = Some(value.parse()?),
                "queue_size" => queue_size = Some(value.parse()?),
                "buf_ttl" => buf_ttl = Some(Duration::from_micros(value.parse()?)),
                v => return Err(Error::invalid_params(format!("unsupported option: {v}"))),
            }
        }
        Ok(Self {
            path,
            timeout,
            buf_size,
            queue_size,
            buf_ttl,
        })
    }
}

impl fmt::Display for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)?;
        if let Some(timeout) = self.timeout {
            write!(f, ",timeout={}", timeout.as_secs_f64())?;
        }
        if let Some(buf_size) = self.buf_size {
            write!(f, ",buf_size={}", buf_size)?;
        }
        if let Some(queue_size) = self.queue_size {
            write!(f, ",queue_size={}", queue_size)?;
        }
        if let Some(buf_ttl) = self.buf_ttl {
            write!(f, ",buf_ttl={}", buf_ttl.as_micros())?;
        }
        Ok(())
    }
}

enum PublishPayload {
    Single(String, Vec<u8>),
    Bulk(Vec<(String, Vec<u8>)>),
}

const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const EVENT_CHANNEL_SIZE: usize = 1_000;

const ERR_NOT_REGISTERED: &str = "BUS/RT EAPI not registered";

pub type ActionHandlerFn = Box<dyn Fn(&mut Action) -> EResult<()> + Send + Sync + 'static>;

static PUBLISHER_TX: Lazy<Mutex<Option<async_channel::Sender<PublishPayload>>>> =
    Lazy::new(<_>::default);
static ACTION_HANDLERS: Lazy<Mutex<BTreeMap<OID, ActionHandlerFn>>> = Lazy::new(<_>::default);
static CONNECTED: atomic::AtomicBool = atomic::AtomicBool::new(false);

/// # Panics
///
/// Will panic if action handler is already registered
pub fn append_action_handler<F>(oid: OID, f: F)
where
    F: Fn(&mut Action) -> EResult<()> + Send + Sync + 'static,
{
    let oid_c = oid.clone();
    assert!(
        ACTION_HANDLERS.lock().insert(oid, Box::new(f)).is_none(),
        "Action handler for {} is already registered",
        oid_c
    );
}

/// # Panics
///
/// Will panic if action handler is already registered
pub fn append_action_handlers_bulk(oids: &[OID], f: Vec<ActionHandlerFn>) {
    let mut action_handlers = ACTION_HANDLERS.lock();
    for (oid, f) in oids.iter().zip(f) {
        assert!(
            action_handlers.insert(oid.clone(), f).is_none(),
            "Action handler for {} is already registered",
            oid
        );
    }
}

struct Handlers {}

#[async_trait]
impl RpcHandlers for Handlers {
    async fn handle_call(&self, event: RpcEvent) -> RpcResult {
        // keep all methods minimalistic
        let payload = event.payload();
        match event.parse_method()? {
            "test" => {
                if payload.is_empty() {
                    Ok(None)
                } else {
                    Err(RpcError::params(None))
                }
            }
            "info" => {
                if payload.is_empty() {
                    Ok(Some(pack(&crate::plc_info())?))
                } else {
                    Err(RpcError::params(None))
                }
            }
            "thread_stats.get" => {
                if payload.is_empty() {
                    let mut result = BTreeMap::new();
                    let thread_stats = &tasks::controller_stats().lock().thread_stats;
                    for (name, st) in thread_stats {
                        result.insert(name, st.info());
                    }
                    Ok(Some(pack(&result)?))
                } else {
                    Err(RpcError::params(None))
                }
            }
            "thread_stats.reset" => {
                if payload.is_empty() {
                    tasks::reset_thread_stats();
                    Ok(None)
                } else {
                    Err(RpcError::params(None))
                }
            }
            "action" => {
                if payload.is_empty() {
                    return Err(RpcError::params(None));
                }
                let mut action: Action = unpack(payload)?;
                tokio::task::spawn_blocking(move || {
                    let topic = format_action_topic(action.oid());
                    let payload = if let Err(e) = handle_action(&mut action, &topic) {
                        action.event_failed(1, None, Some(Value::String(e.to_string())))
                    } else {
                        action.event_completed(None)
                    };
                    match pack(&payload) {
                        Ok(packed) => {
                            if let Some(tx) = PUBLISHER_TX.lock().as_ref() {
                                tx.send_blocking(PublishPayload::Single(topic, packed))
                                    .log_ef();
                            } else {
                                warn!("action response orphaned, BUS/RT EAPI not registered");
                            }
                        }
                        Err(e) => error!("action payload pack failed: {}", e),
                    }
                })
                .await
                .map_err(Error::failed)?;
                Ok(None)
            }
            _ => Err(RpcError::method(None)),
        }
    }
    async fn handle_notification(&self, _event: RpcEvent) {}
    async fn handle_frame(&self, _frame: Frame) {}
}

fn handle_action(action: &mut Action, action_topic: &str) -> EResult<()> {
    let oid = action.oid().clone();
    if let Some(handler) = ACTION_HANDLERS.lock().get(&oid) {
        if let Some(tx) = PUBLISHER_TX.lock().as_ref() {
            let packed = pack(&action.event_running())?;
            tx.send_blocking(PublishPayload::Single(action_topic.to_owned(), packed))
                .map_err(Error::failed)?;
        } else {
            return Err(Error::failed(ERR_NOT_REGISTERED));
        }
        handler(action)
    } else {
        Err(Error::not_found(format!(
            "BUS/RT EAPI action handler for {} not registered",
            oid
        )))
    }
}

pub(crate) fn launch(action_pool_size: usize) {
    info!("preparing BUS/RT EAPI connection");
    if let Ok(eapi_s) = env::var("PLC_EAPI") {
        match eapi_s.parse::<Params>() {
            Ok(eapi_params) => {
                let timeout = eapi_params.timeout.unwrap_or(busrt::DEFAULT_TIMEOUT);
                info!("eapi.path = {}", eapi_params.path);
                info!("eapi.timeout = {:?}", eapi_params.timeout);
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(action_pool_size)
                    .thread_name("Sbusrt.scada")
                    .build()
                    .unwrap();
                tasks::spawn_service("busrt.scada", move || {
                    rt.block_on(bus(&eapi_params));
                });
                let op = eva_common::op::Op::new(timeout);
                loop {
                    tasks::step_sleep();
                    if CONNECTED.load(atomic::Ordering::Relaxed) || op.is_timed_out() {
                        break;
                    }
                }
            }
            Err(e) => error!("unable to parse PLC_EAPI: {e}"),
        };
    } else {
        warn!("no PLC_EAPI specified, aborting EAPI connection");
    }
}

async fn bus(params: &Params) {
    let name = format!(
        "fieldbus.{}.plc.{}",
        crate::HOSTNAME.get().unwrap(),
        crate::NAME.get().unwrap()
    );
    let mut config = Config::new(&params.path, &name);
    if let Some(timeout) = params.timeout {
        config = config.timeout(timeout);
    }
    if let Some(buf_size) = params.buf_size {
        info!("eapi.buf_size = {buf_size}");
        config = config.buf_size(buf_size);
    }
    if let Some(queue_size) = params.queue_size {
        info!("eapi.queue_size = {queue_size}");
        config = config.queue_size(queue_size);
    }
    if let Some(buf_ttl) = params.buf_ttl {
        info!("eapi.buf_ttl = {:?}", buf_ttl);
        config = config.buf_ttl(buf_ttl);
    }
    loop {
        if let Err(e) = run(&config).await {
            error!("BUS/RT EAPI error: {}", e);
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    }
}

async fn run(config: &Config) -> EResult<()> {
    let client = Client::connect(config).await?;
    let rpc = Arc::new(RpcClient::new(client, Handlers {}));
    info!("BUS/RT EAPI connected");
    let (tx, rx) = async_channel::bounded::<PublishPayload>(EVENT_CHANNEL_SIZE);
    let rpc_c = rpc.clone();
    let publisher_worker = tokio::spawn(async move {
        while let Ok(payload) = rx.recv().await {
            let cl = rpc_c.client();
            let mut client = cl.lock().await;
            match payload {
                PublishPayload::Single(topic, value) => {
                    client.publish(&topic, value.into(), QoS::No).await.log_ef();
                }
                PublishPayload::Bulk(values) => {
                    for (topic, value) in values {
                        if client
                            .publish(&topic, value.into(), QoS::No)
                            .await
                            .log_err()
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }
    });
    PUBLISHER_TX.lock().replace(tx.clone());
    CONNECTED.store(true, atomic::Ordering::Relaxed);
    while rpc.client().lock().await.is_connected() {
        tokio::time::sleep(crate::tasks::SLEEP_STEP).await;
    }
    publisher_worker.abort();
    PUBLISHER_TX.lock().take();
    warn!("BUS/RT EAPI disconnected");
    tokio::time::sleep(RECONNECT_DELAY).await;
    Ok(())
}

pub fn notify<S: ::std::hash::BuildHasher>(
    map: HashMap<&OID, RawStateEventPreparedOwned, S>,
) -> EResult<()> {
    if map.is_empty() {
        return Ok(());
    }
    let mut data = Vec::with_capacity(map.len());
    for (oid, event) in map {
        data.push((
            format!("{}{}", RAW_STATE_TOPIC, oid.as_path()),
            pack(event.state())?,
        ));
    }
    if let Some(tx) = PUBLISHER_TX.lock().as_ref() {
        tx.send_blocking(PublishPayload::Bulk(data))
            .map_err(Error::failed)?;
        Ok(())
    } else {
        Err(Error::failed(ERR_NOT_REGISTERED))
    }
}
