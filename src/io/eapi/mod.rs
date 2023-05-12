use eva_common::value::Value;
use eva_common::OID;
use serde::Deserialize;
use std::error::Error;

pub mod util;

fn default_cache() -> u64 {
    0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputConfig {
    oid_map: Vec<OidMap>,
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
struct InputConfig {
    action_map: Vec<OidMap>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OidMap {
    oid: OID,
    status: Option<String>,
    value: Option<String>,
}

fn generate_output(id: &str, i: usize, scope: &mut codegen::Scope, output_config: OutputConfig) {
    let mut output_fn = codegen::Function::new(&format!("output_{}_{}", id, i + 1));
    output_fn
        .arg(
            "oids",
            "&[::std::sync::Arc<::rplc::export::eva_common::OID>]",
        )
        .arg(
            "cache",
            "&mut ::rplc::export::eva_sdk::controller::RawStateCache",
        );
    let mut block = codegen::Block::new(&format!(
        "if let Err(e) = output_{}_{}_worker(oids, cache)",
        id,
        i + 1
    ));
    block.line("::rplc::export::log::error!(\"{}: {}\", ::rplc::tasks::thread_name(), e);");
    output_fn.push_block(block);
    let mut worker_fn = codegen::Function::new(&format!("output_{}_{}_worker", id, i + 1));
    worker_fn
        .arg(
            "oids",
            "&[::std::sync::Arc<::rplc::export::eva_common::OID>]",
        )
        .arg(
            "cache",
            "&mut ::rplc::export::eva_sdk::controller::RawStateCache",
        );
    worker_fn.ret("Result<(), Box<dyn ::std::error::Error>>");
    worker_fn.line("use ::rplc::export::eva_common::events::RawStateEventOwned;");
    worker_fn.line("use ::rplc::export::eva_common::OID;");
    worker_fn.line("use ::rplc::export::eva_common::value::{to_value, ValueOptionOwned};");
    worker_fn.line("use ::rplc::export::eva_sdk::controller::RawStateEventPreparedOwned;");
    worker_fn.line("let mut result: ::std::collections::HashMap<&OID, RawStateEventPreparedOwned> = <_>::default();");
    let mut out_block = codegen::Block::new("");
    out_block.line("let ctx = CONTEXT.read();");
    for (entry_id, entry) in output_config.oid_map.into_iter().enumerate() {
        if entry.status.is_none() && entry.value.is_none() {}
        assert!(
            !(entry.status.is_none() && entry.value.is_none()),
            "EVA ICS output entry {} for {} has no status/value fields",
            id,
            entry.oid
        );
        out_block.line(format!("// {}", entry.oid));
        if let Some(status) = entry.status {
            out_block.line(format!("let status = i16::try_from(ctx.{})?;", status));
        } else {
            out_block.line("let status = 1;");
        }
        if let Some(value) = entry.value {
            out_block.line(format!(
                "let value = ValueOptionOwned::Value(to_value(ctx.{})?);",
                value
            ));
        } else {
            out_block.line("let value = ValueOptionOwned::No;");
        }
        out_block.line(format!("result.insert(&oids[{}],", entry_id));
        out_block.line("RawStateEventPreparedOwned::from_rse_owned(");
        out_block.line("RawStateEventOwned { status, value, force: false, },");
        out_block.line("None));");
    }
    worker_fn.push_block(out_block);
    let mut block = codegen::Block::new("if ::rplc::tasks::is_active()");
    block.line("cache.retain_map_modified(&mut result);");
    worker_fn.push_block(block);
    worker_fn.line("::rplc::eapi::notify(result)?;");
    worker_fn.line("Ok(())");
    scope.push_fn(output_fn);
    scope.push_fn(worker_fn);
}

fn generate_input(id: &str, i: usize, scope: &mut codegen::Scope, input_config: InputConfig) {
    for (entry_id, entry) in input_config.action_map.into_iter().enumerate() {
        assert!(
            !(entry.status.is_none() && entry.value.is_none()),
            "EVA ICS input entry {} for {} has no status/value fields",
            id,
            entry.oid
        );
        let mut handler_fn = codegen::Function::new(format!(
            "handle_eapi_action_{}_{}_{}",
            id,
            i + 1,
            entry_id + 1
        ));
        handler_fn.arg("action", "&mut ::rplc::export::eva_sdk::controller::Action");
        handler_fn.ret("::rplc::export::eva_common::EResult<()>");
        handler_fn.line("use ::rplc::io::eapi::util::ItemStatusX;");
        handler_fn.line("let params = action.take_unit_params()?;");
        if entry.status.is_some() {
            handler_fn.line("let status = ItemStatusX(params.status).try_into()?;");
        }
        if entry.value.is_some() {
            let mut block =
                codegen::Block::new("let value = if let ::rplc::export::eva_common::value::ValueOptionOwned::Value(v) = params.value");
            block.line("Some(v.try_into()?)");
            block.after("else { None };");
            handler_fn.push_block(block);
        }
        handler_fn.line("let mut ctx = CONTEXT.write();");
        if let Some(status) = entry.status {
            handler_fn.line(format!("ctx.{} = status;", status));
        }
        if let Some(value) = entry.value {
            let mut block = codegen::Block::new("if let Some(v) = value");
            block.line(format!("ctx.{} = v;", value));
            handler_fn.push_block(block);
        }
        handler_fn.line("Ok(())");
        scope.push_fn(handler_fn);
    }
}

pub(crate) fn generate_io(
    id: &str,
    cfg: &Value,
    inputs: &[Value],
    outputs: &[Value],
) -> Result<codegen::Scope, Box<dyn Error>> {
    let id = id.to_lowercase();
    let mut scope = codegen::Scope::new();
    assert_eq!(cfg, &Value::Unit, "EVA ICS I/O must have no config");
    let mut launch_fn = codegen::Function::new(&format!("launch_datasync_{id}"));
    launch_fn.allow("clippy::redundant_clone, clippy::unreadable_literal");
    launch_fn.line("use ::rplc::export::eva_common::OID;");
    for (i, input) in inputs.iter().enumerate() {
        let input_config = InputConfig::deserialize(input.clone())?;
        launch_fn.line("let oids: Vec<OID> = vec![");
        for entry in &input_config.action_map {
            launch_fn.line(format!("\"{}\".parse::<OID>().unwrap(),", entry.oid));
        }
        launch_fn.line("];");
        launch_fn.line("let handlers: Vec<::rplc::eapi::ActionHandlerFn> = vec![");
        for x in 0..input_config.action_map.len() {
            launch_fn.line(format!(
                "Box::new(handle_eapi_action_{}_{}_{}),",
                id,
                i + 1,
                x + 1
            ));
        }
        launch_fn.line("];");
        launch_fn.line("::rplc::eapi::append_action_handlers_bulk(&oids, handlers);");
        generate_input(&id, i, &mut scope, input_config);
    }
    for (i, output) in outputs.iter().enumerate() {
        let output_config = OutputConfig::deserialize(output.clone())?;
        launch_fn.line(
            format!("let mut cache = ::rplc::export::eva_sdk::controller::RawStateCache::new(Some(::std::time::Duration::from_nanos({})));", output_config.cache)
        );
        launch_fn.line("let oids: Vec<::std::sync::Arc<OID>> = vec![");
        for entry in &output_config.oid_map {
            launch_fn.line(format!("\"{}\".parse::<OID>().unwrap().into(),", entry.oid));
        }
        launch_fn.line("];");
        let mut block = codegen::Block::new(&format!(
            "::rplc::tasks::spawn_output_loop(\"{}_{}\", ::std::time::Duration::from_nanos({}), move ||",
            id, i + 1, output_config.sync
        ));
        block.line(format!("output_{}_{}(&oids, &mut cache);", id, i + 1));
        block.after(");");
        launch_fn.push_block(block);
        generate_output(&id, i, &mut scope, output_config);
    }
    scope.push_fn(launch_fn);
    Ok(scope)
}
