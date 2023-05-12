use eva_common::value::Value;
use eva_common::{EResult, Error};
use log::{error, info, warn};
use serde::Deserialize;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::net::TcpListener;
use std::time::Duration;

#[derive(Deserialize, Debug, bmart_derive::EnumStr)]
#[enumstr(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
enum Proto {
    Tcp,
    Rtu,
}

use rmodbus::{
    server::{context::ModbusContext, ModbusFrame},
    ModbusFrameBuf, ModbusProto,
};

pub trait SlaveContext<const C: usize, const D: usize, const I: usize, const H: usize> {
    fn modbus_context(&self) -> &ModbusContext<C, D, I, H>;
    fn modbus_context_mut(&mut self) -> &mut ModbusContext<C, D, I, H>;
}

fn default_maxconn() -> usize {
    5
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct ServerConfig {
    proto: Proto,
    listen: String,
    unit: u8,
    timeout: f64,
    #[serde(default = "default_maxconn")]
    maxconn: usize,
}

pub(crate) fn generate_server_launcher(
    id: usize,
    config: &Value,
    modbus_context_config: &crate::builder::config::ModbusConfig,
) -> EResult<codegen::Block> {
    let config = ServerConfig::deserialize(config.clone())?;
    let name = format!("srv{}_modbus", id);
    let mut launch_block =
        codegen::Block::new(&format!("::rplc::tasks::spawn_service(\"{name}\", move ||"));
    launch_block.line("#[allow(clippy::unreadable_literal)]");
    config.listen.parse::<SocketAddr>().map_err(|e| {
        Error::invalid_params(format!("invalid modbus server listen address: {}", e))
    })?;
    let mut launch_loop = codegen::Block::new("loop");
    let mut launch_str = format!(
        "::rplc::server::modbus::{}_server::<Context, {}>(",
        config.proto,
        modbus_context_config.as_const_generics()
    );
    write!(
        launch_str,
        "{}, \"{}\", &CONTEXT, ::std::time::Duration::from_secs_f64({:.6}), {})",
        config.unit, config.listen, config.timeout, config.maxconn
    )?;
    let mut launch_if = codegen::Block::new(&format!("if let Err(e) = {}", launch_str));
    launch_if.line(format!(
        "::rplc::export::log::error!(\"modbus server {} {} error: {{e}}\");",
        config.proto, config.listen
    ));
    launch_loop.push_block(launch_if);
    launch_loop.line("::rplc::tasks::step_sleep_err();");
    launch_block.push_block(launch_loop);
    launch_block.after(");");
    Ok(launch_block)
}

pub fn handle_tcp_stream<X, const C: usize, const D: usize, const I: usize, const H: usize>(
    stream: Result<std::net::TcpStream, std::io::Error>,
    ctx: &'static crate::LockedContext<X>,
    unit: u8,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>>
where
    X: SlaveContext<C, D, I, H>,
{
    let mut stream = stream?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    loop {
        let mut buf: ModbusFrameBuf = [0; 256];
        let mut response = Vec::new(); // for nostd use FixedVec with alloc [u8;256]
        if stream.read(&mut buf).unwrap_or(0) == 0 {
            break;
        }
        let mut frame = ModbusFrame::new(unit, &buf, ModbusProto::TcpUdp, &mut response);
        frame.parse()?;
        if frame.processing_required {
            if frame.readonly {
                frame.process_read(ctx.read().modbus_context())?;
            } else {
                frame.process_write(ctx.write().modbus_context_mut())?;
            };
        }
        if frame.response_required {
            frame.finalize_response()?;
            if stream.write(response.as_slice()).is_err() {
                break;
            }
        }
    }
    Ok(())
}

pub fn tcp_server<X, const C: usize, const D: usize, const I: usize, const H: usize>(
    unit: u8,
    listen: &str,
    ctx: &'static crate::LockedContext<X>,
    timeout: Duration,
    maxconn: usize,
) -> Result<(), Box<dyn std::error::Error>>
where
    X: SlaveContext<C, D, I, H> + Send + Sync + 'static,
{
    let listener = TcpListener::bind(listen)?;
    let pool = threadpool::ThreadPool::new(maxconn);
    info!("modbus listener started at: {listen}");
    for stream in listener.incoming() {
        pool.execute(move || {
            if let Err(e) = handle_tcp_stream(stream, ctx, unit, timeout) {
                error!("modbus server error: {}", e);
            }
        });
    }
    Ok(())
}

/// # Panics
///
/// Will panic on misconfigured listen string
pub fn rtu_server<X, const C: usize, const D: usize, const I: usize, const H: usize>(
    unit: u8,
    listen: &str,
    ctx: &'static crate::LockedContext<X>,
    timeout: Duration,
    _maxconn: usize,
) -> Result<(), Box<dyn std::error::Error>>
where
    X: SlaveContext<C, D, I, H> + Send + Sync + 'static,
{
    let mut port = crate::comm::serial::open(listen, timeout)?;
    info!("modbus listener started at: {listen}");
    loop {
        let mut buf: ModbusFrameBuf = [0; 256];
        if port.read(&mut buf)? > 0 {
            let mut response = Vec::new();
            let mut frame = ModbusFrame::new(unit, &buf, ModbusProto::Rtu, &mut response);
            if frame.parse().is_err() {
                warn!("broken frame received on {}", listen);
                continue;
            }
            if frame.processing_required {
                let result = if frame.readonly {
                    frame.process_read(ctx.read().modbus_context())
                } else {
                    frame.process_write(ctx.write().modbus_context_mut())
                };
                match result {
                    Ok(()) => {}
                    Err(e) => {
                        warn!("frame processing error on {}: {}", listen, e);
                        continue;
                    }
                }
            }
            if frame.response_required {
                frame.finalize_response()?;
                println!("{:x?}", response);
                port.write_all(&response)?;
            }
        }
    }
}
