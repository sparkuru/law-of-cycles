use crate::{backend::Backend, config::Settings, model::*, system, tui};
use anyhow::{Context, Result, ensure};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(
    name = "kami",
    version,
    about = "law-of-cycles — a terminal workspace for Mihomo"
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Read kami TOML or Mihomo YAML (.yaml/.yml)"
    )]
    pub config: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        conflicts_with = "config",
        help = "Read connection settings from a Mihomo YAML file"
    )]
    pub mihomo_config: Option<PathBuf>,
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
    /// Show core version, routing mode and TUN state
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
        if self.command.is_none()
            && self.config.is_none()
            && self.mihomo_config.is_none()
            && self.controller.is_none()
        {
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
                self.mihomo_config,
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
                return emit(serde_json::to_value(backend.status().await?)?, self.json);
            }
            Command::Proxies { search } => {
                let raw = backend.get(&["proxies"]).await?;
                let proxies = raw["proxies"]
                    .as_object()
                    .context("Invalid proxies response")?;
                let filtered: serde_json::Map<_, _> = proxies
                    .iter()
                    .filter(|(name, _)| name.to_lowercase().contains(&search.to_lowercase()))
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect();
                return emit(Value::Object(filtered), self.json);
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
        let cli = Cli::try_parse_from(["kami", "status", "--mihomo-config", "core.conf"]).unwrap();
        assert_eq!(cli.mihomo_config, Some(PathBuf::from("core.conf")));
        assert!(
            Cli::try_parse_from([
                "kami",
                "--config",
                "kami.toml",
                "--mihomo-config",
                "core.yaml"
            ])
            .is_err()
        );
    }
}
