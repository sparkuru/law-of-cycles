use crate::{
    app::{App, Effect},
    backend::Backend,
    model::{Operation, Topic, Update},
    ui,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, time::Duration};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};

struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("Cannot enable terminal raw mode")?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)
            .context("Cannot initialize terminal")?;
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}
fn restore() {
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

pub async fn run(backend: Backend) -> Result<()> {
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        old_hook(info);
    }));
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .context("Cannot create terminal renderer")?;
    terminal.clear()?;
    let mut app = App::new(backend.is_demo(), backend.endpoint());
    app.show_process = backend.show_process();
    let (tx, mut rx) = mpsc::channel(128);
    let (refresh, _) = watch::channel(0_u64);
    let mut tasks = JoinSet::new();
    for topic in [Topic::Status, Topic::Proxies, Topic::Connections] {
        let (backend, tx, mut refresh) = (backend.clone(), tx.clone(), refresh.subscribe());
        tasks.spawn(async move {
            loop {
                let update = match backend.fetch(topic).await {
                    Ok(data) => Update::Data(topic, data),
                    Err(error) => Update::Error(topic, error.to_string()),
                };
                if tx.send(update).await.is_err() {
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                    changed = refresh.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
    for topic in [Topic::Logs, Topic::Traffic] {
        let (backend, tx) = (backend.clone(), tx.clone());
        tasks.spawn(async move {
            backend.follow(topic, "info", tx).await;
        });
    }
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut dirty = true;
    while !app.quit {
        tokio::select! {
            _ = terminate.recv() => break,
            event = events.next() => {
                let Some(event) = event else { break; };
                let effect = match event.context("Terminal input failed")? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                    Event::Mouse(mouse) => app.mouse(mouse),
                    Event::Resize(_, _) => {
                        app.view.hits.clear();
                        None
                    },
                    _ => None,
                };
                if let Some(effect) = effect {
                    dispatch(effect, &backend, &tx, &refresh, &mut tasks);
                }
                terminal.draw(|frame| ui::draw(frame, &mut app))?;
                dirty = false;
            }
            Some(update) = rx.recv() => {
                app.update(update);
                dirty = true;
            }
            Some(result) = tasks.join_next() => if result.is_err() {
                app.message = "A background task failed; restart kami to reconnect.".into();
                app.busy = false;
                app.refresh_progress = None;
                dirty = true;
            },
            _ = tick.tick() => if dirty {
                terminal.draw(|frame| ui::draw(frame, &mut app))?;
                dirty = false;
            }
        }
    }
    tasks.abort_all();
    Ok(())
}
fn dispatch(
    effect: Effect,
    backend: &Backend,
    tx: &mpsc::Sender<Update>,
    refresh: &watch::Sender<u64>,
    tasks: &mut JoinSet<()>,
) {
    match effect {
        Effect::Refresh => refresh.send_modify(|value| *value = value.wrapping_add(1)),
        Effect::Run(operation) => {
            let (backend, tx, refresh) = (backend.clone(), tx.clone(), refresh.clone());
            tasks.spawn(async move {
                let result = match &operation {
                    Operation::RefreshGroup(group) => backend.refresh_group(group, Some(&tx)).await,
                    _ => backend.execute(&operation).await,
                }
                .map_err(|error| error.to_string());
                if let Operation::Delay(node) = &operation {
                    let delay = result
                        .as_ref()
                        .ok()
                        .and_then(|value| value["delay"].as_u64())
                        .filter(|delay| *delay > 0);
                    let _ = tx
                        .send(Update::Latency {
                            node: node.clone(),
                            delay,
                        })
                        .await;
                }
                let _ = tx.send(Update::Finished(operation.label(), result)).await;
                refresh.send_modify(|value| *value = value.wrapping_add(1));
            });
        }
    }
}
