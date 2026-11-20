use crate::{backend::Backend, model::*};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
    task::JoinSet,
    time::{Instant, timeout_at},
};
use url::{Host, Url};

const MAX_PROXY_HEADERS: usize = 16 * 1024;
const MAX_LOGS: usize = 32;

#[derive(Serialize)]
pub struct MatchedRule {
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: String,
}

#[derive(Serialize)]
pub struct Report {
    pub target: String,
    pub port: u16,
    pub network: &'static str,
    pub proxy: String,
    pub mode_at_start: String,
    pub proxy_connect: &'static str,
    pub observation: &'static str,
    pub connection_id: Option<String>,
    pub rule: Option<MatchedRule>,
    pub route: Option<Vec<String>>,
    pub logs: Vec<Log>,
    pub log_error: Option<String>,
    pub connection_error: Option<String>,
}

impl Report {
    pub fn route_observed(&self) -> bool {
        self.connection_id.is_some() && self.route.is_some()
    }

    pub fn rejected(&self) -> bool {
        self.route.as_ref().is_some_and(|route| {
            route.iter().any(|part| {
                part.eq_ignore_ascii_case("REJECT") || part.eq_ignore_ascii_case("REJECT-DROP")
            })
        })
    }
}

struct Target {
    host: Host<String>,
    port: u16,
}

impl Target {
    fn parse(input: &str, port: u16) -> Result<Self> {
        ensure!(port > 0, "Destination port must be between 1 and 65535");
        ensure!(
            !input.is_empty()
                && input.len() <= 253
                && !input
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control()),
            "Target must be a domain name or IP address"
        );
        let host = match input.parse::<IpAddr>() {
            Ok(IpAddr::V4(ip)) => Host::Ipv4(ip),
            Ok(IpAddr::V6(ip)) => Host::Ipv6(ip),
            Err(_) => Host::parse(input)
                .map_err(|_| anyhow::anyhow!("Target must be a domain name or IP address"))?,
        };
        ensure!(
            !matches!(&host, Host::Domain(name) if name.is_empty()),
            "Target must be a domain name or IP address"
        );
        Ok(Self { host, port })
    }

    fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn matches_connection(&self, connection: &Connection, source: SocketAddr) -> bool {
        let metadata = &connection.metadata;
        if connection.id.is_empty()
            || !metadata.network.eq_ignore_ascii_case("tcp")
            || metadata.source_ip.parse::<IpAddr>().ok().map(normalize_ip)
                != Some(normalize_ip(source.ip()))
            || metadata.source_port.parse::<u16>().ok() != Some(source.port())
            || metadata.destination_port.parse::<u16>().ok() != Some(self.port)
        {
            return false;
        }
        match &self.host {
            Host::Domain(host) => metadata.host.eq_ignore_ascii_case(host),
            Host::Ipv4(ip) => {
                metadata
                    .destination_ip
                    .parse::<IpAddr>()
                    .ok()
                    .map(normalize_ip)
                    == Some(IpAddr::V4(*ip))
            }
            Host::Ipv6(ip) => {
                metadata
                    .destination_ip
                    .parse::<IpAddr>()
                    .ok()
                    .map(normalize_ip)
                    == Some(normalize_ip(IpAddr::V6(*ip)))
            }
        }
    }

    fn matches_log(&self, log: &Log, source: SocketAddr) -> bool {
        let Some((left, right)) = log.payload.split_once(" --> ") else {
            return false;
        };
        if !left.starts_with("[TCP] ")
            || !right
                .split_whitespace()
                .next()
                .is_some_and(|value| value.eq_ignore_ascii_case(&self.authority()))
        {
            return false;
        }
        left.match_indices(' ').any(|(index, _)| {
            let candidate = &left[index + 1..];
            let (address, detail) = match candidate.split_once('(') {
                Some((address, detail)) if detail.ends_with(')') => (address, true),
                Some(_) => ("", false),
                None => (candidate, true),
            };
            detail
                && address.parse::<SocketAddr>().ok().is_some_and(|address| {
                    normalize_ip(address.ip()) == normalize_ip(source.ip())
                        && address.port() == source.port()
                })
        })
    }
}

fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

fn proxy_origin(backend: &Backend, config: &Value, explicit: Option<&str>) -> Result<Url> {
    let default;
    let input = match explicit {
        Some(input) => input,
        None => {
            let port = ["mixed-port", "port"]
                .into_iter()
                .filter_map(|key| config.get(key).and_then(Value::as_u64))
                .find(|port| (1..=u16::MAX as u64).contains(port))
                .context("No mixed or HTTP proxy listener; pass --proxy http://HOST:PORT")?;
            let controller = backend.url(&[]);
            let host = controller.host().context("Missing controller host")?;
            default = format!("http://{host}:{port}");
            &default
        }
    };
    ensure!(
        !input
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace()),
        "Invalid proxy URL"
    );
    let url = Url::parse(input).map_err(|_| anyhow::anyhow!("Invalid proxy URL"))?;
    ensure!(
        url.scheme() == "http"
            && url.host().is_some()
            && url.port_or_known_default().is_some_and(|port| port > 0)
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none(),
        "Ping requires an HTTP proxy origin without credentials or a path"
    );
    Ok(url)
}

async fn connect_proxy(proxy: &Url, target: &Target) -> Result<TcpStream> {
    let host = proxy.host().context("Missing proxy host")?;
    let port = proxy
        .port_or_known_default()
        .context("Missing proxy port")?;
    let connection = match host {
        Host::Domain(host) => TcpStream::connect((host, port)).await,
        Host::Ipv4(ip) => TcpStream::connect(SocketAddr::new(IpAddr::V4(ip), port)).await,
        Host::Ipv6(ip) => TcpStream::connect(SocketAddr::new(IpAddr::V6(ip), port)).await,
    };
    let mut socket = connection.context("Cannot connect to the HTTP proxy listener")?;
    let authority = target.authority();
    socket
        .write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .context("Cannot send proxy CONNECT request")?;
    let mut response = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = socket
            .read(&mut buffer)
            .await
            .context("Cannot read proxy CONNECT response")?;
        ensure!(count > 0, "Proxy closed before its CONNECT response");
        response.extend_from_slice(&buffer[..count]);
        ensure!(
            response.len() <= MAX_PROXY_HEADERS,
            "Proxy CONNECT response exceeded 16 KiB"
        );
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let line = response
        .split(|byte| *byte == b'\n')
        .next()
        .context("Invalid proxy CONNECT response")?;
    let line = std::str::from_utf8(line).context("Invalid proxy CONNECT response")?;
    let mut fields = line.split_whitespace();
    let version = fields.next().unwrap_or_default();
    let status = fields
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .context("Invalid proxy CONNECT response")?;
    ensure!(
        matches!(version, "HTTP/1.0" | "HTTP/1.1"),
        "Invalid proxy CONNECT response"
    );
    ensure!(
        (200..300).contains(&status),
        "Proxy refused CONNECT (HTTP {status})"
    );
    Ok(socket)
}

pub async fn probe(
    backend: &Backend,
    input: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<Report> {
    let target = Target::parse(input, port)?;
    let deadline = Instant::now() + backend.timeout();
    let mut tasks = JoinSet::new();
    let (log_tx, mut log_rx) = mpsc::channel(128);
    let log_backend = backend.clone();
    tasks.spawn(async move {
        let mut backoff = 1;
        if let Err(error) = log_backend
            .stream_once(Topic::Logs, "info", &log_tx, &mut backoff)
            .await
        {
            let _ = log_tx
                .send(Update::Error(Topic::Logs, error.to_string()))
                .await;
        }
    });
    let config = timeout_at(deadline, backend.get(&["configs"]))
        .await
        .context("Probe timed out while reading core configuration")??;
    let mode_at_start = config
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let proxy = proxy_origin(backend, &config, proxy)?;
    let mut socket = timeout_at(deadline, connect_proxy(&proxy, &target))
        .await
        .context("Probe timed out connecting to the proxy")??;
    let source = socket
        .local_addr()
        .context("Cannot determine probe source address")?;
    let (connection_tx, mut connection_rx) = mpsc::channel(1);
    let connection_backend = backend.clone();
    tasks.spawn(async move {
        loop {
            let result = connection_backend
                .fetch(Topic::Connections)
                .await
                .and_then(|data| match data {
                    Data::Connections(connections) => Ok(connections),
                    _ => bail!("Unexpected connections response"),
                });
            let failed = result.is_err();
            if connection_tx
                .send(result.map_err(|error| error.to_string()))
                .await
                .is_err()
                || failed
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let mut connection = None;
    let mut logs = Vec::new();
    let mut log_error = None;
    let mut connection_error = None;
    let mut socket_closed = false;
    let mut logs_open = true;
    let mut connections_open = true;
    let mut byte = [0];
    loop {
        if (connection.is_some() && (!logs.is_empty() || !logs_open))
            || ((socket_closed || !connections_open) && !logs.is_empty())
            || (!connections_open && !logs_open)
        {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            update = log_rx.recv(), if logs_open => match update {
                Some(Update::Data(Topic::Logs, Data::Log(log))) if target.matches_log(&log, source) => {
                    if logs.len() < MAX_LOGS {
                        logs.push(log);
                    }
                }
                Some(Update::Error(Topic::Logs, error)) => {
                    log_error = Some(error);
                    logs_open = false;
                }
                Some(_) => {}
                None => logs_open = false,
            },
            snapshot = connection_rx.recv(), if connections_open && connection.is_none() => match snapshot {
                Some(Ok(snapshot)) => {
                    connection = snapshot.connections.unwrap_or_default().into_iter().find(|item| target.matches_connection(item, source));
                }
                Some(Err(error)) => {
                    connection_error = Some(error);
                    connections_open = false;
                }
                None => connections_open = false,
            },
            result = socket.read(&mut byte), if !socket_closed => {
                if !matches!(result, Ok(count) if count > 0) {
                    socket_closed = true;
                }
            }
        }
    }

    let (connection_id, rule, route) = match connection {
        Some(connection) => {
            let rule = (!connection.rule.is_empty()).then_some(MatchedRule {
                kind: connection.rule,
                payload: connection.rule_payload,
            });
            let route = (!connection.chains.is_empty())
                .then(|| connection.chains.into_iter().rev().collect());
            (Some(connection.id), rule, route)
        }
        None => (None, None, None),
    };
    Ok(Report {
        target: target.host.to_string(),
        port,
        network: "tcp",
        proxy: proxy.to_string(),
        mode_at_start,
        proxy_connect: "accepted",
        observation: if connection_id.is_some() {
            "observed"
        } else {
            "unavailable"
        },
        connection_id,
        rule,
        route,
        logs,
        log_error,
        connection_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use serde_json::json;

    #[test]
    fn target_validation_prevents_connect_request_injection() {
        for value in [
            "",
            "example.com:443",
            "https://example.com",
            "example.com/path",
            "example.com\r\nX-Injected: yes",
            "user@example.com",
        ] {
            assert!(Target::parse(value, 443).is_err(), "{value:?}");
        }
        assert_eq!(
            Target::parse("EXAMPLE.com", 443).unwrap().authority(),
            "example.com:443"
        );
        assert_eq!(
            Target::parse("2001:db8::1", 8443).unwrap().authority(),
            "[2001:db8::1]:8443"
        );
        assert!(Target::parse("example.com", 0).is_err());
    }

    #[test]
    fn connection_and_log_correlation_requires_the_complete_tuple() {
        let target = Target::parse("example.com", 443).unwrap();
        let source: SocketAddr = "127.0.0.1:4567".parse().unwrap();
        let connection: Connection = serde_json::from_value(json!({
            "id":"probe",
            "metadata": {
                "network":"tcp", "host":"example.com", "destinationPort":"443",
                "sourceIP":"::ffff:127.0.0.1", "sourcePort":"4567"
            }
        }))
        .unwrap();
        assert!(target.matches_connection(&connection, source));
        let mut wrong = connection.clone();
        wrong.metadata.source_port = "4568".into();
        assert!(!target.matches_connection(&wrong, source));
        wrong = connection.clone();
        wrong.metadata.host = "other.example".into();
        assert!(!target.matches_connection(&wrong, source));
        wrong = connection.clone();
        wrong.metadata.network = "udp".into();
        assert!(!target.matches_connection(&wrong, source));

        let log = |payload: &str| Log {
            level: "info".into(),
            payload: payload.into(),
        };
        assert!(target.matches_log(
            &log(
                "[TCP] 127.0.0.1:4567(kami, uid=1000) --> example.com:443 match Match using DIRECT"
            ),
            source
        ));
        assert!(target.matches_log(&log("[TCP] dial Proxy (match DomainSuffix/example.com) 127.0.0.1:4567 --> example.com:443 error: failed"), source));
        assert!(!target.matches_log(
            &log("[TCP] 127.0.0.1:4568 --> example.com:443 match Match using DIRECT"),
            source
        ));
        assert!(!target.matches_log(
            &log("[TCP] 127.0.0.1:4567 --> other.example:443 match Match using DIRECT"),
            source
        ));
        assert!(!target.matches_log(
            &log("[UDP] 127.0.0.1:4567 --> example.com:443 match Match using DIRECT"),
            source
        ));
    }

    #[test]
    fn proxy_discovery_validates_origin_and_ipv6() {
        let backend =
            Backend::new(Settings::new("http://[::1]:9090", String::new(), 1.0).unwrap()).unwrap();
        assert_eq!(
            proxy_origin(&backend, &json!({"mixed-port":7890}), None)
                .unwrap()
                .as_str(),
            "http://[::1]:7890/"
        );
        assert_eq!(
            proxy_origin(&backend, &json!({"mixed-port":0,"port":8080}), None)
                .unwrap()
                .as_str(),
            "http://[::1]:8080/"
        );
        for invalid in [
            "http://user:password@localhost:7890",
            "https://localhost:7890",
            "socks5h://localhost:7890",
            "http://localhost:7890/path",
        ] {
            assert!(proxy_origin(&backend, &json!({}), Some(invalid)).is_err());
        }
        assert!(proxy_origin(&backend, &json!({"socks-port":7890}), None).is_err());
    }
}
