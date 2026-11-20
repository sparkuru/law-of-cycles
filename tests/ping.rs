use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
    task::{JoinHandle, JoinSet},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Match,
    MatchWithoutLog,
    Direct,
    Reject,
    DialFailure,
    TranslatedSource,
    ProxyAuth,
    LogError,
    ConnectionError,
    EmptyRoute,
    ExplicitProxy,
}

struct Fixture {
    controller: String,
    proxy: String,
    requests: mpsc::UnboundedReceiver<String>,
    controller_task: JoinHandle<()>,
    proxy_task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.controller_task.abort();
        self.proxy_task.abort();
    }
}

async fn read_headers(socket: &mut TcpStream) -> String {
    let mut result = Vec::new();
    let mut buffer = [0; 2048];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        result.extend_from_slice(&buffer[..count]);
        assert!(result.len() <= 16 * 1024);
        if result.windows(4).any(|item| item == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(result).unwrap()
}

async fn reply(socket: &mut TcpStream, status: u16, body: Value) {
    let body = body.to_string();
    let header = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if socket.write_all(header.as_bytes()).await.is_ok() {
        let _ = socket.write_all(body.as_bytes()).await;
    }
}

fn connection(source: SocketAddr, port: u16, id: &str, case: Case) -> Value {
    let (rule, payload, chains) = match case {
        Case::Direct => ("", "", json!(["DIRECT"])),
        Case::Reject => ("DomainSuffix", "example.com", json!(["REJECT"])),
        Case::EmptyRoute => ("DomainSuffix", "example.com", json!([])),
        _ => ("DomainSuffix", "example.com", json!(["Tokyo 01", "Proxy"])),
    };
    json!({
        "id":id,
        "metadata": {
            "network":"tcp", "sourceIP":source.ip().to_string(),
            "sourcePort":source.port().to_string(), "host":"example.com",
            "destinationPort":port.to_string()
        },
        "rule":rule, "rulePayload":payload, "chains":chains
    })
}

impl Fixture {
    async fn new(case: Case, destination_port: u16) -> Self {
        Self::new_on(case, destination_port, "127.0.0.1:0").await
    }

    async fn new_on(case: Case, destination_port: u16, bind: &str) -> Self {
        let proxy_listener = TcpListener::bind(bind).await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let controller_listener = TcpListener::bind(bind).await.unwrap();
        let controller = format!("http://{}", controller_listener.local_addr().unwrap());
        let proxy = format!("http://{proxy_addr}");
        let (request_tx, requests) = mpsc::unbounded_channel();
        let (source_tx, source_rx) = watch::channel(None::<SocketAddr>);
        let (logs_ready_tx, logs_ready_rx) = watch::channel(false);

        let proxy_request_tx = request_tx.clone();
        let proxy_task = tokio::spawn(async move {
            let (mut socket, source) = proxy_listener.accept().await.unwrap();
            let request = read_headers(&mut socket).await;
            let _ = proxy_request_tx.send(request);
            if case == Case::ProxyAuth {
                socket
                    .write_all(
                        b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                    )
                    .await
                    .unwrap();
                return;
            }
            source_tx.send_replace(Some(source));
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
            if case != Case::DialFailure {
                let mut byte = [0];
                let _ = socket.read(&mut byte).await;
            }
        });

        let controller_task = tokio::spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = controller_listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let requests = request_tx.clone();
                        let mut source = source_rx.clone();
                        let ready = logs_ready_tx.clone();
                        let mut wait_for_logs = logs_ready_rx.clone();
                        handlers.spawn(async move {
                            let request = read_headers(&mut socket).await;
                            let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap().to_owned();
                            let _ = requests.send(request);
                            if path.starts_with("/logs?") {
                                if case == Case::LogError {
                                    ready.send_replace(true);
                                    reply(&mut socket, 500, json!({"error":"private-error-body"})).await;
                                    return;
                                }
                                ready.send_replace(true);
                                if source.wait_for(|value| value.is_some()).await.is_err() {
                                    return;
                                }
                                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n").await.unwrap();
                                let address = (*source.borrow()).unwrap();
                                let other = SocketAddr::new(address.ip(), address.port().wrapping_add(1));
                                let mut messages = vec![
                                    format!("[TCP] {other} --> example.com:{destination_port} match Match using OTHER-SOURCE"),
                                    format!("[TCP] {address} --> other.example:{destination_port} match Match using OTHER-TARGET"),
                                ];
                                match case {
                                    Case::Match | Case::ExplicitProxy | Case::ConnectionError | Case::EmptyRoute => {
                                        messages.push(format!("[TCP] {address}(kami, uid=1000) --> example.com:{destination_port} match DomainSuffix(example.com) using Proxy[Tokyo 01]"));
                                    }
                                    Case::Direct => {
                                        messages.push(format!("[TCP] {address} --> example.com:{destination_port} using DIRECT"));
                                    }
                                    Case::Reject => {
                                        messages.push(format!("[TCP] {address} --> example.com:{destination_port} match DomainSuffix(example.com) using REJECT"));
                                    }
                                    Case::DialFailure => {
                                        messages.push(format!("[TCP] dial Proxy (match DomainSuffix/example.com) {address} --> example.com:{destination_port} error: dial failed"));
                                    }
                                    _ => {}
                                }
                                for message in messages {
                                    let line = format!("{}\n", json!({"type":"info","payload":message}));
                                    let middle = line.len() / 2;
                                    if socket.write_all(&line.as_bytes()[..middle]).await.is_err() {
                                        return;
                                    }
                                    if socket.write_all(&line.as_bytes()[middle..]).await.is_err() {
                                        return;
                                    }
                                }
                                let mut byte = [0];
                                let _ = socket.read(&mut byte).await;
                            } else if path == "/configs" {
                                if wait_for_logs.wait_for(|ready| *ready).await.is_err() {
                                    return;
                                }
                                let mode = if case == Case::Direct { "direct" } else { "rule" };
                                let mixed = if case == Case::ExplicitProxy { 0 } else { proxy_addr.port() };
                                reply(&mut socket, 200, json!({"mode":mode,"mixed-port":mixed})).await;
                            } else if path == "/connections" {
                                if case == Case::ConnectionError {
                                    reply(&mut socket, 500, json!({"error":"private-error-body"})).await;
                                    return;
                                }
                                let address = *source.borrow();
                                let mut items = Vec::new();
                                if let Some(address) = address {
                                    let other = SocketAddr::new(address.ip(), address.port().wrapping_add(1));
                                    items.push(connection(other, destination_port, "unrelated", case));
                                    if matches!(case, Case::Match | Case::MatchWithoutLog | Case::Direct | Case::Reject | Case::LogError | Case::EmptyRoute | Case::ExplicitProxy) {
                                        items.push(connection(address, destination_port, "probe-id", case));
                                    }
                                }
                                reply(&mut socket, 200, json!({"connections":items})).await;
                            } else {
                                reply(&mut socket, 404, json!({})).await;
                            }
                        });
                    }
                    _ = handlers.join_next(), if !handlers.is_empty() => {}
                }
            }
        });
        Self {
            controller,
            proxy,
            requests,
            controller_task,
            proxy_task,
        }
    }

    async fn run(&self, extra: &[&str], timeout: &str) -> std::process::Output {
        self.run_mode(extra, timeout, true).await
    }

    async fn run_mode(&self, extra: &[&str], timeout: &str, json: bool) -> std::process::Output {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kami"));
        if json {
            command.arg("--json");
        }
        command
            .args([
                "--controller",
                &self.controller,
                "--timeout",
                timeout,
                "ping",
                "example.com",
            ])
            .args(extra)
            .env("XDG_CONFIG_HOME", "/nonexistent-kami-ping-test")
            .env_remove("KAMI_CONTROLLER")
            .env_remove("KAMI_TIMEOUT")
            .env("KAMI_SECRET", "ping-test-secret")
            .kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(8), command.output())
            .await
            .unwrap()
            .unwrap()
    }

    async fn run_tty(&self, json: bool, no_color: bool) -> std::process::Output {
        let mut args = vec![
            env!("CARGO_BIN_EXE_kami"),
            "--controller",
            &self.controller,
            "--timeout",
            "2",
            "ping",
            "example.com",
        ];
        if json {
            args.push("--json");
        }
        let child = args
            .iter()
            .map(|arg| law_of_cycles::system::quote_shell(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let mut command = tokio::process::Command::new("script");
        command
            .args(["-q", "-e", "-c", &child, "/dev/null"])
            .env("XDG_CONFIG_HOME", "/nonexistent-kami-ping-test")
            .env_remove("KAMI_CONTROLLER")
            .env_remove("KAMI_TIMEOUT")
            .env("KAMI_SECRET", "ping-test-secret")
            .kill_on_drop(true);
        if no_color {
            command.env("NO_COLOR", "");
        } else {
            command.env_remove("NO_COLOR");
        }
        tokio::time::timeout(Duration::from_secs(8), command.output())
            .await
            .unwrap()
            .unwrap()
    }
}

#[tokio::test]
async fn ping_colors_only_terminal_text_and_respects_no_color() {
    if tokio::process::Command::new("script")
        .arg("--version")
        .output()
        .await
        .is_err()
    {
        return;
    }
    let fixture = Fixture::new(Case::Match, 443).await;
    let output = fixture.run_tty(false, false).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\u{1b}[36mProxy\u{1b}[0m"));

    let fixture = Fixture::new(Case::Match, 443).await;
    let output = fixture.run_tty(false, true).await;
    assert!(output.status.success());
    assert!(!output.stdout.contains(&0x1b));

    let fixture = Fixture::new(Case::Match, 443).await;
    let output = fixture.run_tty(true, false).await;
    assert!(output.status.success());
    assert!(!output.stdout.contains(&0x1b));
    assert_eq!(output_json(&output)["route"], json!(["Proxy", "Tokyo 01"]));
}

#[tokio::test]
async fn ping_human_output_and_ipv6_listener() {
    let ipv6_available = TcpListener::bind("[::1]:0").await.is_ok();
    let bind = if ipv6_available {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    };
    let fixture = Fixture::new_on(Case::Match, 443, bind).await;
    let output = fixture.run_mode(&[], "2", false).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("target: example.com:443 (TCP)"));
    assert!(text.contains("rule: DomainSuffix (example.com)"));
    assert!(text.contains("route: Proxy > Tokyo 01"));
    assert!(text.contains("destination not verified"));
    assert!(!text.contains("OTHER-"));
    assert!(!text.contains('\u{1b}'));
}

fn output_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout: {}; stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[tokio::test]
async fn ping_reports_its_connection_and_filters_other_traffic() {
    let mut fixture = Fixture::new(Case::Match, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(
        output.status.success(),
        "stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value = output_json(&output);
    assert_eq!(value["connection_id"], "probe-id");
    assert_eq!(
        value["rule"],
        json!({"type":"DomainSuffix","payload":"example.com"})
    );
    assert_eq!(value["route"], json!(["Proxy", "Tokyo 01"]));
    assert_eq!(value["proxy_connect"], "accepted");
    let logs = value["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert!(
        logs[0]["payload"]
            .as_str()
            .unwrap()
            .contains("DomainSuffix(example.com)")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("OTHER-"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("ping-test-secret"));
    let mut requests = Vec::new();
    while let Ok(request) = fixture.requests.try_recv() {
        requests.push(request);
    }
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("CONNECT example.com:443 HTTP/1.1\r\n"))
    );
    assert!(
        requests
            .iter()
            .filter(|request| request.starts_with("GET "))
            .all(|request| request
                .to_ascii_lowercase()
                .contains("authorization: bearer ping-test-secret"))
    );
}

#[tokio::test]
async fn ping_keeps_missing_evidence_separate_from_a_route() {
    let fixture = Fixture::new(Case::MatchWithoutLog, 443).await;
    let output = fixture.run(&[], "0.4").await;
    assert!(
        output.status.success(),
        "stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value = output_json(&output);
    assert_eq!(value["observation"], "observed");
    assert_eq!(value["logs"], json!([]));

    let fixture = Fixture::new(Case::TranslatedSource, 443).await;
    let output = fixture.run(&[], "0.4").await;
    assert!(!output.status.success());
    let value = output_json(&output);
    assert_eq!(value["proxy_connect"], "accepted");
    assert_eq!(value["observation"], "unavailable");
    assert!(value["rule"].is_null());
    assert!(value["route"].is_null());
    assert_eq!(value["logs"], json!([]));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("OTHER-"));

    let fixture = Fixture::new(Case::EmptyRoute, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(!output.status.success());
    let value = output_json(&output);
    assert_eq!(value["observation"], "observed");
    assert!(value["route"].is_null());
}

#[tokio::test]
async fn ping_reports_direct_rejection_and_dial_failures() {
    let fixture = Fixture::new(Case::Direct, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(
        output.status.success(),
        "stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value = output_json(&output);
    assert_eq!(value["mode_at_start"], "direct");
    assert!(value["rule"].is_null());
    assert_eq!(value["route"], json!(["DIRECT"]));

    let fixture = Fixture::new(Case::Reject, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(!output.status.success());
    assert_eq!(output_json(&output)["route"], json!(["REJECT"]));
    assert!(String::from_utf8_lossy(&output.stderr).contains("rejected"));

    let fixture = Fixture::new(Case::DialFailure, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(!output.status.success());
    let value = output_json(&output);
    assert_eq!(value["observation"], "unavailable");
    assert!(value["rule"].is_null());
    assert!(
        value["logs"][0]["payload"]
            .as_str()
            .unwrap()
            .contains("dial failed")
    );
}

#[tokio::test]
async fn ping_handles_proxy_overrides_auth_and_log_errors() {
    let fixture = Fixture::new(Case::ExplicitProxy, 8443).await;
    let output = fixture
        .run(&["--proxy", &fixture.proxy, "--port", "8443"], "2")
        .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output_json(&output)["port"], 8443);

    let fixture = Fixture::new(Case::ProxyAuth, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTP 407"));
    assert!(output.stdout.is_empty());

    let fixture = Fixture::new(Case::LogError, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(output.status.success());
    let value = output_json(&output);
    assert_eq!(value["connection_id"], "probe-id");
    assert!(value["log_error"].as_str().unwrap().contains("HTTP 500"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-error-body"));

    let fixture = Fixture::new(Case::ConnectionError, 443).await;
    let output = fixture.run(&[], "2").await;
    assert!(!output.status.success());
    let value = output_json(&output);
    assert!(
        value["connection_error"]
            .as_str()
            .unwrap()
            .contains("HTTP 500")
    );
    assert_eq!(value["logs"].as_array().unwrap().len(), 1);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-error-body"));
}
