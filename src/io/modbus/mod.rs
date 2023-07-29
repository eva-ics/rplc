use crate::tasks;
use eva_common::value::Value;
use serde::Deserialize;
use std::error::Error;
use std::fmt::Write as _;
use std::net::SocketAddr;
pub use types::{Coils, Registers, SwapModbusEndianess};

mod regs;
mod types;

const DEFAULT_FRAME_DELAY: f64 = 0.1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputConfig {
    #[serde(flatten)]
    reg: regs::Reg,
    unit: u8,
    #[serde(default)]
    map: Vec<RegMapInput>,
    #[serde(deserialize_with = "crate::interval::deserialize_interval_as_nanos")]
    sync: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputConfig {
    #[serde(flatten)]
    reg: regs::Reg,
    unit: u8,
    #[serde(default)]
    map: Vec<RegMapOutput>,
    #[serde(deserialize_with = "crate::interval::deserialize_interval_as_nanos")]
    sync: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegMapInput {
    #[serde(default, flatten)]
    offset: regs::MapOffset,
    target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegMapOutput {
    #[serde(default, flatten)]
    offset: regs::MapOffset,
    source: String,
}

fn default_timeout() -> f64 {
    1.0
}

fn default_frame_delay() -> f64 {
    DEFAULT_FRAME_DELAY
}

#[derive(Deserialize, Copy, Clone, Debug)]
#[serde(rename_all = "lowercase")]
enum Proto {
    Tcp,
    Udp,
    Rtu,
    Ascii,
}

impl Proto {
    fn as_rmodbus_proto_str(self) -> &'static str {
        match self {
            Proto::Ascii => "Ascii",
            Proto::Tcp | Proto::Udp => "TcpUdp",
            Proto::Rtu => "Rtu",
        }
    }
    fn generate_datasync(
        self,
        path: &str,
        timeout: f64,
        frame_delay: f64,
    ) -> Result<String, Box<dyn Error>> {
        let comm = match self {
            Proto::Tcp => {
                path.parse::<SocketAddr>()?;
                format!(
                    r#"::rplc::comm::tcp::TcpComm::create("{}", ::std::time::Duration::from_secs_f64({:.6})).unwrap()"#,
                    path, timeout
                )
            }
            Proto::Rtu => {
                crate::comm::serial::check_path(path);
                let mut res = format!(r#"::rplc::comm::serial::SerialComm::create("{}", "#, path);
                write!(res, "::std::time::Duration::from_secs_f64({:.6}),", timeout)?;
                write!(
                    res,
                    "::std::time::Duration::from_secs_f64({:.6}),",
                    frame_delay
                )?;
                write!(res, ").unwrap()")?;
                res
            }
            _ => unimplemented!(),
        };
        Ok(comm)
    }
}

fn push_launcher(kind: tasks::Kind, sync: u64, num: usize, id: &str, f: &mut codegen::Function) {
    f.line("let comm_c = comm.clone();");
    let mut spawn_block = codegen::Block::new(&format!(
        "::rplc::tasks::spawn_{}_loop(\"{}_{}\", ::std::time::Duration::from_nanos({}), move ||",
        kind, id, num, sync,
    ));
    spawn_block.line(&format!("{kind}_{id}_{num}(&comm_c);"));
    spawn_block.after(");");
    f.push_block(spawn_block);
}

#[derive(Deserialize)]
struct Config {
    path: String,
    proto: Proto,
    #[serde(default = "default_timeout")]
    timeout: f64,
    #[serde(default = "default_frame_delay")]
    frame_delay: f64,
}

fn push_input_worker(
    num: usize,
    id: &str,
    config: InputConfig,
    proto: Proto,
    scope: &mut codegen::Scope,
) {
    let f_input = scope.new_fn(&format!("input_{id}_{num}"));
    f_input.arg("comm", "&::rplc::comm::Communicator");
    let mut loop_block =
        codegen::Block::new(&format!("if let Err(e) = input_{id}_{num}_worker(comm)"));
    loop_block.line("::rplc::export::log::error!(\"{}: {}\", ::rplc::tasks::thread_name(), e);");
    f_input.push_block(loop_block);
    let f_input_worker = scope.new_fn(&format!("input_{id}_{num}_worker"));
    f_input_worker.arg("comm", "&::rplc::comm::Communicator");
    f_input_worker.ret("Result<(), Box<dyn ::std::error::Error>>");
    f_input_worker
        .line("use ::rplc::export::rmodbus::{self, client::ModbusRequest, guess_response_frame_len, ModbusProto};");
    f_input_worker.line(format!(
        "use ::rplc::io::modbus::{};",
        config.reg.kind().as_helper_type_str()
    ));
    let mut req_block = codegen::Block::new("let (mreq, response) =");
    req_block.after(";");
    req_block.line(format!(
        "let mut mreq = ModbusRequest::new({}, ModbusProto::{});",
        config.unit,
        proto.as_rmodbus_proto_str()
    ));
    req_block.line("let mut request = Vec::new();");
    req_block.line(format!(
        "mreq.generate_get_{}s({}, {}, &mut request)?;",
        config.reg.kind(),
        config.reg.offset(),
        config.reg.number()
    ));
    req_block.line("let _lock = comm.lock();");
    req_block.line("comm.write(&request)?;");
    req_block.line("let mut buf = [0u8; 6];");
    req_block.line("comm.read_exact(&mut buf)?;");
    req_block.line("let mut response = buf.to_vec();");
    req_block.line(format!(
        "let len = guess_response_frame_len(&buf, ModbusProto::{})?;",
        proto.as_rmodbus_proto_str()
    ));
    let mut lf_block = codegen::Block::new("if len > 6");
    lf_block.line("let mut rest = vec![0u8; (len - 6) as usize];");
    lf_block.line("comm.read_exact(&mut rest)?;");
    lf_block.line("response.extend(rest);");
    req_block.push_block(lf_block);
    req_block.line("(mreq, response)");
    f_input_worker.push_block(req_block);
    f_input_worker.line("let mut data = Vec::new();");
    match config.reg.kind() {
        regs::Kind::Coil | regs::Kind::Discrete => {
            let mut r_block =
                codegen::Block::new("if let Err(e) = mreq.parse_bool(&response, &mut data)");
            let mut r_unknown = codegen::Block::new("if e == rmodbus::ErrorKind::UnknownError");
            r_unknown.line("comm.reconnect();");
            r_block.push_block(r_unknown);
            r_block.line("return Err(e.into());");
            f_input_worker.push_block(r_block);
            f_input_worker.line("let regs = Coils(data);");
        }
        regs::Kind::Input | regs::Kind::Holding => {
            let mut r_block =
                codegen::Block::new("if let Err(e) = mreq.parse_u16(&response, &mut data)");
            let mut r_unknown = codegen::Block::new("if e == rmodbus::ErrorKind::UnknownError");
            r_unknown.line("comm.reconnect();");
            r_block.push_block(r_unknown);
            r_block.line("return Err(e.into());");
            f_input_worker.push_block(r_block);
            f_input_worker.line("let regs = Registers(data);");
        }
    }
    if !config.map.is_empty() {
        let mut cp_block = codegen::Block::new("");
        cp_block.line("let mut ctx = CONTEXT.write();");
        for i in config.map {
            cp_block.line(format!("// {}", i.target));
            let mut cp_block_try_into = codegen::Block::new("match slice.try_into()");
            cp_block_try_into.line(format!("Ok(v) => ctx.{} = v,", i.target));
            cp_block_try_into.line(format!(
                "Err(e) => ::rplc::export::log::error!(\"modbus ctx.{} set err: {{}}\", e)",
                i.target
            ));
            let mut cp_block_match_slice_at =
                codegen::Block::new(&format!("match regs.slice_at({})", i.offset.offset()));
            cp_block_match_slice_at.line("Ok(slice) => ");
            cp_block_match_slice_at.push_block(cp_block_try_into);
            cp_block_match_slice_at.line(format!(
                "Err(e) => ::rplc::export::log::error!(\"modbus slice err ctx.{}: {{}}\", e)",
                i.target
            ));
            cp_block.push_block(cp_block_match_slice_at);
        }
        f_input_worker.push_block(cp_block);
    }
    f_input_worker.line("Ok(())");
}

fn push_output_worker(
    num: usize,
    id: &str,
    config: OutputConfig,
    proto: Proto,
    scope: &mut codegen::Scope,
) {
    let f_output = scope.new_fn(&format!("output_{id}_{num}"));
    f_output.arg("comm", "&::rplc::comm::Communicator");
    let mut loop_block =
        codegen::Block::new(&format!("if let Err(e) = output_{id}_{num}_worker(comm)"));
    loop_block.line("::rplc::export::log::error!(\"{}: {}\", ::rplc::tasks::thread_name(), e);");
    f_output.push_block(loop_block);
    let f_output_worker = scope.new_fn(&format!("output_{id}_{num}_worker"));
    f_output_worker.arg("comm", "&::rplc::comm::Communicator");
    f_output_worker.ret("Result<(), Box<dyn ::std::error::Error>>");
    f_output_worker
        .line("use ::rplc::export::rmodbus::{self, client::ModbusRequest, guess_response_frame_len, ModbusProto};");
    f_output_worker.line(format!(
        "use ::rplc::io::modbus::{};",
        config.reg.kind().as_helper_type_str()
    ));
    f_output_worker.line(format!(
        "let mut data: Vec<{}> = vec![{}; {}];",
        config.reg.kind().as_type_str(),
        config.reg.kind().as_type_default_value_str(),
        config.reg.number()
    ));
    let mut cp_block = codegen::Block::new("");
    cp_block.line("let ctx = CONTEXT.read();");
    for m in config.map {
        cp_block.line(format!("// {}", m.source));
        cp_block.line(format!("let offset = {};", m.offset.offset()));
        cp_block.line(format!(
            "let payload = {}::from(&ctx.{});",
            config.reg.kind().as_helper_type_str(),
            m.source
        ));
        let mut iter_block = codegen::Block::new("for (i, p) in payload.0.into_iter().enumerate()");
        iter_block
            .line("*data.get_mut(i+offset).ok_or(::rplc::export::rmodbus::ErrorKind::OOB)? = p;");
        cp_block.push_block(iter_block);
    }
    f_output_worker.push_block(cp_block);
    f_output_worker.line("let mut request = Vec::new();");
    f_output_worker.line(format!(
        "let mut mreq = ModbusRequest::new({}, ModbusProto::{});",
        config.unit,
        proto.as_rmodbus_proto_str()
    ));
    f_output_worker.line(format!(
        "mreq.generate_set_{}s_bulk({}, &data, &mut request)?;",
        config.reg.kind(),
        config.reg.offset(),
    ));
    let mut resp_block = codegen::Block::new("let response =");
    resp_block.line("let _lock = comm.lock();");
    resp_block.line("comm.write(&request)?;");
    resp_block.line("let mut buf = [0u8; 6];");
    resp_block.line("comm.read_exact(&mut buf)?;");
    resp_block.line("let mut response = buf.to_vec();");
    resp_block.line(format!(
        "let len = guess_response_frame_len(&buf, ModbusProto::{})?;",
        proto.as_rmodbus_proto_str()
    ));
    let mut lf_block = codegen::Block::new("if len > 6");
    lf_block.line("let mut rest = vec![0u8; (len - 6) as usize];");
    lf_block.line("comm.read_exact(&mut rest)?;");
    lf_block.line("response.extend(rest);");
    resp_block.push_block(lf_block);
    resp_block.line("response");
    resp_block.after(";");
    f_output_worker.push_block(resp_block);
    let mut r_block = codegen::Block::new("if let Err(e) = mreq.parse_ok(&response)");
    let mut r_unknown = codegen::Block::new("if e == rmodbus::ErrorKind::UnknownError");
    r_unknown.line("comm.reconnect();");
    r_block.push_block(r_unknown);
    r_block.line("return Err(e.into());");
    f_output_worker.push_block(r_block);
    f_output_worker.line("Ok(())");
}

pub(crate) fn generate_io(
    id: &str,
    cfg: &Value,
    inputs: &[Value],
    outputs: &[Value],
) -> Result<codegen::Scope, Box<dyn Error>> {
    let id = id.to_lowercase();
    let mut scope = codegen::Scope::new();
    let config = Config::deserialize(cfg.clone())?;
    let mut launch_fn = codegen::Function::new(&format!("launch_datasync_{id}"));
    launch_fn.allow("clippy::redundant_clone, clippy::unreadable_literal");
    launch_fn.line(format!(
        "let comm_obj = {};",
        config
            .proto
            .generate_datasync(&config.path, config.timeout, config.frame_delay)?
    ));
    if !inputs.is_empty() || !outputs.is_empty() {
        launch_fn.line("let comm: ::rplc::comm::Communicator = ::std::sync::Arc::new(comm_obj);");
    }
    for (i, input) in inputs.iter().enumerate() {
        let mut input_config = InputConfig::deserialize(input.clone())?;
        for m in &mut input_config.map {
            m.offset.normalize(input_config.reg.offset())?;
        }
        push_launcher(
            tasks::Kind::Input,
            input_config.sync,
            i + 1,
            &id,
            &mut launch_fn,
        );
        push_input_worker(i + 1, &id, input_config, config.proto, &mut scope);
    }
    for (i, output) in outputs.iter().enumerate() {
        let mut output_config = OutputConfig::deserialize(output.clone())?;
        for m in &mut output_config.map {
            m.offset.normalize(output_config.reg.offset())?;
        }
        push_launcher(
            tasks::Kind::Output,
            output_config.sync,
            i + 1,
            &id,
            &mut launch_fn,
        );
        push_output_worker(i + 1, &id, output_config, config.proto, &mut scope);
    }
    scope.push_fn(launch_fn);
    Ok(scope)
}
