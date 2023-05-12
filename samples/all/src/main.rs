use rplc::prelude::*;

mod plc;
mod plc_types;

use std::time::Duration;
use std::time::Instant;

#[plc_program(loop = "500ms")]
fn p1() {
    let mut ctx = plc_context_mut!();
    //info!("p1");
    if ctx.temperature > 25.0 {
        ctx.fan = true;
        ctx.fan2 = false;
    } else if ctx.temperature < 23.0 {
        ctx.fan = false;
        ctx.fan2 = true;
    }
    info!("fan1: {}, fan2: {}", ctx.fan, ctx.fan2);
    ctx.data.subfield.a += 1_000_000_000;
    ctx.data.subfield.b += 10;
    let temp = ctx.temperature;
    ctx.modbus.set_holding(20, (temp * 100.0) as u16).unwrap();
    ctx.timers.t1 = Some(Instant::now());
    if let Some(info) = rplc::tasks::controller_stats().lock().current_thread_info() {
        ctx.modbus.set_inputs_from_u32(100, info.iters).unwrap();
        ctx.modbus.set_input(102, info.jitter_max).unwrap();
        ctx.modbus.set_input(103, info.jitter_min).unwrap();
        ctx.modbus.set_input(104, info.jitter_avg).unwrap();
        ctx.modbus.set_input(105, info.jitter_last).unwrap();
    }
}

#[plc_program(loop = "1s")]
fn p2() {
    let mut ctx = plc_context_mut!();
    ctx.data.counter += 1;
    //info!("p2");
    info!(
        "temperature: {}, counter: {}, opc temps: {:?}",
        ctx.temperature, ctx.data.counter, ctx.data.opc_temp
    );
    ctx.data.subfield.temp_out = ctx.temperature;
    if ctx.router_if1 {
        info!("router up");
    } else {
        warn!("router down");
    }
}

fn get_if_status(
    oid: &[u32],
    session: &mut snmp::SyncSession,
) -> Result<bool, Box<dyn std::error::Error>> {
    let response = session
        .get(oid)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "snmp"))?;
    if let Some((_oid, snmp::Value::Integer(state))) = response.varbinds.last() {
        Ok(state == 1)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "snmp value",
        ))?
    }
}

fn spawn_check_router() {
    let oid = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 8, 1];
    let agent_addr = "10.90.34.1:161";
    let community = b"public";
    let timeout = Duration::from_secs(2);
    let mut session = snmp::SyncSession::new(agent_addr, community, Some(timeout), 0).unwrap();
    rplc::tasks::spawn_input_loop(
        "router1",
        Duration::from_secs(2),
        move || match get_if_status(oid, &mut session) {
            Ok(v) => {
                plc_context_mut!().router_if1 = v;
            }
            Err(e) => {
                error!("{}", e);
            }
        },
    );
}

fn spawn_relays() {
    rplc::tasks::spawn_output_loop("relays", Duration::from_secs(2), move || {
        info!("relays set");
    });
}

fn shutdown() {
    warn!("shutting down");
    let mut ctx = plc_context_mut!();
    ctx.fan = false;
    ctx.fan2 = false;
    ctx.fan3 = false;
    ctx.fan4 = false;
    warn!("shutdown program completed");
}

use std::fs;

fn main() {
    init_plc!();
    rplc::tasks::on_shutdown(shutdown);
    if let Ok(data) = fs::read("plc.dat") {
        info!("loading context");
        *plc_context_mut!() = rmp_serde::from_slice(&data).unwrap();
    }
    p1_spawn();
    p2_spawn();
    spawn_check_router();
    spawn_relays();
    rplc::tasks::spawn_stats_log(Duration::from_secs(5));
    run_plc!();
    fs::write(
        "plc.dat",
        rmp_serde::to_vec_named(&*plc_context!()).unwrap(),
    )
    .unwrap();
}
