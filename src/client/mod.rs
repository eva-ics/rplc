use crate::tasks::{Affinity, ThreadInfo};
use crate::{api, eapi, PlcInfo};
use bmart_derive::{EnumStr, Sorting};
use eva_common::payload::{pack, unpack};
use eva_common::prelude::Value;
use eva_common::{EResult, Error};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const TIMEOUT: Duration = Duration::from_secs(2);
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(2);
const START_STOP_TIMEOUT: Duration = Duration::from_secs(60);
const PLC_SYSTEMD_PREFIX: &str = "rplc.";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const SYSTEMD_DIR: &str = "/etc/systemd/system";

#[derive(Default, Debug, Sorting)]
#[sorting(id = "spid")]
struct ProcInfo {
    name: String,
    spid: i32,
    cpu_id: u8,
    rt_priority: i8,
}

async fn proc_info(procfs_path: &Path) -> Result<ProcInfo, Box<dyn std::error::Error>> {
    let mut stat_path = procfs_path.to_owned();
    stat_path.push("stat");
    let stat = fs::read_to_string(stat_path).await?;
    let vals: Vec<&str> = stat.split(' ').collect();
    let name = vals.get(1).ok_or_else(|| Error::invalid_data("COL2"))?;
    if name.len() < 2 {
        return Err(Error::invalid_data("COL2").into());
    }
    Ok(ProcInfo {
        name: name[1..name.len() - 1].to_owned(),
        spid: procfs_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .parse()?,
        cpu_id: vals
            .get(38)
            .ok_or_else(|| Error::invalid_data("COL39"))?
            .parse()?,
        rt_priority: vals
            .get(39)
            .ok_or_else(|| Error::invalid_data("COL40"))?
            .parse()?,
    })
}

#[inline]
async fn get_proc_info(procfs_path: &Path) -> ProcInfo {
    proc_info(procfs_path).await.unwrap_or_else(|e| ProcInfo {
        name: format!("PROCFS ERR: {}", e),
        ..ProcInfo::default()
    })
}

#[derive(Serialize, Deserialize, Sorting)]
#[sorting(id = "name")]
pub struct ThreadInfoExtended {
    pub name: String,
    pub spid: i32,
    pub cpu_id: u8,
    pub rt_priority: i8,
    pub thread_info: Option<ThreadInfo>,
}

pub fn plc_socket_path(var_dir: &Path, name: &str) -> EResult<PathBuf> {
    let mut path = var_dir.to_owned();
    path.push(format!("{}.plcsock", name));
    if path.exists() {
        Ok(path)
    } else {
        Err(Error::not_found("no API socket, is PLC process running?"))
    }
}

pub async fn stat_extended(name: &str, var_dir: &Path) -> EResult<Vec<ThreadInfoExtended>> {
    let socket_path = plc_socket_path(var_dir, name)?;
    let info: PlcInfo = api_call(&socket_path, "info", None).await?;
    let mut thread_info_map: BTreeMap<String, Option<ThreadInfo>> =
        api_call(&socket_path, "thread_stats.get", None).await?;
    let mut paths = fs::read_dir(format!("/proc/{}/task", info.pid)).await?;
    let mut tasks = Vec::new();
    while let Some(path) = paths.next_entry().await? {
        if path.file_type().await?.is_dir() {
            let p = get_proc_info(&path.path()).await;
            let thread_info = thread_info_map.remove(&p.name);
            tasks.push(ThreadInfoExtended {
                name: p.name,
                spid: p.spid,
                cpu_id: p.cpu_id,
                rt_priority: p.rt_priority,
                thread_info: thread_info.flatten(),
            });
        }
    }
    tasks.sort();
    Ok(tasks)
}

pub async fn info(name: &str, var_dir: &Path) -> EResult<PlcInfo> {
    let socket_path = plc_socket_path(var_dir, name)?;
    let result: PlcInfo = api_call(&socket_path, "info", None).await?;
    Ok(result)
}

pub async fn reset_stat(name: &str, var_dir: &Path) -> EResult<()> {
    let socket_path = plc_socket_path(var_dir, name)?;
    api_call::<()>(&socket_path, "thread_stats.reset", None).await?;
    Ok(())
}

pub async fn test(name: &str, var_dir: &Path) -> EResult<()> {
    let socket_path = plc_socket_path(var_dir, name)?;
    api_call::<()>(&socket_path, "test", None).await?;
    Ok(())
}

pub async fn list(var_dir: &Path) -> EResult<Vec<String>> {
    let mut plcs = BTreeSet::<String>::new();
    let mut paths = fs::read_dir(var_dir).await?;
    while let Some(path) = paths.next_entry().await? {
        let p = path.path();
        if let Some(ext) = p.extension() {
            if ext == "plcsock" {
                if let Some(name) = p.file_stem() {
                    plcs.insert(name.to_string_lossy().to_string());
                }
            }
        }
    }
    for (name, _) in systemd_units().await.map_err(Error::failed)? {
        plcs.insert(name);
    }
    let mut result: Vec<String> = plcs.into_iter().collect();
    result.sort();
    Ok(result)
}

#[derive(Serialize, Deserialize)]
pub struct PlcInfoExtended {
    pub plc_info: PlcInfo,
    pub sds: Option<SystemdUnitStats>,
}

/// # Panics
///
/// should not panic
pub async fn list_extended(var_dir: &Path) -> EResult<Vec<PlcInfoExtended>> {
    let mut infos = BTreeMap::new();
    let mut paths = fs::read_dir(var_dir).await?;
    while let Some(path) = paths.next_entry().await? {
        let p = path.path();
        if let Some(ext) = p.extension() {
            if ext == "plcsock" {
                if let Some(name) = p.file_stem() {
                    let name = name.to_string_lossy().to_string();
                    let plc_info = info(&name, var_dir).await.unwrap_or_else(|_| PlcInfo {
                        name: name.clone(),
                        status: -1000,
                        ..PlcInfo::default()
                    });
                    infos.insert(
                        name,
                        PlcInfoExtended {
                            plc_info,
                            sds: None,
                        },
                    );
                }
            }
        }
    }
    for (name, sds) in systemd_units().await.map_err(Error::failed)? {
        if let Some(info) = infos.get_mut(&name) {
            info.sds = Some(sds);
        } else {
            infos.insert(
                name.clone(),
                PlcInfoExtended {
                    plc_info: PlcInfo {
                        name,
                        ..PlcInfo::default()
                    },
                    sds: Some(sds),
                },
            );
        }
    }
    let mut result: Vec<PlcInfoExtended> = infos.into_values().collect();
    result.sort_by(|a, b| a.plc_info.name.partial_cmp(&b.plc_info.name).unwrap());
    Ok(result)
}

async fn api_call<R>(socket_path: &Path, method: &str, params: Option<Value>) -> EResult<R>
where
    R: DeserializeOwned,
{
    tokio::time::timeout(TIMEOUT, _api_call(socket_path, method, params))
        .await
        .map_err(|_| Error::timeout())?
}

async fn _api_call<R>(socket_path: &Path, method: &str, params: Option<Value>) -> EResult<R>
where
    R: DeserializeOwned,
{
    let req = api::Request::new(method, params);
    let mut socket = UnixStream::connect(socket_path).await?;
    let packed = pack(&req)?;
    let mut buf = Vec::with_capacity(packed.len() + 5);
    buf.push(0);
    buf.extend(u32::try_from(packed.len())?.to_le_bytes());
    buf.extend(packed);
    socket.write_all(&buf).await?;
    let mut buf: [u8; 5] = [0; 5];
    socket.read_exact(&mut buf).await?;
    if buf[0] != 0 {
        return Err(Error::invalid_data("invalid header"));
    }
    let mut buf = vec![0; usize::try_from(u32::from_le_bytes(buf[1..].try_into()?))?];
    socket.read_exact(&mut buf).await?;
    let response: api::Response = unpack(&buf)?;
    response.check()?;
    Ok(R::deserialize(response.result.unwrap_or_default())?)
}

fn systemd_plc_and_service_name_from_path(plc: &Path) -> EResult<(String, String)> {
    let name = plc
        .file_name()
        .ok_or_else(|| Error::invalid_params("the file has no name"))?
        .to_string_lossy();
    Ok((name.to_string(), systemd_service_name(&name)))
}

#[inline]
fn systemd_service_name(name: &str) -> String {
    format!("{}{}.service", PLC_SYSTEMD_PREFIX, name)
}

const SYSTEMD_UNIT_CONFIG: &str = r#"[Unit]
Description=
After=network.target
StartLimitIntervalSec=0

[Service]
Type=simple
Restart=always
RestartSec=1
{% for entry in env -%}
Environment="{{ entry }}"
{% endfor -%}
ExecStart={{ bin_path }}

[Install]
WantedBy=multi-user.target
"#;

// TODO thread affinity
// TODO eapi path and settings
pub async fn register(
    plc: &Path,
    var_dir: &Path,
    force: bool,
    affinities: &BTreeMap<String, Affinity>,
    eapi_params: Option<&eapi::Params>,
    vars: &[String],
) -> EResult<(String, String)> {
    if !plc.is_file() {
        return Err(Error::invalid_params(format!(
            "not a file: {}",
            plc.to_string_lossy()
        )));
    }
    let mut su_path = Path::new(SYSTEMD_DIR).to_owned();
    let (plc_name, su_name) = systemd_plc_and_service_name_from_path(plc)?;
    su_path.push(&su_name);
    if su_path.exists() {
        if force {
            let _ = stop(&plc_name).await;
            let _ = unregister(&plc_name).await;
        } else {
            return Err(Error::busy("PLC is already registered"));
        }
    }
    let mut ctx = tera::Context::new();
    let mut env = vec![
        "SYSLOG=1".to_owned(),
        format!("PLC_VAR_DIR={}", var_dir.to_string_lossy()),
    ];
    if let Some(params) = eapi_params {
        env.push(format!("PLC_EAPI={}", params));
    }
    for var in vars {
        env.push(var.clone());
    }
    for (task_name, aff) in affinities {
        env.push(format!(
            "PLC_THREAD_AFFINITY_{}={},{}",
            task_name.replace('.', "__"),
            aff.cpu_id,
            aff.sched_priority
        ));
    }
    ctx.insert("env", &env);
    ctx.insert("bin_path", &plc.canonicalize()?);
    let mut tera = tera::Tera::default();
    let unit_config = tera
        .render_str(SYSTEMD_UNIT_CONFIG, &ctx)
        .map_err(Error::failed)?;
    fs::write(&su_path, unit_config).await?;
    if let Err(e) = systemd_svc_action(&su_name, SystemdAction::Enable).await {
        let _ = fs::remove_file(su_path).await;
        Err(e)
    } else {
        Ok((plc_name, su_name))
    }
}

fn systemd_service_checked(name: &str) -> EResult<(PathBuf, String)> {
    let su_name = systemd_service_name(name);
    let mut su_path = Path::new(SYSTEMD_DIR).to_owned();
    su_path.push(&su_name);
    if !su_path.exists() {
        return Err(Error::not_found("PLC is not registered"));
    }
    Ok((su_path, su_name))
}

pub async fn unregister(name: &str) -> EResult<()> {
    let (su_path, su_name) = systemd_service_checked(name)?;
    let _ = stop(name).await;
    systemd_svc_action(&su_name, SystemdAction::Disable).await?;
    fs::remove_file(su_path).await?;
    Ok(())
}

pub async fn start(name: &str) -> EResult<()> {
    let (_, su_name) = systemd_service_checked(name)?;
    systemd_svc_action(&su_name, SystemdAction::Start).await?;
    Ok(())
}

pub async fn stop(name: &str) -> EResult<()> {
    let (_, su_name) = systemd_service_checked(name)?;
    systemd_svc_action(&su_name, SystemdAction::Stop).await?;
    Ok(())
}

pub async fn restart(name: &str) -> EResult<()> {
    let (_, su_name) = systemd_service_checked(name)?;
    systemd_svc_action(&su_name, SystemdAction::Restart).await?;
    Ok(())
}

/// # Panics
///
/// Should not panic
pub async fn status(name: &str) -> EResult<String> {
    let (_, su_name) = systemd_service_checked(name)?;
    let status_str = systemd_svc_action(&su_name, SystemdAction::Status)
        .await?
        .unwrap();
    Ok(status_str)
}

#[derive(EnumStr, Copy, Clone, Eq, PartialEq)]
#[enumstr(rename_all = "lowercase")]
enum SystemdAction {
    Enable,
    Disable,
    Start,
    Stop,
    Restart,
    Status,
}

impl SystemdAction {
    fn timeout(self) -> Duration {
        match self {
            SystemdAction::Enable | SystemdAction::Disable | SystemdAction::Status => {
                SYSTEMCTL_TIMEOUT
            }
            SystemdAction::Start | SystemdAction::Stop => START_STOP_TIMEOUT,
            SystemdAction::Restart => START_STOP_TIMEOUT * 2,
        }
    }
}

async fn systemd_svc_action(su_name: &str, action: SystemdAction) -> EResult<Option<String>> {
    let result = bmart::process::command(
        SYSTEMCTL,
        &[action.to_string(), su_name.to_owned()],
        action.timeout(),
        bmart::process::Options::default(),
    )
    .await?;
    let mut code = result.code.unwrap_or(-1);
    if action == SystemdAction::Status && code == 3 {
        code = 0;
    }
    if code != 0 {
        return Err(Error::failed("systemctl exited with an error"));
    }
    if action == SystemdAction::Status {
        Ok(Some(result.out.join("\n")))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemdUnitStats {
    pub status: Option<SystemdStatus>,
}

#[derive(EnumStr, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[enumstr(rename_all = "lowercase")]
pub enum SystemdStatus {
    Active,
    Activating,
    Deactivating,
    Inactive,
}

async fn systemd_units() -> EResult<BTreeMap<String, SystemdUnitStats>> {
    let mut units = BTreeMap::new();
    let result = bmart::process::command(
        SYSTEMCTL,
        &["-a"],
        SYSTEMCTL_TIMEOUT,
        bmart::process::Options::default(),
    )
    .await?;
    if result.code.unwrap_or(-1) != 0 {
        return Err(Error::failed("systemctl exited with an error"));
    }
    for line in result.out.into_iter().skip(1) {
        if line.is_empty() {
            break;
        }
        let l = if line.starts_with('●') {
            &line[4..]
        } else {
            &line
        };
        let vals: Vec<&str> = l.split_ascii_whitespace().take(4).collect();
        if vals.len() == 4 {
            if let Some(plc_name) = vals[0].strip_prefix(PLC_SYSTEMD_PREFIX) {
                if let Some(plc_name) = plc_name.strip_suffix(".service") {
                    units.insert(
                        plc_name.to_owned(),
                        SystemdUnitStats {
                            status: vals[2].parse().ok(),
                        },
                    );
                }
            }
        }
    }
    Ok(units)
}
