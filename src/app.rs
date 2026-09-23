use crate::model::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Proxies,
    Connections,
    Logs,
}

impl Page {
    pub const ALL: [Self; 4] = [Self::Overview, Self::Proxies, Self::Connections, Self::Logs];
    pub fn name(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Proxies => "Proxies",
            Self::Connections => "Connections",
            Self::Logs => "Logs",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|page| *page == self).unwrap()
    }
}
#[derive(Clone, Debug)]
pub enum Intent {
    Page(Page),
    Row(String),
    Open(String),
    Run(Operation),
    Actions,
    Modes,
    Services,
    Details(Vec<String>),
    Search,
    Back,
    Follow,
    Refresh,
    RefreshGroup,
    CleanLogs,
    Help,
    Result,
    Confirm(bool),
    MenuItem(usize),
    Dismiss,
}
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub intent: Intent,
    pub disabled: Option<String>,
}
impl MenuItem {
    fn new(label: &str, intent: Intent) -> Self {
        Self {
            label: label.into(),
            intent,
            disabled: None,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Overlay {
    Menu {
        title: String,
        items: Vec<MenuItem>,
        selected: usize,
    },
    Confirm {
        operation: Operation,
        yes: bool,
    },
    Document {
        title: String,
        lines: Vec<String>,
        scroll: u16,
    },
}
#[derive(Clone, Debug)]
pub enum Effect {
    Run(Operation),
    Refresh,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub rect: Rect,
    pub intent: Intent,
}
#[derive(Default)]
pub struct View {
    pub hits: Vec<Hit>,
    pub body: Rect,
    pub list: Rect,
    pub divider: Option<Rect>,
    pub popup: Option<Rect>,
}
pub struct App {
    pub demo: bool,
    pub endpoint: String,
    pub page: Page,
    pub sidebar_focus: bool,
    pub group: Option<String>,
    pub query: String,
    pub searching: bool,
    pub selected: Option<String>,
    pub offset: usize,
    pub status: Option<Status>,
    pub proxies: Proxies,
    pub connections: Connections,
    pub traffic: Traffic,
    pub logs: VecDeque<(String, Log)>,
    pub down_history: VecDeque<u64>,
    sequence: u64,
    pub follow: bool,
    pub errors: BTreeMap<Topic, String>,
    pub message: String,
    pub busy: bool,
    pub refresh_progress: Option<(usize, usize)>,
    pub latency_results: BTreeMap<String, Option<u64>>,
    pub show_process: bool,
    pub quit: bool,
    pub overlay: Option<Overlay>,
    pub view: View,
    pub split: u16,
    dragging: bool,
    last_group_click: Option<(String, u16, u16, Instant)>,
}
impl App {
    pub fn new(demo: bool, endpoint: String) -> Self {
        Self {
            demo,
            endpoint,
            page: Page::Overview,
            sidebar_focus: false,
            group: None,
            query: String::new(),
            searching: false,
            selected: None,
            offset: 0,
            status: None,
            proxies: Proxies::new(),
            connections: Connections::default(),
            traffic: Traffic::default(),
            logs: VecDeque::new(),
            down_history: VecDeque::new(),
            sequence: 0,
            follow: true,
            errors: BTreeMap::new(),
            message: "Select a page or right-click an item for actions.".into(),
            busy: false,
            refresh_progress: None,
            latency_results: BTreeMap::new(),
            show_process: false,
            quit: false,
            overlay: None,
            view: View::default(),
            split: 58,
            dragging: false,
            last_group_click: None,
        }
    }
    pub fn update(&mut self, update: Update) {
        match update {
            Update::GroupProgress { completed, total } => {
                self.refresh_progress = Some((completed, total));
            }
            Update::Latency { node, delay } => {
                self.latency_results.insert(node, delay);
            }
            Update::Data(topic, data) => {
                self.errors.remove(&topic);
                match data {
                    Data::Status(value) => self.status = Some(value),
                    Data::Proxies(value) => self.proxies = value,
                    Data::Connections(value) => self.connections = value,
                    Data::Traffic(value) => {
                        self.down_history.push_back(value.down);
                        if self.down_history.len() > 100 {
                            self.down_history.pop_front();
                        }
                        self.traffic = value;
                    }
                    Data::Log(mut log) => {
                        self.sequence += 1;
                        log.payload = log.payload.chars().take(4096).collect();
                        self.logs.push_back((self.sequence.to_string(), log));
                        if self.logs.len() > 500 {
                            self.logs.pop_front();
                        }
                    }
                }
            }
            Update::Error(topic, error) => {
                self.errors.insert(topic, error);
            }
            Update::Finished(label, result) => {
                self.busy = false;
                self.refresh_progress = None;
                self.message = match result {
                    Ok(value) if value["total"].is_u64() && value["failed"].is_u64() => {
                        let total = value["total"].as_u64().unwrap_or_default();
                        let failed = value["failed"].as_u64().unwrap_or_default();
                        let status = if failed == 0 {
                            "Done"
                        } else if failed == total {
                            "Failed"
                        } else {
                            "Partially completed"
                        };
                        format!(
                            "{status}: {label} — {} succeeded, {failed} failed ({total} total)",
                            total.saturating_sub(failed)
                        )
                    }
                    Ok(value) => format!(
                        "Done: {label} — {}",
                        value
                            .as_object()
                            .map(|object| object
                                .iter()
                                .map(|(key, value)| format!(
                                    "{key}: {}",
                                    value
                                        .as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| value.to_string())
                                ))
                                .collect::<Vec<_>>()
                                .join("; "))
                            .unwrap_or_default()
                    ),
                    Err(error) => format!("Failed: {label} — {error}"),
                };
            }
        }
        self.reconcile();
    }
    pub fn rows(&self) -> Vec<(String, Vec<String>)> {
        let mut rows: Vec<(String, Vec<String>)> = match self.page {
            Page::Overview => Vec::new(),
            Page::Proxies => match &self.group {
                None => self
                    .proxies
                    .iter()
                    .filter(|(_, proxy)| proxy.all.is_some())
                    .map(|(name, proxy)| {
                        (
                            name.clone(),
                            vec![name.clone(), proxy.kind.clone(), proxy.now.clone()],
                        )
                    })
                    .collect(),
                Some(group) => self
                    .proxies
                    .get(group)
                    .and_then(|proxy| proxy.all.as_ref())
                    .map(|all| {
                        all.iter()
                            .map(|name| {
                                let proxy = self.proxies.get(name).cloned().unwrap_or_default();
                                let current = self
                                    .proxies
                                    .get(group)
                                    .is_some_and(|group| group.now == *name);
                                let delay = self
                                    .latency_results
                                    .get(name)
                                    .copied()
                                    .unwrap_or_else(|| {
                                        proxy.history.last().map(|entry| entry.delay)
                                    })
                                    .filter(|value| *value > 0)
                                    .map(|value| format!("{value} ms"))
                                    .unwrap_or_else(|| {
                                        if self.latency_results.get(name) == Some(&None) {
                                            "Failed".into()
                                        } else {
                                            "—".into()
                                        }
                                    });
                                (
                                    name.clone(),
                                    vec![
                                        format!("{} {name}", if current { "●" } else { " " }),
                                        proxy.kind,
                                        delay,
                                    ],
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            Page::Connections => self
                .connections
                .items()
                .iter()
                .map(|item| {
                    let mut cells = vec![
                        format!(
                            "{}:{}",
                            if item.metadata.host.is_empty() {
                                &item.metadata.destination_ip
                            } else {
                                &item.metadata.host
                            },
                            item.metadata.destination_port
                        ),
                        item.chains.join(" > "),
                    ];
                    if self.show_process {
                        cells.insert(0, item.metadata.process.clone());
                    }
                    (item.id.clone(), cells)
                })
                .collect(),
            Page::Logs => self
                .logs
                .iter()
                .map(|(id, log)| (id.clone(), vec![log.level.clone(), log.payload.clone()]))
                .collect(),
        };
        if self.page == Page::Connections {
            rows.sort_by(|a, b| a.0.cmp(&b.0));
        }
        let query = self.query.to_lowercase();
        rows.retain(|(_, cells)| cells.join(" ").to_lowercase().contains(&query));
        rows
    }
    pub fn reconcile(&mut self) {
        let rows = self.rows();
        if self.page == Page::Logs && self.follow {
            self.selected = rows.last().map(|row| row.0.clone());
        } else if !rows
            .iter()
            .any(|row| Some(&row.0) == self.selected.as_ref())
        {
            self.selected = rows.first().map(|row| row.0.clone());
        }
    }
    pub fn move_by(&mut self, amount: isize) {
        self.last_group_click = None;
        self.reconcile();
        let rows = self.rows();
        if self.page == Page::Logs {
            self.follow = false;
        }
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|row| Some(&row.0) == self.selected.as_ref())
            .unwrap_or(0);
        self.selected = Some(
            rows[index.saturating_add_signed(amount).min(rows.len() - 1)]
                .0
                .clone(),
        );
    }
    pub fn change_page(&mut self, page: Page) {
        self.last_group_click = None;
        self.page = page;
        self.group = None;
        self.query.clear();
        self.selected = None;
        self.offset = 0;
        self.follow = true;
        self.reconcile();
    }
    pub fn details(&self) -> Vec<String> {
        let Some(id) = &self.selected else {
            return vec!["Select an item to inspect it.".into()];
        };
        match self.page {
            Page::Proxies => {
                let proxy = self.proxies.get(id).cloned().unwrap_or_default();
                let mut lines = vec![
                    id.clone(),
                    String::new(),
                    format!("Type: {}", proxy.kind),
                    format!("UDP: {}", proxy.udp),
                ];
                if let Some(all) = proxy.all {
                    lines.extend([
                        format!("Members: {}", all.len()),
                        format!("Current: {}", proxy.now),
                    ]);
                }
                if let Some(delay) = self
                    .latency_results
                    .get(id)
                    .copied()
                    .unwrap_or_else(|| proxy.history.last().map(|entry| entry.delay))
                    .filter(|delay| *delay > 0)
                {
                    lines.push(format!("Latency: {delay} ms"));
                } else if self.latency_results.get(id) == Some(&None) {
                    lines.push("Latency: Failed".into());
                } else {
                    lines.push("Latency: —".into());
                }
                if let Some(group) = &self.group {
                    lines.extend([
                        String::new(),
                        format!("Group: {group}"),
                        if self
                            .proxies
                            .get(group)
                            .is_some_and(|group| group.now == *id)
                        {
                            "Currently in use".into()
                        } else {
                            "Not currently selected".into()
                        },
                    ]);
                }
                lines
            }
            Page::Connections => self
                .connections
                .items()
                .iter()
                .find(|item| item.id == *id)
                .map(|item| {
                    let mut lines = vec![
                        format!("ID: {id}"),
                        String::new(),
                        format!("Host: {}", item.metadata.host),
                        format!("IP: {}", item.metadata.destination_ip),
                        format!("Port: {}", item.metadata.destination_port),
                        format!("Source: {}", item.metadata.source_ip),
                        format!("Network: {}", item.metadata.network),
                        format!("Rule: {} {}", item.rule, item.rule_payload),
                        format!("Route: {}", item.chains.join(" > ")),
                        format!("Upload: {}", bytes(item.upload)),
                        format!("Download: {}", bytes(item.download)),
                    ];
                    if self.show_process {
                        lines.insert(5, format!("Process: {}", item.metadata.process));
                    }
                    lines
                })
                .unwrap_or_default(),
            Page::Logs => self
                .logs
                .iter()
                .find(|(key, _)| key == id)
                .map(|(_, log)| vec![log.level.clone(), log.payload.clone()])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }
    pub fn actions(&self) -> Vec<MenuItem> {
        let mut items = Vec::new();
        match self.page {
            Page::Overview => {
                items.push(MenuItem::new("Routing mode…", Intent::Modes));
                items.push(MenuItem::new(
                    if self.status.as_ref().is_some_and(|status| status.tun) {
                        "Turn TUN off"
                    } else {
                        "Turn TUN on"
                    },
                    Intent::Run(Operation::Tun(
                        !self.status.as_ref().is_some_and(|status| status.tun),
                    )),
                ));
                if self.status.is_none() || self.errors.contains_key(&Topic::Status) {
                    for item in &mut items {
                        item.disabled = Some("Wait for a fresh controller status".into());
                    }
                }
                items.push(MenuItem::new("Local service…", Intent::Services));
            }
            Page::Proxies => {
                if let Some(node) = &self.selected {
                    if let Some(group) = &self.group {
                        let mut item = MenuItem::new(
                            "Use node",
                            Intent::Run(Operation::Select {
                                group: group.clone(),
                                node: node.clone(),
                            }),
                        );
                        if self
                            .proxies
                            .get(group)
                            .is_none_or(|group| group.kind != "Selector")
                        {
                            item.disabled = Some("Automatic group; choose a Selector group".into());
                        }
                        items.push(item);
                    } else {
                        items.push(MenuItem::new("Open group", Intent::Open(node.clone())));
                    }
                    items.push(MenuItem::new(
                        "Measure latency",
                        Intent::Run(Operation::Delay(node.clone())),
                    ));
                }
                if self.group.is_some() {
                    items.push(MenuItem::new("Refresh group", Intent::RefreshGroup));
                }
            }
            Page::Connections => {
                if let Some(id) = &self.selected {
                    items.push(MenuItem::new("Details", Intent::Details(self.details())));
                    items.push(MenuItem::new(
                        "Disconnect",
                        Intent::Run(Operation::Close(id.clone())),
                    ));
                }
            }
            Page::Logs => {
                items.push(MenuItem::new("Clean", Intent::CleanLogs));
                items.push(MenuItem::new(
                    if self.follow {
                        "Pause following"
                    } else {
                        "Follow latest"
                    },
                    Intent::Follow,
                ));
            }
        }
        if self.page != Page::Overview {
            if self.selected.is_some() && self.page != Page::Connections {
                items.push(MenuItem::new("Details", Intent::Details(self.details())));
            }
            if self.group.is_some() {
                items.push(MenuItem::new("Back to groups", Intent::Back));
            }
            items.push(MenuItem::new("Search…", Intent::Search));
        }
        items.push(MenuItem::new("Refresh", Intent::Refresh));
        items
    }
    pub fn choose(&mut self, item: MenuItem) -> Option<Effect> {
        if let Some(reason) = item.disabled {
            self.message = reason;
            return None;
        }
        self.overlay = None;
        self.activate(item.intent)
    }
    pub fn activate(&mut self, intent: Intent) -> Option<Effect> {
        self.last_group_click = None;
        match intent {
            Intent::Page(page) => {
                self.change_page(page);
                self.sidebar_focus = false;
            }
            Intent::Row(id) => {
                self.selected = Some(id);
                self.sidebar_focus = false;
                if self.page == Page::Logs {
                    self.follow = false;
                }
            }
            Intent::Open(group) => {
                self.group = Some(group);
                self.query.clear();
                self.selected = None;
                self.offset = 0;
                self.reconcile();
            }
            Intent::Run(operation) => {
                if self.busy {
                    self.message = "Wait for the current operation to finish.".into();
                } else if operation.needs_confirmation() {
                    self.overlay = Some(Overlay::Confirm {
                        operation,
                        yes: false,
                    });
                } else {
                    return self.submit(operation);
                }
            }
            Intent::Confirm(yes) => {
                if let Some(Overlay::Confirm { operation, .. }) = self.overlay.take() {
                    if yes {
                        return self.submit(operation);
                    }
                    self.message = "Cancelled.".into();
                }
            }
            Intent::Actions => {
                self.overlay = Some(Overlay::Menu {
                    title: self.selected.clone().unwrap_or(self.page.name().into()),
                    items: self.actions(),
                    selected: 0,
                })
            }
            Intent::Modes => {
                self.overlay = Some(Overlay::Menu {
                    title: "Routing mode".into(),
                    items: [
                        (Mode::Rule, "Rule — follow routing rules"),
                        (Mode::Global, "Global — use GLOBAL group"),
                        (Mode::Direct, "Direct — bypass proxies"),
                    ]
                    .into_iter()
                    .map(|(mode, label)| MenuItem::new(label, Intent::Run(Operation::Mode(mode))))
                    .collect(),
                    selected: 0,
                })
            }
            Intent::Services => {
                self.overlay = Some(Overlay::Menu {
                    title: "LOCAL mihomo.service".into(),
                    items: [
                        ServiceAction::Status,
                        ServiceAction::Start,
                        ServiceAction::Stop,
                        ServiceAction::Restart,
                        ServiceAction::Enable,
                        ServiceAction::Disable,
                    ]
                    .into_iter()
                    .map(|action| {
                        let label = action.as_str().to_owned();
                        let mut item = MenuItem::new(
                            &label,
                            Intent::Run(Operation::Service {
                                action,
                                unit: "mihomo.service".into(),
                                user: false,
                            }),
                        );
                        if self.demo {
                            item.disabled =
                                Some("Service operations are disabled in demo mode".into());
                        }
                        item
                    })
                    .collect(),
                    selected: 0,
                })
            }
            Intent::Details(lines) => {
                self.overlay = Some(Overlay::Document {
                    title: "Details".into(),
                    lines,
                    scroll: 0,
                })
            }
            Intent::Search => {
                self.searching = self.page != Page::Overview;
                if self.searching {
                    self.sidebar_focus = false;
                }
            }
            Intent::Back => {
                let group = self.group.take();
                self.query.clear();
                self.selected = group;
                self.offset = 0;
                self.reconcile();
            }
            Intent::Follow => {
                self.follow = !self.follow;
                self.reconcile();
            }
            Intent::Refresh => return Some(Effect::Refresh),
            Intent::RefreshGroup => {
                if self.page == Page::Proxies
                    && let Some(group) = self.group.clone()
                {
                    return self.activate(Intent::Run(Operation::RefreshGroup(group)));
                }
            }
            Intent::CleanLogs => {
                self.logs.clear();
                if self.page == Page::Logs {
                    self.selected = None;
                    self.offset = 0;
                }
                self.message = "Logs cleared; incoming logs continue.".into();
            }
            Intent::Help => {
                self.overlay = Some(Overlay::Document {
                    title: "Help".into(),
                    lines: [
                        "Click the sidebar to change pages.",
                        "Click a row to select it; right-click or a opens actions.",
                        "Enter or double-click opens a group; node actions use Enter.",
                        "Tab changes navigation/content focus.",
                        "Drag the list/inspector divider to resize.",
                        "Mouse wheel, arrows and Page Up/Down navigate lists.",
                        "/ searches; Ctrl+C clears search text; Esc leaves search focus.",
                        "Search keeps focus until Esc; q types text, Enter keeps editing.",
                        "m chooses mode; t toggles TUN; s opens local services.",
                        "d measures latency; x disconnects; G follows logs.",
                        "r refreshes the open group, or reloads data elsewhere; c cleans logs.",
                        "Confirmation defaults to Cancel; Tab then Enter confirms.",
                        "q exits kami and leaves Mihomo running.",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                    scroll: 0,
                })
            }
            Intent::Result => {
                self.overlay = Some(Overlay::Document {
                    title: "Last operation".into(),
                    lines: vec![self.message.clone()],
                    scroll: 0,
                })
            }
            Intent::MenuItem(index) => {
                if let Some(Overlay::Menu { items, .. }) = &self.overlay
                    && let Some(item) = items.get(index).cloned()
                {
                    return self.choose(item);
                }
            }
            Intent::Dismiss => self.overlay = None,
        }
        None
    }
    fn submit(&mut self, operation: Operation) -> Option<Effect> {
        if self.busy {
            self.message = "Wait for the current operation to finish.".into();
            return None;
        }
        self.busy = true;
        if let Operation::RefreshGroup(group) = &operation {
            self.refresh_progress = Some((
                0,
                self.proxies
                    .get(group)
                    .and_then(|proxy| proxy.all.as_ref())
                    .map_or(0, Vec::len),
            ));
        }
        self.message = operation.label();
        Some(Effect::Run(operation))
    }
    pub fn key(&mut self, key: KeyEvent) -> Option<Effect> {
        self.last_group_click = None;
        if self.searching {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear()
                }
                KeyCode::Esc => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && self.query.chars().count() < 200 =>
                {
                    self.query.push(ch)
                }
                _ => {}
            }
            self.reconcile();
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return None;
        }
        if self.overlay.is_some() {
            return self.overlay_key(key.code);
        }
        let intent = match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return None;
            }
            KeyCode::Char(ch @ '1'..='4') => {
                Some(Intent::Page(Page::ALL[ch as usize - '1' as usize]))
            }
            KeyCode::Tab => {
                self.sidebar_focus = !self.sidebar_focus;
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.navigate(1);
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.navigate(-1);
                None
            }
            KeyCode::PageDown => {
                self.move_by(self.view.list.height.max(1) as isize);
                None
            }
            KeyCode::PageUp => {
                self.move_by(-(self.view.list.height.max(1) as isize));
                None
            }
            KeyCode::Enter => {
                if self.sidebar_focus {
                    self.sidebar_focus = false;
                    None
                } else if self.page == Page::Proxies && self.group.is_none() {
                    self.selected.clone().map(Intent::Open)
                } else {
                    Some(Intent::Actions)
                }
            }
            KeyCode::Char('a') => Some(Intent::Actions),
            KeyCode::Char('/') => Some(Intent::Search),
            KeyCode::Esc | KeyCode::Left => Some(Intent::Back),
            KeyCode::Char('r') if self.page == Page::Proxies && self.group.is_some() => {
                Some(Intent::RefreshGroup)
            }
            KeyCode::Char('r') => Some(Intent::Refresh),
            KeyCode::Char('R') if self.page == Page::Proxies => Some(Intent::RefreshGroup),
            KeyCode::Char('c') if self.page == Page::Logs => Some(Intent::CleanLogs),
            KeyCode::Char('?') => Some(Intent::Help),
            KeyCode::Char('G') if self.page == Page::Logs => {
                self.follow = true;
                self.reconcile();
                None
            }
            KeyCode::Char(ch) => {
                let label = match ch {
                    'm' => "Routing mode…",
                    't' => {
                        if self.status.as_ref().is_some_and(|status| status.tun) {
                            "Turn TUN off"
                        } else {
                            "Turn TUN on"
                        }
                    }
                    's' => "Local service…",
                    'd' => "Measure latency",
                    'x' => "Disconnect",
                    _ => "",
                };
                if let Some(item) = self.actions().into_iter().find(|item| item.label == label) {
                    return self.choose(item);
                }
                None
            }
            _ => None,
        };
        intent.and_then(|intent| self.activate(intent))
    }
    fn navigate(&mut self, amount: isize) {
        if self.sidebar_focus {
            let index = (self.page.index() as isize + amount).rem_euclid(4) as usize;
            self.change_page(Page::ALL[index]);
        } else {
            self.move_by(amount);
        }
    }
    fn overlay_key(&mut self, key: KeyCode) -> Option<Effect> {
        let mut intent = None;
        match self.overlay.as_mut().unwrap() {
            Overlay::Menu {
                items, selected, ..
            } => match key {
                KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + 1) % items.len(),
                KeyCode::Up | KeyCode::Char('k') => {
                    *selected = (*selected + items.len() - 1) % items.len()
                }
                KeyCode::Enter => intent = Some(Intent::MenuItem(*selected)),
                KeyCode::Esc | KeyCode::Char('q') => intent = Some(Intent::Dismiss),
                _ => {}
            },
            Overlay::Confirm { yes, .. } => match key {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => *yes = !*yes,
                KeyCode::Enter => intent = Some(Intent::Confirm(*yes)),
                KeyCode::Char('y') => intent = Some(Intent::Confirm(true)),
                KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                    intent = Some(Intent::Confirm(false))
                }
                _ => {}
            },
            Overlay::Document { scroll, .. } => match key {
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => {
                    intent = Some(Intent::Dismiss)
                }
                _ => {}
            },
        }
        intent.and_then(|intent| self.activate(intent))
    }
    pub fn mouse(&mut self, event: MouseEvent) -> Option<Effect> {
        self.mouse_at(event, Instant::now())
    }
    fn mouse_at(&mut self, event: MouseEvent, now: Instant) -> Option<Effect> {
        if self.searching {
            self.last_group_click = None;
            return None;
        }
        if !matches!(
            event.kind,
            MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::Up(MouseButton::Left)
                | MouseEventKind::Moved
        ) {
            self.last_group_click = None;
        }
        let position = (event.column, event.row).into();
        if self.overlay.is_some() {
            self.last_group_click = None;
            match event.kind {
                MouseEventKind::ScrollUp => return self.overlay_key(KeyCode::Up),
                MouseEventKind::ScrollDown => return self.overlay_key(KeyCode::Down),
                MouseEventKind::Down(MouseButton::Left) => {
                    let hit = self
                        .view
                        .hits
                        .iter()
                        .rev()
                        .find(|hit| hit.rect.contains(position))
                        .cloned();
                    if let Some(hit) = hit {
                        return self.activate(hit.intent);
                    }
                    if matches!(self.overlay, Some(Overlay::Menu { .. }))
                        && self.view.popup.is_some_and(|rect| !rect.contains(position))
                    {
                        self.overlay = None;
                    }
                }
                _ => {}
            }
            return None;
        }
        if self.dragging {
            self.last_group_click = None;
            if matches!(
                event.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            ) {
                self.split = ((event.column.saturating_sub(self.view.body.x) as u32 * 100)
                    / u32::from(self.view.body.width.max(1)))
                .clamp(35, 72) as u16;
                if matches!(event.kind, MouseEventKind::Up(_)) {
                    self.dragging = false;
                }
            }
            return None;
        }
        match event.kind {
            MouseEventKind::ScrollUp if self.view.list.contains(position) => {
                self.move_by(-3);
                None
            }
            MouseEventKind::ScrollDown if self.view.list.contains(position) => {
                self.move_by(3);
                None
            }
            MouseEventKind::Down(button) => {
                if button == MouseButton::Left
                    && self
                        .view
                        .divider
                        .is_some_and(|rect| rect.contains(position))
                {
                    self.last_group_click = None;
                    self.dragging = true;
                    return None;
                }
                let hit = self
                    .view
                    .hits
                    .iter()
                    .rev()
                    .find(|hit| hit.rect.contains(position))
                    .cloned();
                let Some(hit) = hit else {
                    self.last_group_click = None;
                    return None;
                };
                if button == MouseButton::Right {
                    if matches!(hit.intent, Intent::Row(_)) {
                        self.activate(hit.intent);
                        return self.activate(Intent::Actions);
                    }
                    return None;
                }
                if button == MouseButton::Left {
                    if self.page == Page::Proxies
                        && self.group.is_none()
                        && let Intent::Row(id) = &hit.intent
                        && self
                            .proxies
                            .get(id)
                            .is_some_and(|proxy| proxy.all.is_some())
                    {
                        let double_click = self.last_group_click.as_ref().is_some_and(
                            |(previous, column, row, time)| {
                                previous == id
                                    && *column == event.column
                                    && *row == event.row
                                    && now.saturating_duration_since(*time)
                                        <= Duration::from_millis(400)
                            },
                        );
                        if double_click {
                            return self.activate(Intent::Open(id.clone()));
                        }
                        let click = (id.clone(), event.column, event.row, now);
                        let effect = self.activate(hit.intent);
                        self.last_group_click = Some(click);
                        return effect;
                    }
                    self.activate(hit.intent)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    fn proxy_app() -> App {
        let mut app = App::new(true, "demo".into());
        app.proxies.insert(
            "Proxy".into(),
            Proxy {
                kind: "Selector".into(),
                all: Some(vec!["Node".into()]),
                now: "Node".into(),
                ..Proxy::default()
            },
        );
        app.proxies.insert("Node".into(), Proxy::default());
        app.change_page(Page::Proxies);
        app.view.list = Rect::new(10, 5, 40, 10);
        app.view.hits.push(Hit {
            rect: Rect::new(10, 5, 40, 1),
            intent: Intent::Row("Proxy".into()),
        });
        app
    }

    fn click() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 12,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn double_click_opens_only_a_group_within_the_time_window() {
        let mut app = proxy_app();
        let now = Instant::now();
        assert!(app.mouse_at(click(), now).is_none());
        assert!(app.group.is_none());
        assert_eq!(app.selected.as_deref(), Some("Proxy"));
        app.mouse_at(
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                ..click()
            },
            now + Duration::from_millis(10),
        );
        assert!(
            app.mouse_at(click(), now + Duration::from_millis(300))
                .is_none()
        );
        assert_eq!(app.group.as_deref(), Some("Proxy"));
        app.view.hits[0].intent = Intent::Row("Node".into());
        app.mouse_at(click(), now + Duration::from_millis(350));
        assert!(
            app.mouse_at(click(), now + Duration::from_millis(400))
                .is_none()
        );
        assert!(!app.busy);
        assert!(app.overlay.is_none());

        let mut app = proxy_app();
        app.mouse_at(click(), now);
        app.mouse_at(click(), now + Duration::from_millis(401));
        assert!(app.group.is_none());
    }

    #[test]
    fn intervening_input_or_changed_identity_cancels_double_click() {
        let now = Instant::now();
        for interruption in 0..6 {
            let mut app = proxy_app();
            app.mouse_at(click(), now);
            match interruption {
                0 => {
                    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                }
                1 => {
                    app.mouse_at(
                        MouseEvent {
                            kind: MouseEventKind::ScrollDown,
                            ..click()
                        },
                        now,
                    );
                }
                2 => {
                    app.change_page(Page::Overview);
                    app.change_page(Page::Proxies);
                }
                3 => {
                    app.activate(Intent::Actions);
                    app.activate(Intent::Dismiss);
                }
                4 => {
                    app.mouse_at(
                        MouseEvent {
                            column: 0,
                            ..click()
                        },
                        now,
                    );
                }
                _ => {
                    app.mouse_at(
                        MouseEvent {
                            column: 13,
                            ..click()
                        },
                        now,
                    );
                }
            }
            app.mouse_at(click(), now + Duration::from_millis(100));
            assert!(app.group.is_none(), "interruption {interruption}");
        }
        let mut app = proxy_app();
        app.mouse_at(click(), now);
        app.proxies.insert(
            "Other".into(),
            Proxy {
                all: Some(vec![]),
                ..Proxy::default()
            },
        );
        app.view.hits[0].intent = Intent::Row("Other".into());
        app.mouse_at(click(), now + Duration::from_millis(100));
        assert!(app.group.is_none());
    }

    #[test]
    fn search_owns_keyboard_until_escape_and_keeps_the_filter() {
        let mut app = proxy_app();
        app.sidebar_focus = true;
        app.activate(Intent::Search);
        assert!(!app.sidebar_focus);
        app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(app.searching);
        assert!(!app.quit);
        assert_eq!(app.query, "q");
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.searching && !app.quit && app.query.is_empty());
        app.key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.searching);
        assert_eq!(app.query, "N");
        app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.quit);
    }
}
