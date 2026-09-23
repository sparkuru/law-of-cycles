use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Rule,
    Global,
    Direct,
}
impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Status {
    pub version: String,
    pub mode: String,
    pub tun: bool,
    #[serde(rename = "mixed-port", default)]
    pub mixed_port: u16,
    #[serde(default)]
    pub port: u16,
    #[serde(rename = "socks-port", default)]
    pub socks_port: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Delay {
    #[serde(default)]
    pub delay: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Proxy {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub all: Option<Vec<String>>,
    #[serde(default)]
    pub now: String,
    #[serde(default)]
    pub history: Vec<Delay>,
    #[serde(default)]
    pub udp: bool,
}
pub type Proxies = BTreeMap<String, Proxy>;
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub destination_ip: String,
    #[serde(default)]
    pub destination_port: String,
    #[serde(default)]
    pub source_ip: String,
    #[serde(default)]
    pub process: String,
    #[serde(default)]
    pub network: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub rule_payload: String,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Connections {
    #[serde(default)]
    pub connections: Option<Vec<Connection>>,
    #[serde(default)]
    pub upload_total: u64,
    #[serde(default)]
    pub download_total: u64,
}
impl Connections {
    pub fn items(&self) -> &[Connection] {
        self.connections.as_deref().unwrap_or_default()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Traffic {
    pub up: u64,
    pub down: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Log {
    #[serde(rename = "type")]
    pub level: String,
    pub payload: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Topic {
    Status,
    Proxies,
    Connections,
    Traffic,
    Logs,
}
impl Topic {
    pub fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Proxies => "proxies",
            Self::Connections => "connections",
            Self::Traffic => "traffic",
            Self::Logs => "logs",
        }
    }
}
#[derive(Clone, Debug)]
pub enum Data {
    Status(Status),
    Proxies(Proxies),
    Connections(Connections),
    Traffic(Traffic),
    Log(Log),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ServiceAction {
    Status,
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}
impl ServiceAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Enable => "enable",
            Self::Disable => "disable",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Select {
        group: String,
        node: String,
    },
    Delay(String),
    RefreshGroup(String),
    Mode(Mode),
    Tun(bool),
    Close(String),
    UpdateProvider(String),
    Service {
        action: ServiceAction,
        unit: String,
        user: bool,
    },
}
impl Operation {
    pub fn label(&self) -> String {
        match self {
            Self::Select { group, node } => format!("Use {node} in {group}"),
            Self::Delay(node) => format!("Measure {node}"),
            Self::RefreshGroup(group) => format!("Refresh group {group}"),
            Self::Mode(mode) => format!("Set runtime mode {}", mode.as_str()),
            Self::Tun(enabled) => {
                format!("Turn runtime TUN {}", if *enabled { "on" } else { "off" })
            }
            Self::Close(id) => format!("Disconnect {id}"),
            Self::UpdateProvider(name) => format!("Update {name}"),
            Self::Service { action, unit, .. } => format!("{} LOCAL {unit}", action.as_str()),
        }
    }
    pub fn needs_confirmation(&self) -> bool {
        matches!(self, Self::Tun(_) | Self::Close(_))
            || matches!(self, Self::Service { action, .. } if *action != ServiceAction::Status)
    }
}
#[derive(Debug)]
pub enum Update {
    GroupProgress { completed: usize, total: usize },
    Latency { node: String, delay: Option<u64> },
    Data(Topic, Data),
    Error(Topic, String),
    Finished(String, Result<serde_json::Value, String>),
}

pub fn clean(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}
pub fn bytes(value: u64) -> String {
    let mut size = value as f64;
    for unit in ["B", "KiB", "MiB", "GiB", "TiB"] {
        if size < 1024.0 || unit == "TiB" {
            return format!("{size:.1} {unit}");
        }
        size /= 1024.0;
    }
    unreachable!()
}
