use crate::{backend::Backend, config::Settings, model::*, system, tui};
use anyhow::{Context, Result, ensure};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, IsTerminal, Write},
    path::PathBuf,
};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthStr;

#[derive(Parser, Debug)]
#[command(
    name = "kami",
    version,
    about = "law-of-cycles — a terminal workspace for Mihomo"
)]
pub struct Cli {
    #[arg(
        short = 'c',
        long,
        global = true,
        help = "Read kami TOML or Mihomo YAML (.yaml/.yml)"
    )]
    pub config: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Controller URL; overrides file and environment"
    )]
    pub controller: Option<String>,
    #[arg(long, global = true)]
    pub timeout: Option<f64>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true, help = "Show sanitized error context")]
    pub log: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Open the interactive Ratatui workspace
    Tui {
        #[arg(long, help = "Use sample data without network or host effects")]
        demo: bool,
    },
    /// Show core state, selected nodes and current network I/O
    Status,
    /// List proxies and groups
    Proxies {
        #[arg(long, default_value = "")]
        search: String,
    },
    /// Choose a node in a Selector group
    Select { group: String, node: String },
    /// Measure one node's latency
    Delay { node: String },
    /// Change runtime routing mode
    Mode {
        #[arg(value_enum)]
        value: Mode,
    },
    /// Change runtime TUN enable state
    Tun {
        #[arg(value_enum)]
        value: Toggle,
    },
    /// List active connections
    Connections,
    /// Disconnect exactly one connection
    Close { connection_id: String },
    /// List configured proxy providers
    Providers,
    /// Refresh an existing proxy provider
    Update { provider: String },
    /// Follow logs with bounded automatic reconnect
    Logs {
        #[arg(long, value_enum, default_value = "info")]
        level: LogLevel,
    },
    /// Follow live traffic
    Traffic,
    /// Control an existing LOCAL systemd unit
    Service {
        #[arg(value_enum)]
        action: ServiceAction,
        #[arg(long, default_value = "mihomo.service")]
        unit: String,
        #[arg(long)]
        user: bool,
    },
    /// Print POSIX shell proxy exports
    Env {
        #[arg(long)]
        proxy: Option<String>,
        #[arg(long)]
        unset: bool,
    },
    /// Execute a child with proxy variables; use -- before COMMAND
    Exec {
        #[arg(long)]
        proxy: Option<String>,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        argv: Vec<String>,
    },
}
#[derive(Clone, Debug, ValueEnum)]
pub enum Toggle {
    On,
    Off,
}
#[derive(Clone, Debug, ValueEnum)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}
impl LogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        if self.command.is_none() && self.config.is_none() && self.controller.is_none() {
            Cli::command().print_help()?;
            return Ok(());
        }
        let command = self.command.unwrap_or(Command::Tui { demo: false });
        if let Command::Service { action, unit, user } = &command {
            return emit(system::service(action, unit, *user).await?, self.json);
        }
        if matches!(&command, Command::Env { unset: true, .. }) {
            let keys: Vec<_> = system::proxy_environment("http://127.0.0.1:7890")?
                .into_keys()
                .collect();
            return if self.json {
                emit(json!({"unset":keys}), true)
            } else {
                line(&format!("unset {}", keys.join(" ")))
            };
        }
        if let Command::Exec {
            proxy: Some(proxy),
            argv,
        } = &command
        {
            return system::exec(argv, proxy);
        }
        if let Command::Env {
            proxy: Some(proxy), ..
        } = &command
        {
            return emit_env(proxy, self.json);
        }
        let demo = matches!(command, Command::Tui { demo: true });
        let backend = if demo {
            Backend::demo()?
        } else {
            Backend::new(Settings::load(
                self.config,
                self.controller,
                self.timeout,
                matches!(command, Command::Tui { .. }),
            )?)?
        };
        let operation = match command {
            Command::Tui { .. } => {
                ensure!(
                    !self.json,
                    "TUI does not support --json; use a named CLI command"
                );
                ensure!(
                    io::stdin().is_terminal() && io::stdout().is_terminal(),
                    "TUI needs an interactive terminal; try kami status or kami --help"
                );
                return tui::run(backend).await;
            }
            Command::Status => {
                return show_status(&backend, self.json).await;
            }
            Command::Proxies { search } => {
                let raw = backend.get(&["proxies"]).await?;
                let proxies = raw["proxies"]
                    .as_object()
                    .context("Invalid proxies response")?;
                if self.json {
                    let filtered: serde_json::Map<_, _> = proxies
                        .iter()
                        .filter(|(name, _)| name.to_lowercase().contains(&search.to_lowercase()))
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect();
                    return emit(Value::Object(filtered), true);
                }
                let groups: Proxies = serde_json::from_value(Value::Object(proxies.clone()))
                    .context("Invalid proxies response")?;
                let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
                for row in proxy_tree_lines(&groups, &search, color) {
                    line(&row)?;
                }
                return Ok(());
            }
            Command::Connections => return emit(backend.get(&["connections"]).await?, self.json),
            Command::Providers => {
                return emit(backend.get(&["providers", "proxies"]).await?, self.json);
            }
            Command::Select { group, node } => Operation::Select { group, node },
            Command::Delay { node } => Operation::Delay(node),
            Command::Mode { value } => Operation::Mode(value),
            Command::Tun { value } => Operation::Tun(matches!(value, Toggle::On)),
            Command::Close { connection_id } => Operation::Close(connection_id),
            Command::Update { provider } => Operation::UpdateProvider(provider),
            Command::Logs { level } => {
                return follow(backend, Topic::Logs, level.as_str().into(), self.json).await;
            }
            Command::Traffic => {
                return follow(backend, Topic::Traffic, "info".into(), self.json).await;
            }
            Command::Env { .. } => return emit_env(&backend.proxy_url().await?, self.json),
            Command::Exec { argv, .. } => return system::exec(&argv, &backend.proxy_url().await?),
            Command::Service { .. } => unreachable!(),
        };
        emit(backend.execute(&operation).await?, self.json)
    }
}
#[derive(Serialize)]
struct SelectedNode {
    node: String,
    delay_ms: Option<u64>,
}

async fn show_status(backend: &Backend, machine: bool) -> Result<()> {
    let status = backend.status().await?;
    let (selected, selected_error) = match backend.fetch(Topic::Proxies).await {
        Ok(Data::Proxies(proxies)) => (
            proxies
                .iter()
                .filter(|(_, proxy)| proxy.all.is_some() && !proxy.now.is_empty())
                .map(|(group, proxy)| {
                    (
                        group.clone(),
                        SelectedNode {
                            node: proxy.now.clone(),
                            delay_ms: proxy_delay(&proxies, &proxy.now),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            None,
        ),
        Ok(_) => unreachable!(),
        Err(error) => (BTreeMap::new(), Some(clean(&error.to_string()))),
    };
    let (traffic, traffic_error) = match backend.traffic_sample().await {
        Ok(traffic) => (Some(traffic), None),
        Err(error) => (None, Some(clean(&error.to_string()))),
    };
    if machine {
        let mut value = serde_json::to_value(status)?;
        value["selected"] = serde_json::to_value(selected)?;
        value["traffic"] = serde_json::to_value(traffic)?;
        if let Some(error) = selected_error {
            value["selected_error"] = json!(error);
        }
        if let Some(error) = traffic_error {
            value["traffic_error"] = json!(error);
        }
        return emit(value, true);
    }
    emit(serde_json::to_value(status)?, false)?;
    line("selected nodes:")?;
    if let Some(error) = selected_error {
        line(&format!("  unavailable: {error}"))?;
    } else if selected.is_empty() {
        line("  none")?;
    } else {
        let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        for (group, node) in selected {
            let group = latency_color(&clean(&group), node.delay_ms, color);
            let name = latency_color(&clean(&node.node), node.delay_ms, color);
            let delay = node
                .delay_ms
                .map(|value| format!(" ({value} ms)"))
                .unwrap_or_default();
            line(&format!("  {group}: {name}{delay}"))?;
        }
    }
    match traffic {
        Some(traffic) => line(&format!(
            "network I/O: download {}/s, upload {}/s",
            bytes(traffic.down),
            bytes(traffic.up)
        )),
        None => line(&format!(
            "network I/O: unavailable ({})",
            traffic_error.unwrap_or_default()
        )),
    }
}

fn proxy_delay(proxies: &Proxies, name: &str) -> Option<u64> {
    let mut current = name;
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current) {
            return None;
        }
        let proxy = proxies.get(current)?;
        if proxy.all.is_some() && !proxy.now.is_empty() {
            current = &proxy.now;
            continue;
        }
        return proxy
            .history
            .last()
            .map(|entry| entry.delay)
            .filter(|delay| *delay > 0);
    }
}

fn latency_color(text: &str, delay: Option<u64>, enabled: bool) -> String {
    if !enabled {
        return text.into();
    }
    let code = match delay {
        Some(1..=100) => 32,
        Some(101..=300) => 33,
        Some(_) => 31,
        None => 90,
    };
    format!("\u{1b}[{code}m{text}\u{1b}[0m")
}

fn proxy_tree_lines(proxies: &Proxies, search: &str, color: bool) -> Vec<String> {
    let search = search.to_lowercase();
    let groups: Vec<_> = proxies
        .iter()
        .filter_map(|(name, proxy)| {
            let all = proxy.all.as_ref()?;
            let group_matches = name.to_lowercase().contains(&search);
            let nodes: Vec<_> = all
                .iter()
                .filter(|node| group_matches || node.to_lowercase().contains(&search))
                .collect();
            if !search.is_empty() && !group_matches && nodes.is_empty() {
                None
            } else {
                Some((name, proxy, nodes))
            }
        })
        .collect();
    if groups.is_empty() {
        return vec!["No matching proxy groups or nodes.".into()];
    }
    let width = groups
        .iter()
        .flat_map(|(_, _, nodes)| nodes.iter())
        .map(|node| UnicodeWidthStr::width(format!("├── {}", clean(node)).as_str()))
        .max()
        .unwrap_or(0)
        .max(UnicodeWidthStr::width("GROUP / NODE"));
    let mut rows = vec![format!(
        "{}  {:>8}  SELECTED",
        format!(
            "GROUP / NODE{}",
            " ".repeat(width - UnicodeWidthStr::width("GROUP / NODE"))
        ),
        "LATENCY"
    )];
    for (group, proxy, nodes) in groups {
        let delay = proxy_delay(proxies, &proxy.now);
        rows.push(format!("{}:", latency_color(&clean(group), delay, color)));
        for (index, node) in nodes.iter().enumerate() {
            let branch = if index + 1 == nodes.len() {
                "└──"
            } else {
                "├──"
            };
            let name = clean(node);
            let delay = proxy_delay(proxies, node);
            let selected = proxy.now == **node;
            let label = format!("{branch} {name}");
            let padding = " ".repeat(width.saturating_sub(UnicodeWidthStr::width(label.as_str())));
            let name = if selected {
                latency_color(&name, delay, color)
            } else {
                name
            };
            let latency = delay
                .map(|value| format!("{value} ms"))
                .unwrap_or_else(|| "—".into());
            let latency = latency_color(&format!("{latency:>8}"), delay, color);
            let marker = if selected {
                latency_color("●", delay, color)
            } else {
                String::new()
            };
            rows.push(format!("{branch} {name}{padding}  {latency}  {marker}"));
        }
    }
    rows
}
fn emit_env(proxy: &str, machine: bool) -> Result<()> {
    let env = system::proxy_environment(proxy)?;
    if machine {
        return emit(serde_json::to_value(env)?, true);
    }
    for (key, value) in env {
        line(&format!("export {key}={}", system::quote_shell(&value)))?;
    }
    Ok(())
}
fn line(value: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{value}")?;
    stdout.flush()?;
    Ok(())
}
fn emit(value: Value, machine: bool) -> Result<()> {
    if machine {
        return line(&serde_json::to_string(&value)?);
    }
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            line(&clean(&format!(
                "{key}: {}",
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            )))?;
        }
    }
    Ok(())
}
async fn follow(backend: Backend, topic: Topic, level: String, machine: bool) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(128);
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        backend.follow(topic, &level, tx).await;
    });
    while let Some(update) = rx.recv().await {
        match update {
            Update::Data(_, Data::Traffic(value)) => emit(serde_json::to_value(value)?, machine)?,
            Update::Data(_, Data::Log(value)) => emit(serde_json::to_value(value)?, machine)?,
            Update::Error(_, error) => eprintln!("kami: {}; reconnecting", clean(&error)),
            _ => {}
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_tree_shows_group_context_latency_and_selection() {
        let proxies: Proxies = serde_json::from_value(json!({
            "Proxy": {"type":"Selector","all":["Fast","Slow"],"now":"Fast"},
            "Fast": {"type":"Trojan","history":[{"delay":42}]},
            "Slow": {"type":"Trojan","history":[{"delay":350}]}
        }))
        .unwrap();
        let rows = proxy_tree_lines(&proxies, "", false);
        assert!(rows[0].contains("LATENCY  SELECTED"));
        assert_eq!(rows[1], "Proxy:");
        assert!(rows[2].contains("├── Fast"));
        assert!(rows[2].contains("42 ms"));
        assert!(rows[2].contains("●"));
        assert!(rows[3].contains("└── Slow"));
        assert!(rows[3].contains("350 ms"));
        assert!(!rows[3].contains("●"));
        let filtered = proxy_tree_lines(&proxies, "slow", false).join("\n");
        assert!(filtered.contains("Proxy:"));
        assert!(filtered.contains("Slow"));
        assert!(!filtered.contains("Fast"));
        let colored = proxy_tree_lines(&proxies, "", true).join("\n");
        assert!(colored.contains("\u{1b}[32mProxy\u{1b}[0m:"));
        assert!(colored.contains("\u{1b}[32mFast\u{1b}[0m"));
        assert!(colored.contains("\u{1b}[31m  350 ms\u{1b}[0m"));
    }

    #[test]
    fn parser_preserves_exec_arguments_and_global_options() {
        let cli = Cli::try_parse_from([
            "kami",
            "exec",
            "--proxy",
            "http://localhost:7890",
            "--",
            "printf",
            "$(touch nope)",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Command::Exec { argv, .. } => assert_eq!(argv, ["printf", "$(touch nope)"]),
            _ => panic!("wrong command"),
        }
        assert!(
            Cli::try_parse_from(["kami", "status", "--json"])
                .unwrap()
                .json
        );
        assert!(Cli::try_parse_from(["kami", "--secret", "hidden"]).is_err());
        let cli = Cli::try_parse_from(["kami", "status", "-c", "core.yaml"]).unwrap();
        assert_eq!(cli.config, Some(PathBuf::from("core.yaml")));
        assert!(Cli::try_parse_from(["kami", "--mihomo-config", "core.yaml"]).is_err());
    }
}
