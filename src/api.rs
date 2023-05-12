use crate::tasks;
use eva_common::payload::{pack, unpack};
use eva_common::value::{to_value, Value};
use eva_common::{EResult, Error};
use log::error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::io::{Read, Write};
use std::os::unix;
use std::path::PathBuf;

const JSON_RPC: &str = "2.0";
const MAX_API_CONN: usize = 10;

#[derive(Serialize, Deserialize)]
pub struct Request {
    jsonrpc: String,
    method: String,
    params: Option<Value>,
}

impl Request {
    pub fn new(method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSON_RPC.to_owned(),
            method: method.to_owned(),
            params,
        }
    }
    fn check(&self) -> EResult<()> {
        if self.jsonrpc == JSON_RPC {
            Ok(())
        } else {
            Err(Error::unsupported("unsupported json rpc version"))
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Response {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ResponseError>,
}

impl Response {
    #[inline]
    fn err(e: Error) -> Self {
        Self {
            jsonrpc: JSON_RPC.to_owned(),
            result: None,
            error: Some(ResponseError {
                code: e.kind() as i16,
                message: e.message().map(ToOwned::to_owned),
            }),
        }
    }
    #[inline]
    fn result(val: Value) -> Self {
        Self {
            jsonrpc: JSON_RPC.to_owned(),
            result: Some(val),
            error: None,
        }
    }
    pub fn check(&self) -> EResult<()> {
        if self.jsonrpc != JSON_RPC {
            return Err(Error::unsupported("unsupported json rpc version"));
        }
        if let Some(ref err) = self.error {
            return Err(Error::newc(
                eva_common::ErrorKind::from(err.code),
                err.message.as_deref(),
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct ResponseError {
    code: i16,
    message: Option<String>,
}

impl From<EResult<Value>> for Response {
    fn from(r: EResult<Value>) -> Self {
        match r {
            Ok(v) => Response::result(v),
            Err(e) => Response::err(e),
        }
    }
}

pub(crate) fn spawn_api() -> PathBuf {
    let mut socket_path = crate::var_dir();
    socket_path.push(format!("{}.plcsock", crate::name()));
    let _ = fs::remove_file(&socket_path);
    let listener = unix::net::UnixListener::bind(&socket_path).unwrap();
    tasks::spawn_service("api", move || {
        let pool = threadpool::ThreadPool::new(MAX_API_CONN);
        for sr in listener.incoming() {
            match sr {
                Ok(stream) => {
                    pool.execute(move || {
                        if let Err(e) = handle_api_stream(stream) {
                            error!("API {}", e);
                        }
                    });
                }
                Err(e) => error!("API {}", e),
            }
        }
    });
    socket_path
}

fn handle_api_stream(mut stream: unix::net::UnixStream) -> Result<(), Error> {
    stream.set_read_timeout(Some(crate::DEFAULT_TIMEOUT))?;
    stream.set_write_timeout(Some(crate::DEFAULT_TIMEOUT))?;
    loop {
        let mut buf: [u8; 5] = [0; 5];
        match stream.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => {
                return Err(e.into());
            }
        }
        if buf[0] != 0 {
            return Err(Error::invalid_data("invalid header"));
        }
        let mut buf = vec![0; usize::try_from(u32::from_le_bytes(buf[1..].try_into()?))?];
        stream.read_exact(&mut buf)?;
        let req: Request = unpack(&buf)?;
        req.check()?;
        let response: Response = handle_api_call(&req.method, req.params).into();
        let packed = pack(&response)?;
        let mut buf = Vec::with_capacity(packed.len() + 5);
        buf.push(0u8);
        buf.extend(u32::try_from(packed.len())?.to_le_bytes());
        buf.extend(packed);
        stream.write_all(&buf)?;
    }
    Ok(())
}

fn handle_api_call(method: &str, params: Option<Value>) -> Result<Value, Error> {
    macro_rules! ok {
        () => {
            Ok(Value::Unit)
        };
    }
    macro_rules! invalid_params {
        () => {
            Err(Error::invalid_params("invalid method parameters"))
        };
    }
    match method {
        "test" => {
            if params.is_none() {
                ok!()
            } else {
                invalid_params!()
            }
        }
        "info" => {
            if params.is_none() {
                to_value(crate::plc_info()).map_err(Into::into)
            } else {
                invalid_params!()
            }
        }
        "thread_stats.get" => {
            if params.is_none() {
                let mut result = BTreeMap::new();
                let thread_stats = &tasks::controller_stats().lock().thread_stats;
                for (name, st) in thread_stats {
                    result.insert(name, st.info());
                }
                to_value(result).map_err(Into::into)
            } else {
                invalid_params!()
            }
        }
        "thread_stats.reset" => {
            if params.is_none() {
                tasks::reset_thread_stats();
                ok!()
            } else {
                invalid_params!()
            }
        }
        v => Err(Error::not_implemented(v)),
    }
}
