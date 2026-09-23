use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use law_of_cycles::{
    app::{App, Effect, Intent, Overlay, Page},
    backend::Backend,
    config::Settings,
    model::*,
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
use reqwest::Method;
use serde_json::json;
use std::{process::Command, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
};

async fn demo() -> (App, Backend) {
    let backend = Backend::demo().unwrap();
    let mut app = App::new(true, backend.endpoint());
    for topic in [Topic::Status, Topic::Proxies, Topic::Connections] {
        app.update(Update::Data(topic, backend.fetch(topic).await.unwrap()));
    }
    app.update(Update::Data(
        Topic::Traffic,
        Data::Traffic(Traffic {
            up: 14000,
            down: 320000,
        }),
    ));
    (app, backend)
}
fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn mouse(x: u16, y: u16, kind: MouseEventKind) -> MouseEvent {
    MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

#[tokio::test]
async fn all_layouts_keep_hit_targets_inside_terminal() {
    for (width, height) in [(50, 14), (80, 14), (80, 24), (120, 30), (160, 45)] {
        for page in Page::ALL {
            let (mut app, _) = demo().await;
            app.change_page(page);
            let output = render(&mut app, width, height);
            assert!(output.contains("Law of Cycles"));
            for hit in &app.view.hits {
                assert!(
                    hit.rect.right() <= width && hit.rect.bottom() <= height,
                    "{width}x{height} {hit:?}"
                );
            }
            app.activate(Intent::Actions);
            render(&mut app, width, height);
            for hit in &app.view.hits {
                assert!(hit.rect.right() <= width && hit.rect.bottom() <= height);
            }
        }
    }
}
#[tokio::test]
async fn mouse_selection_does_not_mutate_but_use_node_does() {
    let (mut app, backend) = demo().await;
    app.change_page(Page::Proxies);
    app.activate(Intent::Open("Proxy".into()));
    render(&mut app, 120, 30);
    let hit = app
        .view
        .hits
        .iter()
        .find(|hit| matches!(&hit.intent,Intent::Row(id) if id=="Singapore 01"))
        .unwrap()
        .clone();
    assert!(
        app.mouse(mouse(
            hit.rect.x,
            hit.rect.y,
            MouseEventKind::Down(MouseButton::Left)
        ))
        .is_none()
    );
    assert_eq!(app.selected.as_deref(), Some("Singapore 01"));
    assert!(!app.busy);
    app.mouse(mouse(
        hit.rect.x,
        hit.rect.y,
        MouseEventKind::Down(MouseButton::Right),
    ));
    let effect = app.key(key(KeyCode::Enter)).unwrap();
    let Effect::Run(operation) = effect else {
        panic!("expected mutation")
    };
    backend.execute(&operation).await.unwrap();
    assert_eq!(
        backend.get(&["proxies"]).await.unwrap()["proxies"]["Proxy"]["now"],
        "Singapore 01"
    );
}
#[tokio::test]
async fn menu_disconnect_captures_identity_across_refresh() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Connections);
    app.activate(Intent::Actions);
    app.update(Update::Data(
        Topic::Connections,
        Data::Connections(Connections {
            connections: Some(vec![Connection {
                id: "replacement".into(),
                ..Default::default()
            }]),
            ..Default::default()
        }),
    ));
    app.key(key(KeyCode::Down));
    app.key(key(KeyCode::Enter));
    match &app.overlay {
        Some(Overlay::Confirm {
            operation: Operation::Close(id),
            ..
        }) => assert_eq!(id, "demo-browser"),
        _ => panic!("missing captured target"),
    }
    let Some(Effect::Run(Operation::Close(id))) = app.key(key(KeyCode::Char('y'))) else {
        panic!("expected close")
    };
    assert_eq!(id, "demo-browser");
}
#[tokio::test]
async fn confirmation_defaults_to_cancel_and_blocks_background_mouse() {
    let (mut app, _) = demo().await;
    render(&mut app, 120, 30);
    let nav = app
        .view
        .hits
        .iter()
        .find(|hit| matches!(hit.intent, Intent::Page(Page::Logs)))
        .unwrap()
        .clone();
    app.activate(Intent::Run(Operation::Tun(true)));
    render(&mut app, 120, 30);
    app.mouse(mouse(
        nav.rect.x,
        nav.rect.y,
        MouseEventKind::Down(MouseButton::Left),
    ));
    assert_eq!(app.page, Page::Overview);
    assert!(app.overlay.is_some());
    assert!(app.key(key(KeyCode::Enter)).is_none());
    assert!(!app.busy);
    assert!(app.overlay.is_none());
}
#[tokio::test]
async fn explicit_mode_menu_and_disabled_service_actions() {
    let (mut app, _) = demo().await;
    app.key(key(KeyCode::Char('m')));
    assert!(!app.busy);
    app.key(key(KeyCode::Down));
    assert!(matches!(
        app.key(key(KeyCode::Enter)),
        Some(Effect::Run(Operation::Mode(Mode::Global)))
    ));
    app.busy = false;
    app.activate(Intent::Services);
    assert!(app.key(key(KeyCode::Enter)).is_none());
    assert!(app.message.contains("disabled in demo"));
}
#[tokio::test]
async fn automatic_groups_reject_manual_selection_in_menu() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Proxies);
    app.activate(Intent::Open("Auto".into()));
    render(&mut app, 120, 30);
    assert!(
        !app.view
            .hits
            .iter()
            .any(|hit| matches!(hit.intent, Intent::Run(Operation::Select { .. })))
    );
    app.activate(Intent::Actions);
    assert!(app.key(key(KeyCode::Enter)).is_none());
    assert!(app.message.contains("Automatic"));
    assert!(!app.busy);
}
#[tokio::test]
async fn drag_changes_proxy_split_but_not_selection() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Proxies);
    render(&mut app, 120, 30);
    let divider = app.view.divider.unwrap();
    let selected = app.selected.clone();
    app.mouse(mouse(
        divider.x,
        divider.y,
        MouseEventKind::Down(MouseButton::Left),
    ));
    app.mouse(mouse(
        divider.x + 8,
        divider.y,
        MouseEventKind::Drag(MouseButton::Left),
    ));
    app.mouse(mouse(
        divider.x + 8,
        divider.y,
        MouseEventKind::Up(MouseButton::Left),
    ));
    assert!(app.split > 58);
    assert_eq!(app.selected, selected);
}
#[tokio::test]
async fn logs_are_bounded_and_filtered_follow_pauses_on_selection() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Logs);
    app.query = "visible".into();
    for i in 0..550 {
        app.update(Update::Data(
            Topic::Logs,
            Data::Log(Log {
                level: "info".into(),
                payload: if i % 2 == 0 {
                    "visible".into()
                } else {
                    "hidden".into()
                },
            }),
        ));
    }
    assert_eq!(app.logs.len(), 500);
    assert_eq!(app.selected.as_deref(), Some("549"));
    app.move_by(-1);
    let selected = app.selected.clone();
    app.update(Update::Data(
        Topic::Logs,
        Data::Log(Log {
            level: "info".into(),
            payload: "visible".into(),
        }),
    ));
    assert_eq!(app.selected, selected);
}
#[tokio::test]
async fn details_are_readable_and_scroll_is_clamped() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Connections);
    app.show_process = true;
    let output = render(&mut app, 120, 30);
    assert!(output.contains("SELECTED CONNECTION"));
    assert!(output.contains("firefox"));
    app.activate(Intent::Details(app.details()));
    if let Some(Overlay::Document { scroll, .. }) = &mut app.overlay {
        *scroll = 60000;
    }
    render(&mut app, 80, 24);
    assert!(matches!(
        app.overlay,
        Some(Overlay::Document { scroll: 0..=20, .. })
    ));
}
#[tokio::test]
async fn menu_outside_click_closes_without_navigation() {
    let (mut app, _) = demo().await;
    render(&mut app, 120, 30);
    let nav = app
        .view
        .hits
        .iter()
        .find(|hit| matches!(hit.intent, Intent::Page(Page::Logs)))
        .unwrap()
        .clone();
    app.activate(Intent::Actions);
    render(&mut app, 120, 30);
    app.mouse(mouse(
        nav.rect.x,
        nav.rect.y,
        MouseEventKind::Down(MouseButton::Left),
    ));
    assert!(app.overlay.is_none());
    assert_eq!(app.page, Page::Overview);
}

struct Reply {
    status: u16,
    body: Vec<u8>,
    headers: String,
}
impl Reply {
    fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(&body).unwrap(),
            headers: String::new(),
        }
    }
}
struct Fixture {
    url: String,
    requests: mpsc::UnboundedReceiver<String>,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                loop {
                    let count = socket.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(end) = request.windows(4).position(|item| item == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let _ = tx.send(String::from_utf8(request).unwrap());
                let header = format!(
                    "HTTP/1.1 {} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{}\r\n",
                    reply.status,
                    reply.body.len(),
                    reply.headers
                );
                if socket.write_all(header.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(&reply.body).await;
                }
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    fn backend(&self, secret: &str) -> Backend {
        Backend::new(Settings::new(&self.url, secret.into(), 0.5).unwrap()).unwrap()
    }
}
#[tokio::test]
async fn executable_connects_using_mihomo_yaml_without_modifying_it() {
    let mut fixture = Fixture::new(vec![
        Reply::json(200, json!({"version": "yaml-fixture"})),
        Reply::json(200, json!({"mode": "rule", "tun": {"enable": false}})),
    ])
    .await;
    let path = std::env::temp_dir().join(format!("kami-cli-yaml-{}.yaml", std::process::id()));
    let text = format!(
        "external-controller: '{}'\nsecret: 'yaml-test-token'\nproxies: []\nrules: ['MATCH,DIRECT']\n",
        fixture.url.strip_prefix("http://").unwrap()
    );
    std::fs::write(&path, &text).unwrap();
    let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_kami"))
        .args([
            "--mihomo-config",
            path.to_str().unwrap(),
            "--json",
            "status",
        ])
        .env_remove("KAMI_CONTROLLER")
        .env_remove("KAMI_SECRET")
        .env_remove("KAMI_TIMEOUT")
        .output()
        .await
        .unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(text, after);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("yaml-fixture"));
    assert!(!output.contains("yaml-test-token"));
    for _ in 0..2 {
        assert!(
            fixture
                .requests
                .recv()
                .await
                .unwrap()
                .contains("authorization: Bearer yaml-test-token")
        );
    }
}

#[tokio::test]
async fn controller_uses_bearer_auth_and_redacts_error_bodies() {
    let mut fixture = Fixture::new(vec![Reply::json(401, json!({"secret":"do-not-print"}))]).await;
    let error = fixture
        .backend("credential")
        .get(&["version"])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("401"));
    assert!(!error.contains("do-not-print"));
    assert!(!error.contains("credential"));
    assert!(
        fixture
            .requests
            .recv()
            .await
            .unwrap()
            .contains("authorization: Bearer credential")
    );
}
#[tokio::test]
async fn redirects_are_not_followed() {
    let fixture = Fixture::new(vec![Reply {
        status: 302,
        body: Vec::new(),
        headers: "Location: http://127.0.0.1:1/leak\r\n".into(),
    }])
    .await;
    assert!(
        fixture
            .backend("secret")
            .get(&["version"])
            .await
            .unwrap_err()
            .to_string()
            .contains("302")
    );
}
#[tokio::test]
async fn escaped_names_and_empty_204_are_supported() {
    let mut fixture = Fixture::new(vec![Reply {
        status: 204,
        body: Vec::new(),
        headers: String::new(),
    }])
    .await;
    let backend = fixture.backend("");
    backend
        .execute(&Operation::Close("日本 /?#".into()))
        .await
        .unwrap();
    let request = fixture.requests.recv().await.unwrap();
    assert!(request.starts_with("DELETE /connections/"));
    assert!(request.contains("%2F%3F%23"));
}
#[tokio::test]
async fn malformed_json_and_readback_mismatch_are_errors() {
    let fixture = Fixture::new(vec![Reply::json(200, json!(["wrong shape"]))]).await;
    assert!(
        fixture
            .backend("")
            .get(&["version"])
            .await
            .unwrap_err()
            .to_string()
            .contains("JSON object")
    );
    let fixture = Fixture::new(vec![
        Reply::json(204, json!({})),
        Reply::json(200, json!({"mode":"rule"})),
    ])
    .await;
    assert!(
        fixture
            .backend("")
            .execute(&Operation::Mode(Mode::Global))
            .await
            .unwrap_err()
            .to_string()
            .contains("not reflected")
    );
}
#[tokio::test]
async fn tun_patch_is_minimal_and_readback_is_checked() {
    let mut fixture = Fixture::new(vec![
        Reply {
            status: 204,
            body: Vec::new(),
            headers: String::new(),
        },
        Reply::json(200, json!({"tun":{"enable":true,"auto-route":true}})),
    ])
    .await;
    let result = fixture
        .backend("")
        .execute(&Operation::Tun(true))
        .await
        .unwrap();
    assert_eq!(result["network-verified"], false);
    let request = fixture.requests.recv().await.unwrap();
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).unwrap(),
        json!({"tun":{"enable":true}})
    );
}
#[tokio::test]
async fn streams_reconnect_after_eof_and_keep_data_typed() {
    let response = || Reply {
        status: 200,
        body: b"\r\n{\"up\":12,\"down\":24}\n".to_vec(),
        headers: String::new(),
    };
    let fixture = Fixture::new(vec![response(), response()]).await;
    let backend = fixture.backend("");
    let (tx, mut rx) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        backend.follow(Topic::Traffic, "info", tx).await;
    });
    let mut frames = 0;
    tokio::time::timeout(Duration::from_secs(4), async {
        while frames < 2 {
            if let Some(Update::Data(_, Data::Traffic(value))) = rx.recv().await {
                assert_eq!(value.down, 24);
                frames += 1;
            }
        }
    })
    .await
    .unwrap();
    task.abort();
}
#[tokio::test]
async fn oversized_stream_line_is_rejected() {
    let fixture = Fixture::new(vec![Reply {
        status: 200,
        body: vec![b'x'; 65537],
        headers: String::new(),
    }])
    .await;
    let backend = fixture.backend("");
    let (tx, mut rx) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        backend.follow(Topic::Logs, "info", tx).await;
    });
    let result = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result,Update::Error(_,error) if error.contains("64 KiB")));
    task.abort();
}
#[tokio::test]
async fn timeout_is_bounded_and_credentials_are_not_in_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = Backend::new(
        Settings::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "do-not-print".into(),
            0.05,
        )
        .unwrap(),
    )
    .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), backend.get(&["version"]))
        .await
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(!error.contains("do-not-print"));
}
#[tokio::test]
async fn status_omits_sensitive_configuration() {
    let fixture = Fixture::new(vec![
        Reply::json(200, json!({"version":"test"})),
        Reply::json(
            200,
            json!({"mode":"rule","secret":"hidden","tun":{"enable":false}}),
        ),
    ])
    .await;
    let value = serde_json::to_value(fixture.backend("").status().await.unwrap()).unwrap();
    assert_eq!(value["version"], "test");
    assert!(value.get("secret").is_none());
}
#[test]
fn executable_keeps_cli_contract_and_child_exit_code() {
    let binary = env!("CARGO_BIN_EXE_kami");
    let output = Command::new(binary).args(["--version"]).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("0.1.0"));
    let output = Command::new(binary)
        .args(["env", "--unset", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["unset"].is_array()
    );
    let output = Command::new(binary)
        .env("KAMI_SECRET", "hidden")
        .args([
            "exec",
            "--proxy",
            "http://localhost:7890",
            "--",
            "sh",
            "-c",
            "printf '%s:%s' \"$http_proxy\" \"${KAMI_SECRET-unset}\"; exit 7",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "http://localhost:7890:unset"
    );
}
#[test]
fn startup_without_an_explicit_target_prints_help() {
    let help = Command::new(env!("CARGO_BIN_EXE_kami"))
        .arg("-h")
        .output()
        .unwrap();
    for args in [vec![], vec!["--json"], vec!["--timeout", "2"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_kami"))
            .args(args)
            .env("KAMI_CONTROLLER", "invalid-controller")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(output.stdout, help.stdout);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_kami"))
        .args(["--controller", "http://127.0.0.1:9090"])
        .env_remove("KAMI_SECRET")
        .env_remove("KAMI_TIMEOUT")
        .env("XDG_CONFIG_HOME", "/nonexistent-kami-test")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
}

#[test]
fn noninteractive_tui_fails_with_a_useful_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_kami"))
        .args(["tui", "--demo"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
}

#[test]
fn tui_without_a_controller_exits_before_opening_the_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_kami"))
        .arg("tui")
        .env_remove("KAMI_CONTROLLER")
        .env_remove("KAMI_SECRET")
        .env("XDG_CONFIG_HOME", "/nonexistent-kami-test")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("TUI needs a controller"));
    assert!(error.contains("--controller"));
    assert!(error.contains("KAMI_SECRET"));
    assert!(error.contains("--config"));
    assert!(!error.contains("interactive terminal"));
}
#[tokio::test]
async fn delay_has_encoded_query_and_timeout() {
    let mut fixture = Fixture::new(vec![Reply::json(200, json!({"delay":42}))]).await;
    fixture
        .backend("")
        .request(
            Method::GET,
            &["proxies", "node/a", "delay"],
            None,
            &[
                ("url", "https://www.gstatic.com/generate_204"),
                ("timeout", "5000"),
            ],
        )
        .await
        .unwrap();
    let request = fixture.requests.recv().await.unwrap();
    assert!(request.contains("node%2Fa/delay?"));
    assert!(request.contains("timeout=5000"));
}

#[tokio::test]
async fn redesigned_header_and_overview_keep_address_in_content() {
    let (mut app, _) = demo().await;
    app.demo = false;
    app.endpoint = "http://127.0.0.1:9090/".into();
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(56, 0)].symbol(), "円");
    let output = render(&mut app, 120, 30);
    let header = output.lines().next().unwrap();
    assert!(header.contains("Law of Cycles"));
    assert!(header.contains("ONLINE"));
    assert!(!header.contains("9090"));
    assert!(output.contains("http://127.0.0.1:9090/"));
    assert!(output.contains("DOWNLOAD HISTORY"));
    app.update(Update::Error(Topic::Status, "unavailable".into()));
    assert!(
        render(&mut app, 120, 30)
            .lines()
            .next()
            .unwrap()
            .contains("OFFLINE")
    );
}

#[tokio::test]
async fn group_refresh_button_and_keyboard_preserve_selection() {
    for (width, height) in [(50, 14), (80, 14), (120, 30)] {
        let (mut app, backend) = demo().await;
        app.change_page(Page::Proxies);
        app.activate(Intent::Open("Proxy".into()));
        app.query = "Tokyo".into();
        let output = render(&mut app, width, height);
        assert!(output.contains("Refresh group"));
        assert!(app.view.list.height > 0);
        assert!(output.contains("Tokyo 01"));
        let selected = app.selected.clone();
        let hit = app
            .view
            .hits
            .iter()
            .find(|hit| matches!(hit.intent, Intent::RefreshGroup))
            .unwrap()
            .clone();
        let Some(Effect::Run(Operation::RefreshGroup(group))) = app.mouse(mouse(
            hit.rect.x,
            hit.rect.y,
            MouseEventKind::Down(MouseButton::Left),
        )) else {
            panic!("missing refresh effect")
        };
        assert_eq!(group, "Proxy");
        assert!(app.key(key(KeyCode::Char('r'))).is_none());
        let (tx, mut rx) = mpsc::channel(128);
        let result = backend.refresh_group(&group, Some(&tx)).await.unwrap();
        while let Ok(update) = rx.try_recv() {
            app.update(update);
        }
        app.update(Update::Finished(
            "Refresh group Proxy".into(),
            Ok(result.clone()),
        ));
        assert_eq!(
            result["total"],
            app.proxies["Proxy"].all.as_ref().unwrap().len()
        );
        assert_eq!(app.selected, selected);
        assert!(!app.busy);
        assert!(app.refresh_progress.is_none());
        assert!(matches!(
            app.key(key(KeyCode::Char('r'))),
            Some(Effect::Run(Operation::RefreshGroup(_)))
        ));
    }
}

#[tokio::test]
async fn clean_button_clears_local_logs_and_accepts_new_entries() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Logs);
    for payload in ["first", "second"] {
        app.update(Update::Data(
            Topic::Logs,
            Data::Log(Log {
                level: "info".into(),
                payload: payload.into(),
            }),
        ));
    }
    app.move_by(-1);
    app.query = "first".into();
    app.offset = 10;
    let output = render(&mut app, 50, 14);
    assert!(output.contains("Clean"));
    let hit = app
        .view
        .hits
        .iter()
        .find(|hit| matches!(hit.intent, Intent::CleanLogs))
        .unwrap()
        .clone();
    assert!(
        app.mouse(mouse(
            hit.rect.x,
            hit.rect.y,
            MouseEventKind::Down(MouseButton::Left)
        ))
        .is_none()
    );
    assert!(app.logs.is_empty());
    assert!(app.selected.is_none());
    assert_eq!(app.offset, 0);
    assert!(!app.busy);
    app.query.clear();
    app.update(Update::Data(
        Topic::Logs,
        Data::Log(Log {
            level: "info".into(),
            payload: "new entry".into(),
        }),
    ));
    assert_eq!(app.logs.len(), 1);
    assert!(render(&mut app, 120, 30).contains("new entry"));
    app.key(key(KeyCode::Char('c')));
    assert!(app.logs.is_empty());
}

#[tokio::test]
async fn connection_process_column_is_opt_in() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Connections);
    let output = render(&mut app, 120, 30);
    assert!(output.contains("DESTINATION"));
    assert!(output.contains("ROUTE"));
    assert!(!output.contains("PROCESS"));
    assert!(!output.contains("firefox"));
    assert!(app.rows().iter().all(|(_, cells)| cells.len() == 2));
    app.show_process = true;
    let output = render(&mut app, 120, 30);
    assert!(output.contains("PROCESS"));
    assert!(output.contains("firefox"));
    assert!(app.rows().iter().all(|(_, cells)| cells.len() == 3));
}

#[tokio::test]
async fn manual_latency_failure_does_not_show_stale_success() {
    let (mut app, backend) = demo().await;
    app.change_page(Page::Proxies);
    app.activate(Intent::Open("Proxy".into()));
    app.activate(Intent::Row("Tokyo 01".into()));
    app.update(Update::Latency {
        node: "Tokyo 01".into(),
        delay: None,
    });
    app.update(Update::Data(
        Topic::Proxies,
        backend.fetch(Topic::Proxies).await.unwrap(),
    ));
    let row = app
        .rows()
        .into_iter()
        .find(|(id, _)| id == "Tokyo 01")
        .unwrap();
    assert_eq!(row.1[2], "Failed");
    assert!(app.details().iter().any(|line| line == "Latency: Failed"));
}

#[tokio::test]
async fn focus_indicator_and_row_highlight_follow_tab_and_search() {
    for width in [50, 120] {
        let (mut app, _) = demo().await;
        app.change_page(Page::Proxies);
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let row = app.view.list;
        let content_color = terminal.backend().buffer()[(row.x, row.y)].bg;
        assert!(render(&mut app, width, 30).contains("FOCUS: CONTENT / Proxies"));
        app.key(key(KeyCode::Tab));
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        assert_ne!(
            content_color,
            terminal.backend().buffer()[(row.x, row.y)].bg
        );
        assert!(render(&mut app, width, 30).contains("FOCUS: NAVIGATION"));
        app.key(key(KeyCode::Tab));
        assert!(render(&mut app, width, 30).contains("FOCUS: CONTENT / Proxies"));
        app.activate(Intent::Search);
        let output = render(&mut app, width, 30);
        assert!(output.contains("FOCUS: SEARCH / Proxies"));
        assert!(output.contains("Ctrl+C clears"));
        assert!(output.contains("q types text"));
        assert!(!output.lines().last().unwrap().trim().ends_with("q quit"));
    }
}

#[tokio::test]
async fn search_keeps_cursor_visible_for_long_unicode_input_and_escape_retains_filter() {
    let (mut app, _) = demo().await;
    app.change_page(Page::Connections);
    app.activate(Intent::Search);
    app.query = "日本語".repeat(60);
    let mut terminal = Terminal::new(TestBackend::new(50, 14)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let search = app
        .view
        .hits
        .iter()
        .find(|hit| matches!(hit.intent, Intent::Search))
        .unwrap()
        .rect;
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(search.contains(cursor));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.query.is_empty() && app.searching && !app.quit);
    app.key(key(KeyCode::Char('q')));
    app.key(key(KeyCode::Enter));
    assert_eq!(app.query, "q");
    assert!(app.searching && !app.quit);
    app.key(key(KeyCode::Esc));
    assert_eq!(app.query, "q");
    assert!(!app.searching);
    app.key(key(KeyCode::Char('q')));
    assert!(app.quit);
}

#[tokio::test]
async fn connection_details_are_below_full_width_rows_at_all_sizes() {
    for (width, height) in [(50, 14), (80, 14), (80, 24), (120, 30), (160, 45)] {
        let (mut app, _) = demo().await;
        app.change_page(Page::Connections);
        let output = render(&mut app, width, height);
        assert_eq!(app.view.list.width, app.view.body.width);
        assert!(app.view.list.height > 0);
        assert!(app.view.divider.is_none());
        let details_y = output
            .lines()
            .position(|line| line.contains("SELECTED CONNECTION") || line.contains("Selected:"))
            .unwrap();
        assert!(details_y >= app.view.list.bottom() as usize);
        assert!(
            app.view
                .hits
                .iter()
                .any(|hit| matches!(hit.intent, Intent::Details(_)))
        );
        app.move_by(1);
        let output = render(&mut app, width, height);
        let lower = output
            .lines()
            .skip(details_y)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lower.contains("packages.example.org"));
        assert!(!lower.contains("example.com"));
    }
}

#[tokio::test]
async fn overview_summarizes_reported_ports_routes_protocols_and_warnings() {
    let (mut app, _) = demo().await;
    app.update(Update::Data(
        Topic::Logs,
        Data::Log(Log {
            level: "warning".into(),
            payload: "provider delayed".into(),
        }),
    ));
    let output = render(&mut app, 120, 30);
    for expected in [
        "MIX 7890",
        "PROXY GROUPS",
        "Tokyo 01",
        "TCP 2",
        "provider delayed",
    ] {
        assert!(output.contains(expected), "missing {expected}");
    }
    app.update(Update::Error(Topic::Proxies, "controller down".into()));
    assert!(render(&mut app, 120, 30).contains("PROXY GROUPS · stale"));
}

#[tokio::test]
async fn clicked_menu_page_stays_highlighted_when_content_has_focus() {
    for width in [50, 120] {
        let (mut app, _) = demo().await;
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        for page in [Page::Proxies, Page::Connections, Page::Logs] {
            terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
            let hit = app
                .view
                .hits
                .iter()
                .find(|hit| matches!(hit.intent, Intent::Page(p) if p == page))
                .unwrap()
                .clone();
            app.mouse(mouse(
                hit.rect.x,
                hit.rect.y,
                MouseEventKind::Down(MouseButton::Left),
            ));
            assert_eq!(app.page, page);
            assert!(!app.sidebar_focus);
            terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
            let active = terminal.backend().buffer()[(hit.rect.x, hit.rect.y)].bg;
            let other = app
                .view
                .hits
                .iter()
                .find(|hit| matches!(hit.intent, Intent::Page(Page::Overview)))
                .unwrap()
                .rect;
            assert_ne!(active, terminal.backend().buffer()[(other.x, other.y)].bg);
            app.key(key(KeyCode::Tab));
            app.key(key(KeyCode::Tab));
            terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
            assert_eq!(
                active,
                terminal.backend().buffer()[(hit.rect.x, hit.rect.y)].bg
            );
        }
    }
}

#[tokio::test]
async fn proxy_parent_entry_is_pinned_and_returns_to_the_same_group() {
    for (width, height) in [(50, 14), (80, 24), (120, 30)] {
        for query in ["", "no matching node"] {
            let (mut app, _) = demo().await;
            app.change_page(Page::Proxies);
            app.activate(Intent::Open("Proxy".into()));
            app.query = query.into();
            app.offset = 99;
            let output = render(&mut app, width, height);
            assert!(output.contains("..  Back to groups"));
            assert!(app.view.list.height >= 1);
            let hit = app
                .view
                .hits
                .iter()
                .find(|hit| matches!(hit.intent, Intent::Back))
                .unwrap()
                .clone();
            assert!(hit.rect.y < app.view.list.y);
            assert!(
                app.mouse(mouse(
                    hit.rect.x,
                    hit.rect.y,
                    MouseEventKind::Down(MouseButton::Left)
                ))
                .is_none()
            );
            assert!(app.group.is_none());
            assert_eq!(app.selected.as_deref(), Some("Proxy"));
            assert!(app.query.is_empty());
            assert_eq!(app.offset, 0);
            assert!(!app.busy);
            assert!(!render(&mut app, width, height).contains("..  Back to groups"));
        }
    }
}
