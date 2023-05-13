use crate::io::Kind;
use eva_common::value::Value;
use indexmap::IndexMap;
use inflector::Inflector;
use serde::Deserialize;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Config {
    version: u16,
    #[serde(default)]
    pub(crate) core: CoreConfig,
    #[serde(default)]
    context: ContextConfig,
    #[cfg(feature = "eva")]
    #[serde(default)]
    pub(crate) eapi: EapiConfig,
    #[serde(default)]
    io: Vec<Io>,
    #[serde(default)]
    server: Vec<ServerConfig>,
}

fn default_stop_timeout() -> f64 {
    crate::DEFAULT_STOP_TIMEOUT
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoreConfig {
    #[serde(default = "default_stop_timeout")]
    pub(crate) stop_timeout: f64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            stop_timeout: default_stop_timeout(),
        }
    }
}

#[cfg(feature = "eva")]
#[inline]
fn default_eapi_action_pool_size() -> usize {
    1
}

#[cfg(feature = "eva")]
#[derive(Deserialize, Debug)]
pub(crate) struct EapiConfig {
    #[serde(default = "default_eapi_action_pool_size")]
    pub(crate) action_pool_size: usize,
}

#[cfg(feature = "eva")]
impl Default for EapiConfig {
    fn default() -> Self {
        Self {
            action_pool_size: default_eapi_action_pool_size(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct ServerConfig {
    kind: crate::server::Kind,
    #[allow(dead_code)]
    config: Value,
}

#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
struct ContextConfig {
    #[serde(default)]
    serialize: bool,
    #[cfg(feature = "modbus")]
    #[serde(default)]
    modbus: Option<ModbusConfig>,
    #[serde(default)]
    fields: IndexMap<String, ContextField>,
}

#[cfg(feature = "modbus")]
#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModbusConfig {
    #[serde(default)]
    pub(crate) c: usize,
    #[serde(default)]
    pub(crate) d: usize,
    #[serde(default)]
    pub(crate) i: usize,
    #[serde(default)]
    pub(crate) h: usize,
}

#[cfg(feature = "modbus")]
impl ModbusConfig {
    pub(crate) fn as_const_generics(&self) -> String {
        format!("{}, {}, {}, {}", self.c, self.d, self.i, self.h)
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Io {
    id: String,
    kind: Kind,
    #[allow(dead_code)]
    #[serde(default)]
    config: Value,
    #[allow(dead_code)]
    #[serde(default)]
    input: Vec<Value>,
    #[allow(dead_code)]
    #[serde(default)]
    output: Vec<Value>,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum ContextField {
    Map(IndexMap<String, ContextField>),
    Type(String),
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P, context: &tera::Context) -> Result<Self, Box<dyn Error>> {
        let config_tpl = fs::read_to_string(path)?;
        let config: Config =
            serde_yaml::from_str(&tera::Tera::default().render_str(&config_tpl, context)?)?;
        if config.version != 1 {
            unimplemented!("config version {} is not supported", config.version);
        }
        Ok(config)
    }
    #[allow(unreachable_code)]
    pub fn generate_io<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>> {
        let mut m = codegen::Scope::new();
        m.raw(crate::builder::AUTO_GENERATED);
        m.raw("#[allow(unused_imports)]");
        m.raw("use crate::plc::context::{Context, CONTEXT};");
        #[allow(unused_mut)]
        let mut funcs: Vec<String> = Vec::new();
        //let mut output_required: bool = false;
        for i in &self.io {
            match i.kind {
                #[cfg(feature = "modbus")]
                Kind::Modbus => {
                    m.raw(
                        crate::io::modbus::generate_io(&i.id, &i.config, &i.input, &i.output)?
                            .to_string(),
                    );
                }
                #[cfg(feature = "opcua")]
                Kind::OpcUa => {
                    m.raw(
                        crate::io::opcua::generate_io(&i.id, &i.config, &i.input, &i.output)?
                            .to_string(),
                    );
                }
                #[cfg(feature = "eva")]
                Kind::Eapi => {
                    m.raw(
                        crate::io::eapi::generate_io(&i.id, &i.config, &i.input, &i.output)?
                            .to_string(),
                    );
                }
            }
            funcs.push(format!("launch_datasync_{}", i.id.to_lowercase()));
        }
        let f_launch_datasync = m.new_fn("launch_datasync").vis("pub");
        for function in funcs {
            f_launch_datasync.line(format!("{}();", function));
        }
        #[allow(unused_variables)]
        for (i, serv) in self.server.iter().enumerate() {
            match serv.kind {
                #[cfg(feature = "modbus")]
                crate::server::Kind::Modbus => {
                    f_launch_datasync.push_block(crate::server::modbus::generate_server_launcher(
                        i + 1,
                        &serv.config,
                        self.context
                            .modbus
                            .as_ref()
                            .expect("modbus not specified in PLC context"),
                    )?);
                }
            }
        }
        let f_stop_datasync = m.new_fn("stop_datasync").vis("pub");
        f_stop_datasync.line("::rplc::tasks::stop_if_no_output_or_sfn();");
        super::write(path, m.to_string())?;
        Ok(())
    }
    pub fn generate_context<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>> {
        let mut b = path.as_ref().to_path_buf();
        b.pop();
        b.pop();
        let mut bm = b.clone();
        b.push("plc_types.rs");
        bm.push("plc_types");
        bm.push("mod.rs");
        let mut m = codegen::Scope::new();
        m.raw(crate::builder::AUTO_GENERATED);
        if b.exists() || bm.exists() {
            m.raw("#[allow(clippy::wildcard_imports)]");
            m.raw("use crate::plc_types::*;");
        }
        m.import("::rplc::export::parking_lot", "RwLock");
        m.import("::rplc::export::once_cell::sync", "Lazy");
        if self.context.serialize {
            m.import("::rplc::export::serde", "Serialize");
            m.import("::rplc::export::serde", "Deserialize");
            m.import("::rplc::export::serde", "self");
        }
        m.raw("#[allow(dead_code)] pub(crate) static CONTEXT: Lazy<RwLock<Context>> = Lazy::new(<_>::default);");
        generate_structs(
            "Context",
            &self.context.fields,
            &mut m,
            #[cfg(feature = "modbus")]
            self.context.modbus.as_ref(),
            self.context.serialize,
        )?;
        super::write(path, m.to_string())?;
        Ok(())
    }
}

fn parse_iec_type(tp: &str) -> &str {
    match tp {
        "BOOL" => "bool",
        "BYTE" | "USINT" => "u8",
        "WORD" | "UINT" => "u16",
        "DWORD" | "UDINT" => "u32",
        "LWORD" | "ULINT" => "u64",
        "SINT" => "i8",
        "INT" => "i16",
        "DINT" => "i32",
        "LINT" => "i64",
        "REAL" => "f32",
        "LREAL" => "f64",
        _ => tp,
    }
}

fn parse_type(t: &str) -> String {
    let tp = t.trim();
    if tp.ends_with(']') && !tp.starts_with('[') {
        let mut sp = tp.split('[');
        let mut result = String::new();
        let base_tp = parse_iec_type(sp.next().unwrap());
        for d in sp {
            let size = d[0..d.len() - 1].trim().parse::<usize>().unwrap();
            if result.is_empty() {
                write!(result, "[{}; {}]", base_tp, size).unwrap();
            } else {
                result = format!("[{}; {}]", result, size);
            }
        }
        result
    } else {
        parse_iec_type(tp).to_owned()
    }
}

fn generate_structs(
    name: &str,
    fields: &IndexMap<String, ContextField>,
    scope: &mut codegen::Scope,
    #[cfg(feature = "modbus")] modbus_config: Option<&ModbusConfig>,
    serialize: bool,
) -> Result<(), Box<dyn Error>> {
    let mut st: codegen::Struct = codegen::Struct::new(name);
    st.allow("dead_code")
        .allow("clippy::module_name_repetitions")
        .derive("Default")
        .repr("C")
        .vis("pub");
    if serialize {
        st.derive("Serialize").derive("Deserialize");
        st.attr("serde(crate = \"self::serde\")");
    }
    for (k, v) in fields {
        match v {
            ContextField::Type(t) => {
                let mut field = codegen::Field::new(k, parse_type(t));
                field.vis("pub");
                if serialize {
                    field.annotation.push("#[serde(default)]".to_owned());
                }
                st.push_field(field);
            }
            ContextField::Map(m) => {
                let (mut field, sub_name) = if k.ends_with(']') {
                    let (number, field_name) = if let Some(pos) = k.rfind('[') {
                        (
                            k[pos + 1..k.len() - 1].parse::<usize>().map_err(|e| {
                                eva_common::Error::invalid_params(format!(
                                    "invalid struct name: {} ({})",
                                    k, e
                                ))
                            })?,
                            &k[..pos],
                        )
                    } else {
                        return Err(eva_common::Error::invalid_params(format!(
                            "invalid struct name: {}",
                            k
                        ))
                        .into());
                    };
                    let sub_name = format!("{}{}", name, field_name.to_title_case());
                    (
                        codegen::Field::new(field_name, format!("[{}; {}]", sub_name, number)),
                        sub_name,
                    )
                } else {
                    let sub_name = format!("{}{}", name, k.to_title_case());
                    (codegen::Field::new(k, &sub_name), sub_name)
                };
                field.vis("pub");
                if serialize {
                    field.annotation.push("#[serde(default)]".to_owned());
                }
                st.push_field(field);
                generate_structs(
                    &sub_name,
                    m,
                    scope,
                    #[cfg(feature = "modbus")]
                    None,
                    serialize,
                )?;
            }
        }
    }
    #[cfg(feature = "modbus")]
    if let Some(c) = modbus_config {
        let mut field = codegen::Field::new(
            "modbus",
            format!(
                "::rplc::export::rmodbus::server::context::ModbusContext<{}>",
                c.as_const_generics()
            ),
        );
        field.vis("pub");
        if serialize {
            field.annotation.push("#[serde(default)]".to_owned());
        }
        st.push_field(field);
    }
    scope.push_struct(st);
    #[cfg(feature = "modbus")]
    if let Some(c) = modbus_config {
        let im = scope.new_impl(&format!(
            "::rplc::server::modbus::SlaveContext<{}> for Context",
            c.as_const_generics()
        ));
        {
            let fn_ctx = im
                .new_fn("modbus_context")
                .arg_ref_self()
                .ret(format!(
                    "&::rplc::export::rmodbus::server::context::ModbusContext<{}>",
                    c.as_const_generics()
                ))
                .attr("inline");
            fn_ctx.line("&self.modbus");
        }
        {
            let fn_ctx_mut = im
                .new_fn("modbus_context_mut")
                .arg_mut_self()
                .ret(format!(
                    "&mut ::rplc::export::rmodbus::server::context::ModbusContext<{}>",
                    c.as_const_generics()
                ))
                .attr("inline");
            fn_ctx_mut.line("&mut self.modbus");
        }
    }
    Ok(())
}
