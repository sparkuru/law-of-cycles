use crate::{
    app::{App, Hit, Intent, Overlay, Page, View},
    model::{Topic, bytes, clean},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row,
        Sparkline, Table, TableState, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BACKGROUND: Color = Color::Rgb(20, 19, 27);
const FOREGROUND: Color = Color::Rgb(228, 222, 236);
const ACCENT: Color = Color::Rgb(196, 167, 231);
const PINK: Color = Color::Rgb(235, 188, 206);
const MUTED: Color = Color::Rgb(155, 146, 170);
const BORDER: Color = Color::Rgb(54, 48, 64);
const SELECTION: Color = Color::Rgb(48, 38, 62);
const GOOD: Color = Color::Rgb(156, 207, 188);
const WARNING: Color = Color::Rgb(235, 207, 156);
const ERROR: Color = Color::Rgb(235, 111, 146);
fn accent() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
fn selected() -> Style {
    Style::default().fg(ACCENT).bg(SELECTION)
}
fn content_focused(app: &App) -> bool {
    !app.sidebar_focus && !app.searching && app.overlay.is_none()
}
fn focus_style(focused: bool) -> Style {
    if focused {
        selected().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    }
}
fn current_page_style(app: &App) -> Style {
    if app.sidebar_focus && !app.searching && app.overlay.is_none() {
        selected().bold()
    } else {
        selected()
    }
}
fn page_title(frame: &mut Frame, app: &App, area: Rect, title: &str) {
    text(
        frame,
        area,
        format!("{} {title}", if content_focused(app) { "›" } else { " " }),
        focus_style(content_focused(app)),
    );
}
fn block(title: impl Into<String>, focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(FOREGROUND).bg(BACKGROUND))
        .title(format!(" {} ", clean(&title.into())))
        .border_style(Style::default().fg(if focused { ACCENT } else { BORDER }))
}
fn text(frame: &mut Frame, rect: Rect, text: impl Into<String>, style: Style) {
    frame.render_widget(Paragraph::new(clean(&text.into())).style(style), rect);
}
fn line_rect(area: Rect, offset: u16) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        u16::from(offset < area.height),
    )
}
fn button(
    frame: &mut Frame,
    app: &mut App,
    position: (u16, u16),
    label: &str,
    intent: Intent,
    available: u16,
    enabled: bool,
) -> u16 {
    let label = format!("[ {label} ]");
    let width = label.width() as u16;
    if width > available || position.1 >= frame.area().bottom() {
        return 0;
    }
    let rect = Rect::new(position.0, position.1, width, 1);
    text(
        frame,
        rect,
        label,
        if enabled {
            selected()
        } else {
            Style::default().fg(MUTED)
        },
    );
    if enabled {
        app.view.hits.push(Hit { rect, intent });
    }
    width + 1
}
fn paragraph(frame: &mut Frame, rect: Rect, lines: Vec<String>, scroll: u16) {
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .map(|line| Line::from(clean(&line)))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0)),
        rect,
    );
}
fn separator(frame: &mut Frame, rect: Rect, borders: Borders, focused: bool) {
    frame.render_widget(
        Block::new()
            .borders(borders)
            .border_style(Style::default().fg(if focused { ACCENT } else { BORDER })),
        rect,
    );
}
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.view = View::default();
    frame.render_widget(
        Block::new().style(Style::default().fg(FOREGROUND).bg(BACKGROUND)),
        area,
    );
    if area.width < 50 || area.height < 14 {
        text(
            frame,
            area,
            "Law of Cycles: resize to 50 × 14; q quits",
            accent(),
        );
        return;
    }
    let area = area.inner(Margin::new(1, 0));
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(area);
    header(frame, app, rows[0]);
    let body = navigation(frame, app, rows[1]);
    app.view.body = body;
    match app.page {
        Page::Overview => overview(frame, app, body),
        _ => browser(frame, app, body),
    }
    footer(frame, app, rows[2]);
    if app.overlay.is_some() {
        app.view.hits.clear();
        overlay(frame, app, area);
    }
}
fn header(frame: &mut Frame, app: &App, area: Rect) {
    let (badge, color) = if app.demo {
        ("DEMO", WARNING)
    } else if app.errors.contains_key(&Topic::Status) {
        ("OFFLINE", ERROR)
    } else if app.status.is_some() {
        ("ONLINE", GOOD)
    } else {
        ("CONNECTING", MUTED)
    };
    let title = "円環の理";
    let width = title.width() as u16;
    let left = (area.width - width) / 2;
    text(
        frame,
        Rect::new(area.x, area.y, left, 1),
        "Law of Cycles",
        Style::default().fg(PINK).bold(),
    );
    text(
        frame,
        Rect::new(area.x + left, area.y, width, 1),
        title,
        Style::default().fg(PINK),
    );
    let label = format!("● {badge}");
    let width = label.width() as u16;
    text(
        frame,
        Rect::new(area.right() - width, area.y, width, 1),
        label,
        Style::default().fg(color),
    );
    separator(frame, line_rect(area, 1), Borders::BOTTOM, false);
    let focus = if app.overlay.is_some() {
        "DIALOG".to_owned()
    } else if app.searching {
        format!("SEARCH / {}", app.page.name())
    } else if app.sidebar_focus {
        "NAVIGATION".to_owned()
    } else {
        format!("CONTENT / {}", app.page.name())
    };
    text(
        frame,
        line_rect(area, 2),
        format!(" FOCUS: {focus}"),
        focus_style(true),
    );
}
fn navigation(frame: &mut Frame, app: &mut App, area: Rect) -> Rect {
    if area.width < 78 {
        let slots = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(line_rect(area, 0));
        for (page, rect) in Page::ALL.into_iter().zip(slots.iter().copied()) {
            let name = if page == Page::Connections && rect.width < 15 {
                "Conns"
            } else {
                page.name()
            };
            text(
                frame,
                rect,
                format!("{} {name}", page.index() + 1),
                if app.page == page {
                    current_page_style(app)
                } else {
                    Style::default().fg(MUTED)
                },
            );
            app.view.hits.push(Hit {
                rect,
                intent: Intent::Page(page),
            });
        }
        return Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        );
    }
    let columns = Layout::horizontal([Constraint::Length(20), Constraint::Min(30)])
        .spacing(2)
        .split(area);
    separator(frame, columns[0], Borders::RIGHT, app.sidebar_focus);
    let inner = Rect::new(
        columns[0].x,
        columns[0].y,
        columns[0].width - 2,
        columns[0].height,
    );
    let spacing = if inner.height >= 12 { 2 } else { 1 };
    for page in Page::ALL {
        let rect = line_rect(inner, page.index() as u16 * spacing);
        text(
            frame,
            rect,
            format!(
                "{} {} {}",
                if app.page == page && app.sidebar_focus && app.overlay.is_none() {
                    "›"
                } else {
                    " "
                },
                page.index() + 1,
                page.name()
            ),
            if app.page == page {
                current_page_style(app)
            } else {
                Style::default().fg(MUTED)
            },
        );
        app.view.hits.push(Hit {
            rect,
            intent: Intent::Page(page),
        });
    }
    if inner.height >= 7 {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Type ", Style::default().fg(MUTED)),
                    Span::styled("TAB", accent()),
                ]),
                Line::styled("switch focus", Style::default().fg(MUTED)),
            ]),
            Rect::new(inner.x + 1, inner.bottom() - 2, inner.width - 1, 2),
        );
    }
    columns[1]
}
fn overview(frame: &mut Frame, app: &mut App, area: Rect) {
    page_title(frame, app, line_rect(area, 0), "Overview");
    if let Some(status) = &app.status {
        let version = clean(&format!("mihomo {}", status.version));
        let width = (version.width() as u16).min(area.width.saturating_sub(14));
        frame.render_widget(
            Paragraph::new(version)
                .right_aligned()
                .style(Style::default().fg(MUTED)),
            Rect::new(area.right() - width, area.y, width, 1),
        );
    }
    let compact = area.height < 13;
    let metrics_y = if compact { 1 } else { 2 };
    let metrics = Layout::horizontal([Constraint::Ratio(1, 3); 3])
        .spacing(1)
        .split(Rect::new(
            area.x,
            area.y + metrics_y,
            area.width,
            if compact { 2 } else { 3 },
        ));
    for (rect, label, value, total) in [
        (
            metrics[0],
            "↓ DOWNLOAD",
            format!("{}/s", bytes(app.traffic.down)),
            format!("Total {}", bytes(app.connections.download_total)),
        ),
        (
            metrics[1],
            "↑ UPLOAD",
            format!("{}/s", bytes(app.traffic.up)),
            format!("Total {}", bytes(app.connections.upload_total)),
        ),
        (
            metrics[2],
            "CONNECTIONS",
            app.connections.items().len().to_string(),
            "active connections".into(),
        ),
    ] {
        text(frame, line_rect(rect, 0), label, Style::default().fg(MUTED));
        text(
            frame,
            line_rect(rect, 1),
            value,
            Style::default().fg(FOREGROUND).bold(),
        );
        if !compact {
            text(frame, line_rect(rect, 2), total, Style::default().fg(MUTED));
        }
    }
    let (mode, tun) = app
        .status
        .as_ref()
        .map(|s| (s.mode.as_str(), if s.tun { "Enabled" } else { "Disabled" }))
        .unwrap_or(("unknown", "unknown"));
    if compact {
        text(
            frame,
            line_rect(area, 3),
            format!("Mode {mode}   TUN {tun}"),
            Style::default().fg(MUTED),
        );
        text(
            frame,
            line_rect(area, 4),
            &app.endpoint,
            Style::default().fg(MUTED),
        );
    } else {
        let history: Vec<_> = app.down_history.iter().copied().collect();
        text(
            frame,
            line_rect(area, 6),
            "DOWNLOAD HISTORY",
            Style::default().fg(MUTED),
        );
        let chart_height = if area.height >= 21 { 3 } else { 1 };
        frame.render_widget(
            Sparkline::default()
                .data(&history)
                .style(Style::default().fg(ACCENT)),
            Rect::new(area.x, area.y + 7, area.width, chart_height),
        );
        let y = area.y + 8 + chart_height;
        let info_height = if area.height >= 18 { 4 } else { 3 };
        let columns = Layout::horizontal([Constraint::Ratio(1, 2); 2])
            .spacing(2)
            .split(Rect::new(area.x, y, area.width, info_height));
        text(
            frame,
            line_rect(columns[0], 0),
            "RUNTIME",
            Style::default().fg(MUTED),
        );
        text(
            frame,
            line_rect(columns[0], 1),
            format!("Routing mode  {mode}"),
            Style::default().fg(ACCENT),
        );
        text(
            frame,
            line_rect(columns[0], 2),
            format!("TUN           {tun}"),
            Style::default().fg(if app.status.as_ref().is_some_and(|s| s.tun) {
                GOOD
            } else {
                MUTED
            }),
        );
        text(
            frame,
            line_rect(columns[1], 0),
            "CONTROLLER",
            Style::default().fg(MUTED),
        );
        text(
            frame,
            line_rect(columns[1], 1),
            &app.endpoint,
            Style::default().fg(FOREGROUND),
        );
        text(
            frame,
            line_rect(columns[1], 2),
            if app.demo {
                "Demo · no external effects"
            } else if app.errors.contains_key(&Topic::Status) {
                "Offline · last known data"
            } else if app.status.is_some() {
                "Online"
            } else {
                "Connecting…"
            },
            Style::default().fg(if app.errors.contains_key(&Topic::Status) {
                WARNING
            } else {
                GOOD
            }),
        );
        if info_height >= 4 {
            let ports = app
                .status
                .as_ref()
                .map(|s| {
                    [
                        ("MIX", s.mixed_port),
                        ("HTTP", s.port),
                        ("SOCKS", s.socks_port),
                    ]
                    .into_iter()
                    .filter(|(_, port)| *port != 0)
                    .map(|(label, port)| format!("{label} {port}"))
                    .collect::<Vec<_>>()
                    .join(" · ")
                })
                .unwrap_or_default();
            text(
                frame,
                line_rect(columns[0], 3),
                if ports.is_empty() {
                    "Ports: none reported".into()
                } else {
                    ports
                },
                Style::default().fg(MUTED),
            );
            text(
                frame,
                line_rect(columns[1], 3),
                format!("Read errors: {}", app.errors.len()),
                Style::default().fg(if app.errors.is_empty() {
                    MUTED
                } else {
                    WARNING
                }),
            );
        }
        let summary_y = y + info_height + 1;
        let available = area.bottom().saturating_sub(summary_y + 1);
        if available >= 2 {
            text(
                frame,
                Rect::new(area.x, summary_y, area.width, 1),
                if app.errors.contains_key(&Topic::Proxies) {
                    "PROXY GROUPS · stale"
                } else {
                    "PROXY GROUPS"
                },
                Style::default().fg(MUTED),
            );
            let groups: Vec<_> = app
                .proxies
                .iter()
                .filter(|(_, proxy)| proxy.all.is_some())
                .collect();
            let budget = available.saturating_sub(if available >= 5 { 3 } else { 1 });
            for (offset, (name, proxy)) in groups.iter().take(budget as usize).enumerate() {
                text(
                    frame,
                    Rect::new(area.x, summary_y + 1 + offset as u16, area.width, 1),
                    format!(
                        "{name}  →  {}  · {}",
                        if proxy.now.is_empty() {
                            "—"
                        } else {
                            &proxy.now
                        },
                        proxy.kind
                    ),
                    Style::default().fg(ACCENT),
                );
            }
            if groups.is_empty() {
                text(
                    frame,
                    Rect::new(area.x, summary_y + 1, area.width, 1),
                    "No proxy groups reported",
                    Style::default().fg(MUTED),
                );
            }
            if available >= 5 {
                let tcp = app
                    .connections
                    .items()
                    .iter()
                    .filter(|c| c.metadata.network.eq_ignore_ascii_case("tcp"))
                    .count();
                let udp = app
                    .connections
                    .items()
                    .iter()
                    .filter(|c| c.metadata.network.eq_ignore_ascii_case("udp"))
                    .count();
                text(
                    frame,
                    line_rect(area, area.height - 3),
                    format!(
                        "Connections  TCP {tcp} · UDP {udp} · Other {}",
                        app.connections.items().len().saturating_sub(tcp + udp)
                    ),
                    Style::default().fg(MUTED),
                );
                let recent = app.logs.iter().rev().find(|(_, log)| {
                    matches!(
                        log.level.to_lowercase().as_str(),
                        "warn" | "warning" | "error" | "fatal"
                    )
                });
                text(
                    frame,
                    line_rect(area, area.height - 2),
                    recent
                        .map(|(_, log)| format!("Recent {}: {}", log.level, log.payload))
                        .unwrap_or_else(|| {
                            format!(
                                "Logs: {} buffered · no warning/error in buffer",
                                app.logs.len()
                            )
                        }),
                    Style::default().fg(if recent.is_some() { WARNING } else { MUTED }),
                );
            }
        }
    }
    button(
        frame,
        app,
        (area.x, area.bottom() - 1),
        "Runtime actions a",
        Intent::Actions,
        area.width,
        true,
    );
}
fn action_bar(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let y = area.bottom() - 1;
    let used = button(
        frame,
        app,
        (area.x, y),
        "Actions a",
        Intent::Actions,
        area.width,
        true,
    );
    if let Some(item) = app.actions().first() {
        button(
            frame,
            app,
            (area.x + used, y),
            &item.label,
            item.intent.clone(),
            area.width.saturating_sub(used),
            item.disabled.is_none() && !app.busy,
        );
    }
}
fn browser(frame: &mut Frame, app: &mut App, area: Rect) {
    app.reconcile();
    let title = if let Some(group) = &app.group {
        format!("Proxies / {group}")
    } else {
        app.page.name().into()
    };
    let action = if app.page == Page::Proxies && app.group.is_some() {
        Some((
            if app.refresh_progress.is_some() {
                "Refreshing…"
            } else {
                "Refresh group"
            },
            Intent::RefreshGroup,
            !app.busy,
        ))
    } else if app.page == Page::Logs {
        Some(("Clean", Intent::CleanLogs, !app.logs.is_empty()))
    } else {
        None
    };
    let reserved = action
        .as_ref()
        .map_or(0, |(label, _, _)| label.width() as u16 + 5);
    page_title(
        frame,
        app,
        Rect::new(area.x, area.y, area.width.saturating_sub(reserved), 1),
        &title,
    );
    if let Some((label, intent, enabled)) = action {
        let width = label.width() as u16 + 4;
        button(
            frame,
            app,
            (area.right() - width, area.y),
            label,
            intent,
            width,
            enabled,
        );
    }
    let rows = app.rows();
    let count = if app.page == Page::Logs {
        format!(
            "{} · {} entries",
            if app.follow { "Live" } else { "Paused" },
            rows.len()
        )
    } else {
        format!("{} items", rows.len())
    };
    let count_width = count.width() as u16;
    let search_rect = Rect::new(
        area.x,
        area.y + 1,
        area.width.saturating_sub(count_width + 1),
        1,
    );
    let mut visible_query = String::new();
    let mut query_width = 0;
    for ch in app.query.chars().rev() {
        let width = ch.width().unwrap_or(0) as u16;
        if query_width + width > search_rect.width.saturating_sub(3) {
            break;
        }
        visible_query.push(ch);
        query_width += width;
    }
    let visible_query: String = visible_query.chars().rev().collect();
    let search = format!(
        "/ {}",
        if app.query.is_empty() && !app.searching {
            "Search…"
        } else {
            &visible_query
        }
    );
    text(frame, search_rect, search, focus_style(app.searching));
    if app.searching && app.overlay.is_none() && search_rect.width >= 3 {
        frame.set_cursor_position((search_rect.x + 2 + query_width, search_rect.y));
    }
    app.view.hits.push(Hit {
        rect: search_rect,
        intent: Intent::Search,
    });
    text(
        frame,
        Rect::new(area.right() - count_width, area.y + 1, count_width, 1),
        count,
        Style::default().fg(MUTED),
    );
    let content_offset = if area.height < 10 { 2 } else { 3 };
    let mut content = Rect::new(
        area.x,
        area.y + content_offset,
        area.width,
        area.height.saturating_sub(content_offset),
    );
    if app.page == Page::Proxies && app.group.is_some() {
        let back = line_rect(content, 0);
        text(
            frame,
            back,
            "..  Back to groups",
            Style::default().fg(ACCENT),
        );
        app.view.hits.push(Hit {
            rect: back,
            intent: Intent::Back,
        });
        content.y += 1;
        content.height = content.height.saturating_sub(1);
    }
    let mut list_rect = content;
    let mut inspector = None;
    let mut connection_detail = None;
    if app.page == Page::Connections {
        let detail_height = if content.height >= 12 {
            8
        } else if content.height >= 8 {
            5
        } else {
            2
        };
        list_rect.height = content.height.saturating_sub(detail_height);
        connection_detail = Some(Rect::new(
            content.x,
            list_rect.bottom(),
            content.width,
            detail_height,
        ));
    } else if content.width >= 76 && app.page == Page::Proxies {
        let width = ((u32::from(content.width) * u32::from(app.split)) / 100) as u16;
        let width = width.clamp(30, content.width - 28);
        list_rect.width = width;
        let divider = Rect::new(content.x + width, content.y, 1, content.height);
        app.view.divider = Some(divider);
        separator(frame, divider, Borders::LEFT, false);
        inspector = Some(Rect::new(
            content.x + width + 2,
            content.y,
            content.width - width - 2,
            content.height,
        ));
    }
    let headers = match app.page {
        Page::Proxies if app.group.is_some() => vec!["NODE", "TYPE", "LATENCY"],
        Page::Proxies => vec!["GROUP", "TYPE", "CURRENT"],
        Page::Connections if app.show_process => vec!["PROCESS", "DESTINATION", "ROUTE"],
        Page::Connections => vec!["DESTINATION", "ROUTE"],
        _ => vec!["LEVEL", "MESSAGE"],
    };
    let constraints = match app.page {
        Page::Logs => vec![Constraint::Length(8), Constraint::Min(10)],
        Page::Connections if !app.show_process => {
            vec![Constraint::Percentage(65), Constraint::Percentage(35)]
        }
        Page::Proxies if app.group.is_some() => vec![
            Constraint::Min(10),
            Constraint::Length(if list_rect.width >= 50 { 12 } else { 9 }),
            Constraint::Length(9),
        ],
        _ => vec![
            Constraint::Percentage(42),
            Constraint::Percentage(29),
            Constraint::Percentage(29),
        ],
    };
    let items: Vec<_> = rows
        .iter()
        .map(|(_, cells)| {
            Row::new(
                cells
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| {
                        let color = if app.page == Page::Logs && index == 0 {
                            match cell.to_lowercase().as_str() {
                                "error" | "fatal" => ERROR,
                                "warning" | "warn" => WARNING,
                                "info" => GOOD,
                                _ => MUTED,
                            }
                        } else if app.page == Page::Proxies && app.group.is_some() && index == 2 {
                            match cell
                                .split_whitespace()
                                .next()
                                .and_then(|v| v.parse::<u64>().ok())
                            {
                                Some(0) => ERROR,
                                Some(1..=100) => GOOD,
                                Some(_) => WARNING,
                                None if cell == "Failed" => ERROR,
                                None => MUTED,
                            }
                        } else {
                            FOREGROUND
                        };
                        Cell::from(clean(cell)).style(Style::default().fg(color))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let table_rect = Rect::new(
        list_rect.x,
        list_rect.y,
        list_rect.width,
        list_rect
            .height
            .saturating_sub(if connection_detail.is_some() {
                0
            } else if area.height < 10 {
                1
            } else {
                2
            }),
    );
    let table = Table::new(items, constraints)
        .column_spacing(1)
        .header(Row::new(headers).style(Style::default().fg(MUTED)))
        .row_highlight_style(if content_focused(app) {
            selected().bold()
        } else {
            Style::default().fg(MUTED).bg(Color::Rgb(27, 25, 36))
        })
        .highlight_symbol(if content_focused(app) { "› " } else { "· " });
    let index = rows
        .iter()
        .position(|row| Some(&row.0) == app.selected.as_ref());
    let mut state = TableState::default()
        .with_selected(index)
        .with_offset(app.offset);
    frame.render_stateful_widget(table, table_rect, &mut state);
    app.offset = state.offset();
    app.view.list = Rect::new(
        table_rect.x,
        table_rect.y + 1,
        table_rect.width,
        table_rect.height.saturating_sub(1),
    );
    for (offset, (identity, _)) in rows
        .iter()
        .skip(app.offset)
        .take(app.view.list.height as usize)
        .enumerate()
    {
        app.view.hits.push(Hit {
            rect: Rect::new(
                app.view.list.x,
                app.view.list.y + offset as u16,
                app.view.list.width,
                1,
            ),
            intent: Intent::Row(identity.clone()),
        });
    }
    if rows.is_empty() && app.view.list.height > 0 {
        text(
            frame,
            app.view.list,
            if !app.query.is_empty() {
                "No matching items."
            } else if app.page == Page::Logs {
                "Waiting for new entries…"
            } else {
                "Waiting for items…"
            },
            Style::default().fg(MUTED),
        );
    }
    if let Some(rect) = connection_detail {
        connection_details(frame, app, rect);
    } else if let Some(rect) = inspector {
        text(
            frame,
            line_rect(rect, 0),
            if app.page == Page::Proxies {
                "SELECTED NODE / GROUP"
            } else {
                "SELECTED CONNECTION"
            },
            Style::default().fg(MUTED),
        );
        let details = app.details();
        if let Some(first) = details.first() {
            text(frame, line_rect(rect, 2), first, Style::default().fg(PINK));
        }
        paragraph(
            frame,
            Rect::new(
                rect.x,
                rect.y + 3,
                rect.width,
                rect.height.saturating_sub(6),
            ),
            details.into_iter().skip(1).collect(),
            0,
        );
        action_bar(frame, app, rect);
        if app.page == Page::Proxies && app.group.is_some() {
            text(
                frame,
                line_rect(list_rect, list_rect.height.saturating_sub(1)),
                "● in use   › selected",
                Style::default().fg(MUTED),
            );
        }
    } else {
        action_bar(frame, app, list_rect);
    }
}
fn connection_details(frame: &mut Frame, app: &mut App, area: Rect) {
    let connection = app
        .connections
        .items()
        .iter()
        .find(|item| Some(&item.id) == app.selected.as_ref());
    if area.height <= 2 {
        let summary = connection
            .map(|item| {
                format!(
                    "Selected: {}:{} · {}",
                    if item.metadata.host.is_empty() {
                        &item.metadata.destination_ip
                    } else {
                        &item.metadata.host
                    },
                    item.metadata.destination_port,
                    item.metadata.network
                )
            })
            .unwrap_or_else(|| "Select a connection to inspect it.".into());
        text(
            frame,
            line_rect(area, 0),
            summary,
            Style::default().fg(PINK),
        );
        action_bar(frame, app, area);
        return;
    }
    frame.render_widget(
        Block::new()
            .borders(Borders::TOP)
            .title(" SELECTED CONNECTION ")
            .border_style(Style::default().fg(BORDER))
            .title_style(Style::default().fg(MUTED)),
        line_rect(area, 0),
    );
    if let Some(item) = connection {
        let destination = format!(
            "{}:{}",
            if item.metadata.host.is_empty() {
                &item.metadata.destination_ip
            } else {
                &item.metadata.host
            },
            item.metadata.destination_port
        );
        let content = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(2),
        );
        if area.width >= 70 {
            let columns = Layout::horizontal([Constraint::Ratio(1, 2); 2])
                .spacing(2)
                .split(content);
            let mut identity = vec![
                destination,
                format!("Network: {}", item.metadata.network),
                format!("Rule: {} {}", item.rule, item.rule_payload),
                format!(
                    "Source: {}",
                    if item.metadata.source_ip.is_empty() {
                        "—"
                    } else {
                        &item.metadata.source_ip
                    }
                ),
            ];
            if app.show_process {
                identity.insert(2, format!("Process: {}", item.metadata.process));
            }
            let traffic = [
                format!("Route: {}", item.chains.join(" > ")),
                format!("Upload: {}", bytes(item.upload)),
                format!("Download: {}", bytes(item.download)),
                format!("ID: {}", item.id),
                format!(
                    "IP: {}",
                    if item.metadata.destination_ip.is_empty() {
                        "—"
                    } else {
                        &item.metadata.destination_ip
                    }
                ),
            ];
            for (column, lines) in [
                (columns[0], identity.as_slice()),
                (columns[1], traffic.as_slice()),
            ] {
                for (row, line) in lines.iter().take(column.height as usize).enumerate() {
                    text(
                        frame,
                        line_rect(column, row as u16),
                        line,
                        Style::default().fg(if row == 0 { PINK } else { FOREGROUND }),
                    );
                }
            }
        } else {
            let mut lines = vec![
                destination,
                format!("Route: {}", item.chains.join(" > ")),
                format!(
                    "{} · ↑ {} · ↓ {}",
                    item.metadata.network,
                    bytes(item.upload),
                    bytes(item.download)
                ),
                format!("Rule: {} {}", item.rule, item.rule_payload),
            ];
            if app.show_process {
                lines.push(format!("Process: {}", item.metadata.process));
            }
            for (row, line) in lines.iter().take(content.height as usize).enumerate() {
                text(
                    frame,
                    line_rect(content, row as u16),
                    line,
                    Style::default().fg(if row == 0 { PINK } else { FOREGROUND }),
                );
            }
        }
    } else {
        text(
            frame,
            line_rect(area, 1),
            "Select a connection to inspect it.",
            Style::default().fg(MUTED),
        );
    }
    action_bar(frame, app, area);
}
fn footer(frame: &mut Frame, app: &mut App, area: Rect) {
    separator(frame, line_rect(area, 0), Borders::TOP, false);
    let message = if app.searching {
        "Editing search · Ctrl+C clears · Esc leaves input".into()
    } else if !app.errors.is_empty() {
        app.errors
            .iter()
            .map(|(topic, error)| format!("{}: {error}", topic.name()))
            .collect::<Vec<_>>()
            .join("; ")
    } else if let Some((completed, total)) = app.refresh_progress {
        format!("Refreshing group · {completed} / {total}")
    } else {
        format!("{}{}", if app.busy { "Working: " } else { "" }, app.message)
    };
    let rect = line_rect(area, 1);
    text(
        frame,
        rect,
        message,
        Style::default().fg(if app.busy || !app.errors.is_empty() {
            WARNING
        } else {
            MUTED
        }),
    );
    app.view.hits.push(Hit {
        rect,
        intent: Intent::Result,
    });
    let hints = if app.searching {
        "Type to filter · q types text · Esc then q quits"
    } else if app.sidebar_focus {
        "↑↓ choose page  Enter/Tab content  ? help  q quit"
    } else if area.width < 60 {
        "1–4 pages  / search  a actions  ? help  q quit"
    } else if app.page == Page::Logs {
        "1–4 pages  / search  c clean  a actions  q quit"
    } else if app.page == Page::Proxies && app.group.is_some() {
        "1–4 pages  / search  r refresh group  a actions  q quit"
    } else {
        "1–4 pages  Tab focus  / search  a actions  ? help  q quit"
    };
    text(frame, line_rect(area, 2), hints, Style::default().fg(MUTED));
}
fn overlay(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(overlay) = app.overlay.clone() else {
        return;
    };
    let width = area.width.saturating_sub(4).min(76);
    let desired = match &overlay {
        Overlay::Menu { items, .. } => items.len() as u16 + 4,
        Overlay::Confirm { operation, .. } => (operation.label().width() as u16 / width.max(1)) + 8,
        Overlay::Document { .. } => area.height.saturating_sub(6),
    };
    let height = desired.max(7).min(area.height.saturating_sub(4));
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    app.view.popup = Some(rect);
    frame.render_widget(Clear, rect);
    let title = match &overlay {
        Overlay::Menu { title, .. } | Overlay::Document { title, .. } => title.as_str(),
        Overlay::Confirm { .. } => "Confirm action",
    };
    let border = block(title, true);
    let inner = border.inner(rect).inner(Margin::new(1, 0));
    frame.render_widget(border, rect);
    match overlay {
        Overlay::Menu {
            items,
            selected: index,
            ..
        } => {
            let size = inner.height.saturating_sub(2) as usize;
            let start = index.saturating_sub(size.saturating_sub(1));
            let list: Vec<_> = items
                .iter()
                .map(|item| {
                    ListItem::new(format!(
                        "{}{}",
                        item.label,
                        if item.disabled.is_some() {
                            " (unavailable)"
                        } else {
                            ""
                        }
                    ))
                    .style(Style::default().fg(if item.disabled.is_some() {
                        MUTED
                    } else {
                        FOREGROUND
                    }))
                })
                .collect();
            let mut state = ListState::default()
                .with_selected(Some(index))
                .with_offset(start);
            frame.render_stateful_widget(
                List::new(list).highlight_style(selected()),
                Rect::new(inner.x, inner.y, inner.width, size as u16),
                &mut state,
            );
            for (row, _) in items.iter().enumerate().skip(state.offset()).take(size) {
                app.view.hits.push(Hit {
                    rect: Rect::new(
                        inner.x,
                        inner.y + (row - state.offset()) as u16,
                        inner.width,
                        1,
                    ),
                    intent: Intent::MenuItem(row),
                });
            }
            text(
                frame,
                Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
                items[index]
                    .disabled
                    .clone()
                    .unwrap_or_else(|| "Click / Enter: run   Esc: cancel".into()),
                Style::default().fg(MUTED),
            );
            button(
                frame,
                app,
                (inner.x, inner.bottom() - 1),
                "Close",
                Intent::Dismiss,
                inner.width,
                true,
            );
        }
        Overlay::Confirm { operation, yes } => {
            paragraph(
                frame,
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(3),
                ),
                vec![
                    operation.label(),
                    String::new(),
                    "This acts on the target shown above.".into(),
                ],
                0,
            );
            let y = inner.bottom() - 1;
            for (offset, label, value) in [(0, " Cancel ", false), (13, " Confirm ", true)] {
                let rect = Rect::new(inner.x + offset, y, 11, 1);
                text(
                    frame,
                    rect,
                    label,
                    if yes == value { selected() } else { accent() },
                );
                app.view.hits.push(Hit {
                    rect,
                    intent: Intent::Confirm(value),
                });
            }
        }
        Overlay::Document { lines, scroll, .. } => {
            let content = Rect::new(
                inner.x,
                inner.y,
                inner.width,
                inner.height.saturating_sub(2),
            );
            let paragraph = Paragraph::new(
                lines
                    .iter()
                    .map(|line| Line::from(clean(line)))
                    .collect::<Vec<_>>(),
            )
            .wrap(Wrap { trim: false });
            let max = paragraph
                .line_count(content.width)
                .saturating_sub(content.height as usize)
                .min(u16::MAX as usize) as u16;
            let scroll = scroll.min(max);
            if let Some(Overlay::Document { scroll: value, .. }) = app.overlay.as_mut() {
                *value = scroll;
            }
            frame.render_widget(paragraph.scroll((scroll, 0)), content);
            button(
                frame,
                app,
                (inner.x, inner.bottom() - 1),
                "Close",
                Intent::Dismiss,
                inner.width,
                true,
            );
        }
    }
}
