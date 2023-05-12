use crate::tasks;
pub use cache::OpcCache;
use eva_common::value::Value;
use serde::Deserialize;
pub use session::{OpcSafeSess, OpcSafeSession};
use std::error::Error;
use std::time::Duration;

mod cache;
mod session;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputConfig {
    #[serde(default)]
    nodes: Vec<NodeMap>,
    #[serde(deserialize_with = "crate::interval::deserialize_interval_as_nanos")]
    sync: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputConfig {
    #[serde(default)]
    nodes: Vec<NodeMap>,
    #[serde(deserialize_with = "crate::interval::deserialize_interval_as_nanos")]
    sync: u64,
    #[serde(
        default = "default_cache",
        deserialize_with = "crate::interval::deserialize_interval_as_nanos"
    )]
    cache: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeMap {
    id: String,
    map: String,
}

fn default_timeout() -> f64 {
    1.0
}

fn default_cache() -> u64 {
    0
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(untagged)]
enum OpcAuth {
    #[default]
    Anonymous,
    User(UserAuth),
    X509(X509Auth),
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct UserAuth {
    user: String,
    password: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct X509Auth {
    cert_file: String,
    key_file: String,
}

#[derive(Deserialize)]
struct Config {
    pki_dir: Option<String>,
    #[serde(default)]
    trust_server_certs: bool,
    #[serde(default)]
    create_keys: bool,
    #[serde(default = "default_timeout")]
    timeout: f64,
    #[serde(default)]
    auth: OpcAuth,
    url: String,
}

fn push_launcher(
    kind: tasks::Kind,
    map: &[NodeMap],
    sync: u64,
    num: usize,
    id: &str,
    f: &mut codegen::Function,
    cache: Option<u64>,
) {
    f.line("let sess = session.clone();");
    f.line("let node_ids: Vec<NodeId> = vec![");
    for m in map {
        f.line(format!("\"{}\".parse().unwrap(),", m.id));
    }
    f.line("];");
    if kind == tasks::Kind::Output {
        f.line(
            format!("let mut cache = ::rplc::io::opcua::OpcCache::new(Some(::std::time::Duration::from_nanos({})));", cache.unwrap())
        );
    }
    let mut spawn_block = codegen::Block::new(&format!(
        "::rplc::tasks::spawn_{}_loop(\"{}_{}\", ::std::time::Duration::from_nanos({}), move ||",
        kind, id, num, sync,
    ));
    if kind == tasks::Kind::Output {
        spawn_block.line(&format!("{kind}_{id}_{num}(&sess, &node_ids, &mut cache);"));
    } else {
        spawn_block.line(&format!("{kind}_{id}_{num}(&sess, &node_ids);"));
    }
    spawn_block.after(");");
    f.push_block(spawn_block);
}

fn push_input_worker(num: usize, id: &str, config: InputConfig, scope: &mut codegen::Scope) {
    let f_input = scope.new_fn(&format!("input_{id}_{num}"));
    f_input.arg("session", "&::rplc::io::opcua::OpcSafeSession");
    f_input.arg(
        "node_ids",
        "&[::rplc::export::opcua::types::node_id::NodeId]",
    );
    let mut loop_block = codegen::Block::new(&format!(
        "if let Err(e) = input_{id}_{num}_worker(session, node_ids)"
    ));
    loop_block.line("session.reconnect();");
    loop_block.line("::rplc::export::log::error!(\"{}: {}\", ::rplc::tasks::thread_name(), e);");
    f_input.push_block(loop_block);
    let f_input_worker = scope.new_fn(&format!("input_{id}_{num}_worker"));
    f_input_worker.arg("session", "&::rplc::io::opcua::OpcSafeSession");
    f_input_worker.arg(
        "node_ids",
        "&[::rplc::export::opcua::types::node_id::NodeId]",
    );
    f_input_worker.ret("Result<(), Box<dyn ::std::error::Error>>");
    f_input_worker.line("use ::rplc::export::opcua::client::prelude::*;");
    f_input_worker.line("let to_read = vec![");
    for i in 0..config.nodes.len() {
        f_input_worker.line(format!(
            r#"ReadValueId {{
        node_id: node_ids[{i}].clone(),
        attribute_id: AttributeId::Value as u32,
        index_range: UAString::null(),
        data_encoding: QualifiedName::null(),
        }}"#
        ));
    }
    f_input_worker.line("];");
    f_input_worker.line("let result = session.read(&to_read, TimestampsToReturn::Neither, 0.0)??;");
    f_input_worker.line("let mut ctx = CONTEXT.write();");
    let mut for_block = codegen::Block::new("for (i, res) in result.into_iter().enumerate()");
    let mut match_idx_block = codegen::Block::new("match i");
    for (i, node) in config.nodes.into_iter().enumerate() {
        let mut idx_block = codegen::Block::new(&format!("{i} =>"));
        let mut val_block = codegen::Block::new("if let Some(value) = res.value");
        let mut val_into_block = codegen::Block::new("if let Ok(v) = value.try_into()");
        val_into_block.line(format!("ctx.{} = v;", node.map));
        val_into_block.after(&format!(
            " else {{ ::rplc::export::log::error!(\"OPC error set OPC {{}} to ctx.{{}}\", node_ids[{i}], \"{}\"); }}",
            node.map
        ));
        val_block.push_block(val_into_block);
        val_block.after(&format!(
            " else {{ ::rplc::export::log::error!(\"OPC read error {{}}\", node_ids[{i}]); }}"
        ));
        idx_block.push_block(val_block);
        match_idx_block.push_block(idx_block);
    }
    match_idx_block.push_block(codegen::Block::new("_ =>"));
    for_block.push_block(match_idx_block);
    f_input_worker.push_block(for_block);
    f_input_worker.line("Ok(())");
}

fn push_output_worker(num: usize, id: &str, config: OutputConfig, scope: &mut codegen::Scope) {
    let f_output = scope.new_fn(&format!("output_{id}_{num}"));
    f_output.arg("session", "&::rplc::io::opcua::OpcSafeSession");
    f_output.arg(
        "node_ids",
        "&[::rplc::export::opcua::types::node_id::NodeId]",
    );
    f_output.arg("cache", "&mut ::rplc::io::opcua::OpcCache");
    let mut loop_block = codegen::Block::new(&format!(
        "if let Err(e) = output_{id}_{num}_worker(session, node_ids, cache)"
    ));
    loop_block.line("::rplc::export::log::error!(\"{}: {}\", ::rplc::tasks::thread_name(), e);");
    f_output.push_block(loop_block);
    let f_output_worker = scope.new_fn(&format!("output_{id}_{num}_worker"));
    f_output_worker.arg("session", "&::rplc::io::opcua::OpcSafeSession");
    f_output_worker.arg(
        "node_ids",
        "&[::rplc::export::opcua::types::node_id::NodeId]",
    );
    f_output_worker.arg("cache", "&mut ::rplc::io::opcua::OpcCache");
    f_output_worker.ret("Result<(), Box<dyn ::std::error::Error>>");
    f_output_worker.line("use ::rplc::export::opcua::client::prelude::*;");
    f_output_worker.line("let now = DateTime::now();");
    let mut block_values = codegen::Block::new("let mut values =");
    block_values.line("let ctx = CONTEXT.read();");
    block_values.line(format!(
        "let mut values = ::std::collections::HashMap::with_capacity({});",
        config.nodes.len()
    ));
    for (i, m) in config.nodes.iter().enumerate() {
        block_values.line(format!(
            "values.insert(&node_ids[{i}], Variant::from(ctx.{}));",
            m.map
        ));
    }
    block_values.line("values");
    block_values.after(";");
    f_output_worker.push_block(block_values);
    let mut block = codegen::Block::new("if ::rplc::tasks::is_active()");
    block.line("cache.retain_map_modified(&mut values);");
    f_output_worker.push_block(block);
    let mut write_block = codegen::Block::new("if !values.is_empty()");
    write_block.line(
        r#"let to_write: Vec<WriteValue> = values
        .into_iter()
        .map(|(n, v)| WriteValue {
            node_id: n.clone(),
            attribute_id: AttributeId::Value as u32,
            index_range: UAString::null(),
            value: DataValue {
                value: Some(v),
                status: Some(StatusCode::Good),
                source_timestamp: Some(now),
                ..DataValue::default()
            },
        })
        .collect();
    "#,
    );
    write_block.line("let statuses = session.write(&to_write)??;");
    let mut status_block =
        codegen::Block::new("for (i, status) in statuses.into_iter().enumerate()");
    status_block.line("if i == node_ids.len() { break; }");
    status_block.line("if status != StatusCode::Good { ::rplc::export::log::error!(\"OPC write error {}: {}\", node_ids[i], status); }");
    write_block.push_block(status_block);
    f_output_worker.push_block(write_block);
    f_output_worker.line("Ok(())");
}

#[allow(clippy::too_many_lines)]
pub(crate) fn generate_io(
    id: &str,
    cfg: &Value,
    inputs: &[Value],
    outputs: &[Value],
) -> Result<codegen::Scope, Box<dyn Error>> {
    let mut scope = codegen::Scope::new();
    if inputs.is_empty() && outputs.is_empty() {
        return Ok(scope);
    }
    let id = id.to_lowercase();
    let config = Config::deserialize(cfg.clone())?;
    let mut launch_fn = codegen::Function::new(format!("launch_datasync_{id}"));
    launch_fn.allow("clippy::redundant_clone, clippy::unreadable_literal");
    launch_fn.line("use ::rplc::export::opcua::client::prelude::*;");
    launch_fn.line("use ::std::path::Path;");
    if let Some(pki_dir) = config.pki_dir {
        launch_fn.line(format!(
            "let pki_dir = Path::new(\"{}\").to_owned();",
            pki_dir
        ));
    } else {
        launch_fn.line("let mut pki_dir = ::rplc::var_dir();");
        launch_fn.line("pki_dir.push(format!(\"{}_pki\", crate::plc::NAME));");
    }
    match config.auth {
        OpcAuth::Anonymous => {
            launch_fn.line("let token = IdentityToken::Anonymous;");
        }
        OpcAuth::User(u) => {
            launch_fn.line(format!(
                "let token = IdentityToken::UserName(\"{}\".to_string(), \"{}\".to_string());",
                u.user, u.password
            ));
        }
        OpcAuth::X509(x) => {
            if x.cert_file.starts_with('/') {
                launch_fn.line(format!(
                    "let cert_file = Path::new(\"{}\").to_owned();",
                    x.cert_file
                ));
            } else {
                launch_fn.line("let mut cert_file = pki_dir.clone();");
                launch_fn.line(format!("cert_file.push(\"{}\");", x.cert_file));
            }
            if x.key_file.starts_with('/') {
                launch_fn.line(format!(
                    "let key_file = Path::new(\"{}\").to_owned();",
                    x.key_file
                ));
            } else {
                launch_fn.line("let mut key_file = pki_dir.clone();");
                launch_fn.line(format!("key_file.push(\"{}\");", x.cert_file));
            }
            launch_fn.line("let token = IdentityToken::X509(cert_file, key_file);");
        }
    }
    launch_fn.line(format!(
        r#"let client = ClientBuilder::new()
        .application_name(format!("rplc.{{}}", crate::plc::NAME))
        .application_uri("urn:")
        .product_uri("urn:")
        .ignore_clock_skew()
        .trust_server_certs({})
        .create_sample_keypair({})
        .pki_dir(pki_dir)
        .session_retry_limit(0)
        .session_name(format!("{{}}.{{}}", ::rplc::hostname(), crate::plc::NAME))
        .session_timeout({})
        .client()
        .unwrap();
        "#,
        config.trust_server_certs,
        config.create_keys,
        Duration::from_secs_f64(config.timeout).as_millis()
    ));
    launch_fn.line(format!(
        r#"let session = ::std::sync::Arc::new(::rplc::io::opcua::OpcSafeSess::new(
        client,
        (
            "{}",
            "None",
            MessageSecurityMode::None,
            UserTokenPolicy::anonymous(),
        ),
        token
    ));
    "#,
        config.url
    ));
    for (i, input) in inputs.iter().enumerate() {
        let input_config = InputConfig::deserialize(input.clone())?;
        if !input_config.nodes.is_empty() {
            push_launcher(
                tasks::Kind::Input,
                &input_config.nodes,
                input_config.sync,
                i + 1,
                &id,
                &mut launch_fn,
                None,
            );
            push_input_worker(i + 1, &id, input_config, &mut scope);
        }
    }
    for (i, output) in outputs.iter().enumerate() {
        let output_config = OutputConfig::deserialize(output.clone())?;
        if !output_config.nodes.is_empty() {
            push_launcher(
                tasks::Kind::Output,
                &output_config.nodes,
                output_config.sync,
                i + 1,
                &id,
                &mut launch_fn,
                Some(output_config.cache),
            );
            push_output_worker(i + 1, &id, output_config, &mut scope);
        }
    }
    scope.push_fn(launch_fn);
    Ok(scope)
}
