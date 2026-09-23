use crate::{config::Settings, model::*, system};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream};
use reqwest::{
    Method,
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use url::Url;

#[derive(Clone)]
pub struct Backend {
    settings: Settings,
    http: reqwest::Client,
    demo: Option<Arc<Mutex<Demo>>>,
}
struct Demo {
    config: Value,
    proxies: Value,
    connections: Value,
}
impl Backend {
    pub fn new(settings: Settings) -> Result<Self> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(settings.timeout)
            .user_agent("kami/0.1.0")
            .build()
            .context("Cannot initialize HTTP client")?;
        Ok(Self {
            settings,
            http,
            demo: None,
        })
    }
    pub fn demo() -> Result<Self> {
        let mut backend = Self::new(Settings::new("http://demo.invalid", String::new(), 5.0)?)?;
        backend.demo = Some(Arc::new(Mutex::new(Demo {
            config: json!({"mode":"rule", "tun":{"enable":false}, "mixed-port":7890}),
            proxies: json!({
                "Proxy":{"type":"Selector","all":["Tokyo 01","Singapore 01","DIRECT"],"now":"Tokyo 01"},
                "Media":{"type":"Selector","all":["Singapore 01","Tokyo 01"],"now":"Singapore 01"},
                "Auto":{"type":"URLTest","all":["Tokyo 01","Singapore 01"],"now":"Tokyo 01"},
                "Tokyo 01":{"type":"Shadowsocks","udp":true,"history":[{"delay":38}]},
                "Singapore 01":{"type":"Trojan","udp":true,"history":[{"delay":72}]},
                "DIRECT":{"type":"Direct","udp":true,"history":[]}
            }),
            connections: json!({"uploadTotal":14700,"downloadTotal":3523400,"connections":[
                {"id":"demo-browser","metadata":{"host":"example.com","destinationPort":"443","sourceIP":"127.0.0.1","network":"tcp","process":"firefox"},"chains":["Tokyo 01","Proxy"],"rule":"DomainSuffix","rulePayload":"example.com","upload":12400,"download":3400000},
                {"id":"demo-terminal","metadata":{"host":"packages.example.org","destinationPort":"443","sourceIP":"127.0.0.1","network":"tcp","process":"curl"},"chains":["Singapore 01","Media"],"rule":"Match","upload":2300,"download":123400}
            ]}),
        })));
        Ok(backend)
    }
    pub fn is_demo(&self) -> bool {
        self.demo.is_some()
    }
    pub fn show_process(&self) -> bool {
        self.settings.connections.show_process
    }
    pub async fn refresh_group(
        &self,
        group: &str,
        tx: Option<&mpsc::Sender<Update>>,
    ) -> Result<Value> {
        let proxies = self.get(&["proxies"]).await?;
        let members = proxies["proxies"][group]["all"]
            .as_array()
            .context("Selected proxy is not a group")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("Invalid group member")
            })
            .collect::<Result<std::collections::BTreeSet<_>>>()?;
        let total = members.len();
        if let Some(tx) = tx {
            let _ = tx
                .send(Update::GroupProgress {
                    completed: 0,
                    total,
                })
                .await;
        }
        let mut pending = stream::iter(members.into_iter().map(|node| async move {
            let result = self
                .request(
                    Method::GET,
                    &["proxies", &node, "delay"],
                    None,
                    &[
                        ("url", "https://www.gstatic.com/generate_204"),
                        ("timeout", "5000"),
                    ],
                )
                .await;
            let delay = result
                .ok()
                .and_then(|value| value["delay"].as_u64())
                .filter(|delay| *delay > 0);
            (node, delay)
        }))
        .buffer_unordered(4);
        let mut completed = 0;
        let mut failed = 0;
        while let Some((node, delay)) = pending.next().await {
            completed += 1;
            failed += usize::from(delay.is_none());
            if let Some(tx) = tx {
                let _ = tx.send(Update::Latency { node, delay }).await;
                let _ = tx.send(Update::GroupProgress { completed, total }).await;
            }
        }
        Ok(json!({"group":group,"total":total,"succeeded":total - failed,"failed":failed}))
    }
    pub fn endpoint(&self) -> String {
        if self.is_demo() {
            "Demo controller".into()
        } else {
            self.settings.controller.to_string()
        }
    }
    pub fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.settings.controller.clone();
        url.path_segments_mut()
            .expect("validated HTTP origin")
            .clear()
            .extend(segments);
        url
    }
    fn builder(&self, method: Method, segments: &[&str]) -> Result<reqwest::RequestBuilder> {
        let mut request = self.http.request(method, self.url(segments));
        if !self.settings.secret.is_empty() {
            let mut secret = HeaderValue::from_str(&format!("Bearer {}", self.settings.secret))
                .context("Invalid controller credential")?;
            secret.set_sensitive(true);
            request = request.header(AUTHORIZATION, secret);
        }
        Ok(request)
    }
    pub async fn request(
        &self,
        method: Method,
        path: &[&str],
        body: Option<Value>,
        query: &[(&str, &str)],
    ) -> Result<Value> {
        if let Some(demo) = &self.demo {
            return demo
                .lock()
                .map_err(|_| anyhow::anyhow!("Demo state unavailable"))?
                .request(method, path, body);
        }
        let timeout = if path.last() == Some(&"delay") {
            self.settings.timeout.max(Duration::from_secs(7))
        } else {
            self.settings.timeout
        };
        let mut request = self.builder(method, path)?.timeout(timeout).query(query);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .context("Controller unavailable; check address, TLS and core status")?;
        check_status(response.status())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("Controller response interrupted or timed out")?
        {
            ensure!(
                bytes.len() + chunk.len() <= 16 * 1024 * 1024,
                "Controller response exceeded 16 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(json!({}));
        }
        let value: Value =
            serde_json::from_slice(&bytes).context("Controller returned invalid JSON")?;
        ensure!(
            value.is_object(),
            "Controller response must be a JSON object"
        );
        Ok(value)
    }
    pub async fn get(&self, path: &[&str]) -> Result<Value> {
        self.request(Method::GET, path, None, &[]).await
    }
    pub async fn status(&self) -> Result<Status> {
        let version = self.get(&["version"]).await?;
        let config = self.get(&["configs"]).await?;
        ensure!(
            config.get("tun").is_none_or(Value::is_object),
            "Invalid core TUN configuration"
        );
        serde_json::from_value(json!({
            "version": version.get("version").cloned().unwrap_or(json!("unknown")),
            "mode": config.get("mode").cloned().unwrap_or(json!("unknown")),
            "tun": config.pointer("/tun/enable").cloned().unwrap_or(json!(false)),
            "mixed-port": config.get("mixed-port").cloned().unwrap_or(json!(0)),
            "port": config.get("port").cloned().unwrap_or(json!(0)),
            "socks-port": config.get("socks-port").cloned().unwrap_or(json!(0)),
        }))
        .context("Invalid core configuration response")
    }
    pub async fn fetch(&self, topic: Topic) -> Result<Data> {
        Ok(match topic {
            Topic::Status => Data::Status(self.status().await?),
            Topic::Proxies => Data::Proxies(
                serde_json::from_value(self.get(&["proxies"]).await?["proxies"].clone())
                    .context("Invalid proxies response")?,
            ),
            Topic::Connections => Data::Connections(
                serde_json::from_value(self.get(&["connections"]).await?)
                    .context("Invalid connections response")?,
            ),
            _ => bail!("This endpoint is a stream"),
        })
    }
    pub async fn execute(&self, operation: &Operation) -> Result<Value> {
        match operation {
            Operation::RefreshGroup(group) => self.refresh_group(group, None).await,
            Operation::Select { group, node } => {
                let proxies = self.get(&["proxies"]).await?;
                let entry = &proxies["proxies"][group];
                ensure!(
                    entry["type"] == "Selector",
                    "Only Selector groups support manual selection"
                );
                ensure!(
                    entry["all"]
                        .as_array()
                        .is_some_and(|all| all.iter().any(|item| item.as_str() == Some(node))),
                    "Node is not a member of the selected group"
                );
                self.request(
                    Method::PUT,
                    &["proxies", group],
                    Some(json!({"name":node})),
                    &[],
                )
                .await?;
                Ok(json!({"group":group,"selected":node}))
            }
            Operation::Delay(node) => {
                self.request(
                    Method::GET,
                    &["proxies", node, "delay"],
                    None,
                    &[
                        ("url", "https://www.gstatic.com/generate_204"),
                        ("timeout", "5000"),
                    ],
                )
                .await
            }
            Operation::Mode(mode) => {
                self.request(
                    Method::PATCH,
                    &["configs"],
                    Some(json!({"mode":mode.as_str()})),
                    &[],
                )
                .await?;
                ensure!(
                    self.get(&["configs"]).await?["mode"] == mode.as_str(),
                    "Mode update was not reflected by the core"
                );
                Ok(json!({"mode":mode.as_str(),"scope":"runtime"}))
            }
            Operation::Tun(enabled) => {
                self.request(
                    Method::PATCH,
                    &["configs"],
                    Some(json!({"tun":{"enable":enabled}})),
                    &[],
                )
                .await?;
                ensure!(
                    self.get(&["configs"])
                        .await?
                        .pointer("/tun/enable")
                        .and_then(Value::as_bool)
                        == Some(*enabled),
                    "TUN update was not reflected by the core; inspect core logs"
                );
                Ok(json!({"tun":enabled,"scope":"runtime","network-verified":false}))
            }
            Operation::Close(id) => {
                ensure!(!id.is_empty(), "Connection ID must not be empty");
                self.request(Method::DELETE, &["connections", id], None, &[])
                    .await?;
                Ok(json!({"closed":id}))
            }
            Operation::UpdateProvider(name) => {
                ensure!(!name.is_empty(), "Provider name must not be empty");
                self.request(Method::PUT, &["providers", "proxies", name], None, &[])
                    .await?;
                Ok(json!({"updated":name}))
            }
            Operation::Service { action, unit, user } => {
                ensure!(
                    !self.is_demo(),
                    "Service operations are disabled in demo mode"
                );
                system::service(action, unit, *user).await
            }
        }
    }
    pub async fn proxy_url(&self) -> Result<String> {
        let status = self.status().await?;
        let (scheme, port) = if status.mixed_port > 0 {
            ("http", status.mixed_port)
        } else if status.port > 0 {
            ("http", status.port)
        } else {
            ("socks5h", status.socks_port)
        };
        ensure!(
            port > 0,
            "No usable proxy listener; pass --proxy or configure a core port"
        );
        let host = self
            .settings
            .controller
            .host()
            .context("Missing controller host")?;
        Ok(format!("{scheme}://{host}:{port}"))
    }
    pub async fn follow(&self, topic: Topic, level: &str, tx: mpsc::Sender<Update>) {
        let mut backoff = 1;
        loop {
            let result = self.stream_once(topic, level, &tx, &mut backoff).await;
            if tx.is_closed() {
                return;
            }
            if let Err(error) = result
                && tx
                    .send(Update::Error(topic, error.to_string()))
                    .await
                    .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(15);
        }
    }
    async fn stream_once(
        &self,
        topic: Topic,
        level: &str,
        tx: &mpsc::Sender<Update>,
        backoff: &mut u64,
    ) -> Result<()> {
        if self.is_demo() {
            let mut tick = 0;
            loop {
                tick += 1;
                let data = match topic {
                    Topic::Traffic => Data::Traffic(Traffic {
                        up: 12000 + tick % 7 * 800,
                        down: 250000 + tick % 11 * 12000,
                    }),
                    _ => Data::Log(Log {
                        level: "info".into(),
                        payload: format!(
                            "Demo #{tick}: firefox -> example.com:443 via Proxy / Tokyo 01"
                        ),
                    }),
                };
                if tx.send(Update::Data(topic, data)).await.is_err() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        let mut response = tokio::time::timeout(
            self.settings.timeout,
            self.builder(Method::GET, &[topic.name()])?
                .query(&[("level", level)])
                .send(),
        )
        .await
        .context("Controller stream connection timed out")?
        .context("Controller stream unavailable")?;
        check_status(response.status())?;
        let mut pending = Vec::new();
        loop {
            let chunk = tokio::time::timeout(
                self.settings.timeout.max(Duration::from_secs(15)),
                response.chunk(),
            )
            .await
            .context("Controller stream idle or interrupted")?
            .context("Controller stream interrupted")?;
            let Some(chunk) = chunk else {
                bail!("Controller stream closed");
            };
            for byte in chunk {
                if byte == b'\n' {
                    if pending.iter().any(|byte: &u8| !byte.is_ascii_whitespace()) {
                        let data = match topic {
                            Topic::Logs => Data::Log(
                                serde_json::from_slice(&pending)
                                    .context("Invalid log stream data")?,
                            ),
                            Topic::Traffic => Data::Traffic(
                                serde_json::from_slice(&pending)
                                    .context("Invalid traffic stream data")?,
                            ),
                            _ => bail!("Unsupported stream"),
                        };
                        if tx.send(Update::Data(topic, data)).await.is_err() {
                            return Ok(());
                        }
                        *backoff = 1;
                    }
                    pending.clear();
                } else {
                    ensure!(
                        pending.len() < 64 * 1024,
                        "Controller stream line exceeded 64 KiB"
                    );
                    pending.push(byte);
                }
            }
        }
    }
}
fn check_status(status: reqwest::StatusCode) -> Result<()> {
    if status.is_success() {
        return Ok(());
    }
    let hint = if matches!(status.as_u16(), 401 | 403) {
        " (check KAMI_SECRET)"
    } else {
        ""
    };
    bail!("Controller returned HTTP {}{hint}", status.as_u16())
}
impl Demo {
    fn request(&mut self, method: Method, path: &[&str], body: Option<Value>) -> Result<Value> {
        match path {
            ["version"] => Ok(json!({"version":"demo-core"})),
            ["configs"] => {
                if method == Method::PATCH {
                    let body = body.context("Missing demo request")?;
                    for (key, value) in body.as_object().context("Invalid demo request")? {
                        self.config[key] = value.clone();
                    }
                }
                Ok(self.config.clone())
            }
            ["proxies"] => Ok(json!({"proxies":self.proxies})),
            ["proxies", name] if method == Method::PUT => {
                self.proxies[*name]["now"] = body.context("Missing node")?["name"].clone();
                Ok(json!({}))
            }
            ["proxies", name, "delay"] => {
                ensure!(self.proxies.get(*name).is_some(), "Unknown demo proxy");
                self.proxies[*name]["history"] = json!([{"delay":38}]);
                Ok(json!({"delay":38}))
            }
            ["connections"] => Ok(self.connections.clone()),
            ["connections", id] if method == Method::DELETE => {
                if let Some(items) = self.connections["connections"].as_array_mut() {
                    items.retain(|item| item["id"] != *id);
                }
                Ok(json!({}))
            }
            ["providers", "proxies"] => Ok(json!({"providers":{}})),
            _ => bail!("This operation is unavailable in demo mode"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn group_refresh_reports_partial_failure_without_switching() {
        let backend = Backend::demo().unwrap();
        backend.demo.as_ref().unwrap().lock().unwrap().proxies["Proxy"]["all"] =
            json!(["Tokyo 01", "missing", "Tokyo 01"]);
        let (tx, mut rx) = mpsc::channel(16);
        let result = backend.refresh_group("Proxy", Some(&tx)).await.unwrap();
        assert_eq!(result["total"], 2);
        assert_eq!(result["succeeded"], 1);
        assert_eq!(result["failed"], 1);
        assert_eq!(
            backend.get(&["proxies"]).await.unwrap()["proxies"]["Proxy"]["now"],
            "Tokyo 01"
        );
        let mut failed_node = false;
        let mut completed = false;
        while let Ok(update) = rx.try_recv() {
            match update {
                Update::Latency { node, delay: None } if node == "missing" => failed_node = true,
                Update::GroupProgress {
                    completed: 2,
                    total: 2,
                } => completed = true,
                _ => {}
            }
        }
        assert!(failed_node && completed);
        assert!(backend.refresh_group("Tokyo 01", None).await.is_err());
    }
    #[tokio::test]
    async fn demo_operations_validate_membership_and_read_back() {
        let backend = Backend::demo().unwrap();
        assert!(
            backend
                .execute(&Operation::Select {
                    group: "Auto".into(),
                    node: "Tokyo 01".into()
                })
                .await
                .is_err()
        );
        assert!(
            backend
                .execute(&Operation::Select {
                    group: "Proxy".into(),
                    node: "missing".into()
                })
                .await
                .is_err()
        );
        backend
            .execute(&Operation::Select {
                group: "Proxy".into(),
                node: "Singapore 01".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            backend.get(&["proxies"]).await.unwrap()["proxies"]["Proxy"]["now"],
            "Singapore 01"
        );
        backend
            .execute(&Operation::Mode(Mode::Global))
            .await
            .unwrap();
        backend.execute(&Operation::Tun(true)).await.unwrap();
        let status = backend.status().await.unwrap();
        assert_eq!(status.mode, "global");
        assert!(status.tun);
        assert!(
            backend
                .execute(&Operation::Service {
                    action: ServiceAction::Start,
                    unit: "mihomo.service".into(),
                    user: false
                })
                .await
                .is_err()
        );
    }
    #[test]
    fn path_segments_escape_slashes_and_unicode() {
        let backend = Backend::demo().unwrap();
        let url = backend.url(&["proxies", "日本 /?#"]);
        assert!(url.path().contains("%2F%3F%23"));
        assert!(!url.path().contains("日本"));
        assert!(url.query().is_none());
    }
}
