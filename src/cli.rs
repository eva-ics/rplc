use clap::Parser;
use colored::Colorize;
use eva_common::EResult;
use prettytable::Row;
use rplc::tasks::{Affinity, Status};
use rplc::{client, eapi};
use std::collections::BTreeMap;
use std::path::Path;

#[macro_use]
extern crate prettytable;

#[derive(Parser)]
struct Args {
    #[clap(long = "color")]
    color: Option<Color>,
    #[clap(subcommand)]
    command: Command,
}

#[derive(clap::ValueEnum, Copy, Clone)]
enum Color {
    Always,
    Never,
}

impl Color {
    fn ovrride(self) {
        colored::control::set_override(match self {
            Color::Always => true,
            Color::Never => false,
        });
    }
}

#[derive(Parser)]
enum Command {
    #[clap(about = "list PLCs")]
    List(ListParams),
    #[clap(about = "Test PLC")]
    Test(PlcParams),
    #[clap(about = "PLC info")]
    Info(PlcParams),
    #[clap(about = "PLC task (thread) stats")]
    Stat(PlcParams),
    #[clap(about = "reset PLC task (thread) stats")]
    Reset(PlcParams),
    #[clap(about = "register PLC binary in systemd")]
    Register(PlcRegisterParams),
    #[clap(about = "unregister PLC binary from systemd (stop if running)")]
    Unregister(PlcParams),
    #[clap(about = "start PLC with systemd")]
    Start(PlcParams),
    #[clap(about = "stop PLC with systemd")]
    Stop(PlcParams),
    #[clap(about = "restart PLC with systemd")]
    Restart(PlcParams),
    #[clap(about = "PLC systemd status")]
    Status(PlcParams),
}

#[derive(Parser)]
struct PlcRegisterParams {
    plc_file_path: String,
    #[clap(short = 'a', help = "thread affinity: NAME=CPU,PRIORITY")]
    thread_affinity: Vec<String>,
    #[clap(
        short = 'e',
        long = "eapi",
        help = "EVA ICS bus connection: path[,timeout=Xs][,buf_size=X][,queue_size=X][,buf_ttl=Xms]"
    )]
    eapi: Option<String>,
    #[clap(long = "var", help = "Custom environment variable: name=value")]
    vars: Vec<String>,
    #[clap(long = "force")]
    force: bool,
    #[clap(short = 's', long = "start", help = "start PLC after registration")]
    start: bool,
}

trait StatusColored {
    fn as_colored_string(&self) -> colored::ColoredString;
}

impl StatusColored for Status {
    fn as_colored_string(&self) -> colored::ColoredString {
        if *self == Status::Active {
            self.to_string().green()
        } else if *self <= Status::Stopping {
            self.to_string().yellow()
        } else {
            self.to_string().normal()
        }
    }
}

#[derive(Parser)]
struct ListParams {
    #[clap(short = 'y', long = "full")]
    full: bool,
}

#[derive(Parser)]
struct PlcParams {
    name: String,
}

fn ctable(titles: &[&str]) -> prettytable::Table {
    let mut table = prettytable::Table::new();
    let format = prettytable::format::FormatBuilder::new()
        .column_separator(' ')
        .borders(' ')
        .separators(
            &[prettytable::format::LinePosition::Title],
            prettytable::format::LineSeparator::new('-', '-', '-', '-'),
        )
        .padding(0, 1)
        .build();
    table.set_format(format);
    let mut titlevec: Vec<prettytable::Cell> = Vec::new();
    for t in titles {
        titlevec.push(cell!(t.blue()));
    }
    table.set_titles(prettytable::Row::new(titlevec));
    table
}

async fn handle_list(p: ListParams, var_dir: &Path) -> EResult<()> {
    if p.full {
        let mut table = ctable(&[
            "name",
            "description",
            "version",
            "systemd",
            "status",
            "pid",
            "uptime",
        ]);
        for r in client::list_extended(var_dir).await? {
            let systemd = r
                .sds
                .and_then(|v| v.status)
                .map_or_else(String::new, |v| v.to_string());
            let plc_info = r.plc_info;
            let status = if plc_info.status == -1000 {
                "API_ERROR".red()
            } else {
                Status::from(plc_info.status).as_colored_string()
            };
            table.add_row(row![
                plc_info.name,
                plc_info.description,
                plc_info.version,
                systemd,
                status,
                plc_info.pid,
                plc_info.uptime.trunc()
            ]);
        }
        table.printstd();
    } else {
        for name in client::list(var_dir).await? {
            println!("{}", name);
        }
    }
    Ok(())
}

async fn handle_stat(p: PlcParams, var_dir: &Path) -> EResult<()> {
    let tasks = client::stat_extended(&p.name, var_dir).await?;
    let mut table = ctable(&[
        "task", "spid", "cpu", "rt", "iters", "jmin", "jmax", "jlast", "javg",
    ]);
    for task in tasks {
        let mut cols = vec![
            cell!(task.name),
            cell!(task.spid.to_string().green()),
            cell!(task.cpu_id.to_string().bold()),
            cell!(if task.rt_priority > 0 {
                task.rt_priority.to_string().cyan()
            } else {
                String::new().normal()
            }),
        ];
        if let Some(t) = task.thread_info {
            let cols_t = vec![
                cell!(t.iters),
                cell!(t.jitter_min),
                cell!(if t.jitter_max < 150 {
                    t.jitter_max.to_string().normal()
                } else if t.jitter_max < 250 {
                    t.jitter_max.to_string().yellow()
                } else {
                    t.jitter_max.to_string().red()
                }),
                cell!(t.jitter_last),
                cell!(t.jitter_avg),
            ];
            cols.extend(cols_t);
        }
        table.add_row(Row::new(cols));
    }
    table.printstd();
    Ok(())
}

async fn handle_info(p: PlcParams, var_dir: &Path) -> EResult<()> {
    let result = client::info(&p.name, var_dir).await?;
    let mut table = ctable(&["variable", "value"]);
    table.add_row(row!["name", result.name]);
    table.add_row(row!["description", result.description]);
    table.add_row(row!["version", result.version]);
    table.add_row(row![
        "status",
        Status::from(result.status).as_colored_string()
    ]);
    table.add_row(row!["pid", result.pid]);
    table.add_row(row!["system_name", result.system_name]);
    table.add_row(row!["uptime", result.uptime.trunc()]);
    table.printstd();
    Ok(())
}

async fn handle_start(name: &str) -> EResult<()> {
    client::start(name).await?;
    println!("{} has been started", name);
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> EResult<()> {
    let args = Args::parse();
    let var_dir = rplc::var_dir();
    if let Some(color) = args.color {
        color.ovrride();
    }
    match args.command {
        Command::List(p) => {
            handle_list(p, &var_dir).await?;
        }
        Command::Test(p) => {
            client::test(&p.name, &var_dir).await?;
            println!("OK");
        }
        Command::Info(p) => {
            handle_info(p, &var_dir).await?;
        }
        Command::Stat(p) => {
            handle_stat(p, &var_dir).await?;
        }
        Command::Reset(p) => {
            client::reset_stat(&p.name, &var_dir).await?;
            println!("{} stats have been reset", p.name);
        }
        Command::Register(p) => {
            let aff: BTreeMap<String, Affinity> = p
                .thread_affinity
                .into_iter()
                .map(|a| {
                    let mut sp = a.split('=');
                    let name = sp.next().unwrap().to_owned();
                    let a: Affinity = sp
                        .next()
                        .ok_or_else(|| panic!("no affinity specified"))
                        .unwrap()
                        .parse()
                        .unwrap();
                    (name, a)
                })
                .collect();
            let eapi_params: Option<eapi::Params> = p.eapi.map(|s| s.parse().unwrap());
            let (name, svc_name) = client::register(
                Path::new(&p.plc_file_path),
                &var_dir,
                p.force,
                &aff,
                eapi_params.as_ref(),
                &p.vars,
            )
            .await?;
            println!(
                "{} has been registered in systemd as: {} ({})",
                p.plc_file_path, name, svc_name
            );
            if p.force {
                handle_start(&name).await?;
            }
        }
        Command::Unregister(p) => {
            client::unregister(&p.name).await?;
            println!("{} has been unregistered from systemd", p.name);
        }
        Command::Start(p) => {
            handle_start(&p.name).await?;
        }
        Command::Stop(p) => {
            client::stop(&p.name).await?;
            println!("{} has been stopped", p.name);
        }
        Command::Restart(p) => {
            client::restart(&p.name).await?;
            println!("{} has been restarted", p.name);
        }
        Command::Status(p) => {
            println!("{}", client::status(&p.name).await?);
        }
    }
    Ok(())
}
