//! Read-only live dashboard. All state is obtained through the canonical IPC service.
use crate::{
    Error, Result,
    protocol::{Operation, PROTOCOL_VERSION, Request},
    runtime,
};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};
use serde_json::Value;
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::watch;

const TABS: [&str; 7] = [
    "Overview",
    "Tasks",
    "Workers",
    "Personas",
    "Continuity",
    "Accounts",
    "Activity",
];
const PAGE_CAP: usize = 1000;

#[derive(Clone, Default)]
struct Snapshot {
    health: Value,
    projects: Vec<Value>,
    tasks: Vec<Value>,
    workers: Vec<Value>,
    personas: Vec<Value>,
    handoffs: Vec<Value>,
    accounts: Vec<Value>,
    activity: Vec<Value>,
    project: usize,
    error: Option<String>,
    refreshed: String,
}

async fn call(dir: &Path, operation: Operation) -> Result<Value> {
    let response = runtime::call(
        dir,
        &Request {
            version: PROTOCOL_VERSION,
            request_id: uuid::Uuid::new_v4().to_string(),
            operation,
        },
    )
    .await?;
    if let Some(error) = response.error {
        return Err(Error::Unavailable(error.message));
    }
    response
        .result
        .ok_or_else(|| Error::Unavailable("empty service response".into()))
}

async fn pages(dir: &Path, operation: impl Fn(u64) -> Operation) -> Result<Vec<Value>> {
    let mut after = 0;
    let mut items = Vec::new();
    loop {
        let page = call(dir, operation(after)).await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        match page["next_cursor"].as_u64() {
            Some(next) if items.len() < PAGE_CAP && next > after => after = next,
            _ => break,
        }
    }
    Ok(items)
}

async fn snapshot(dir: &Path, project: usize) -> Result<Snapshot> {
    let health = call(dir, Operation::Health).await?;
    let projects = pages(dir, |after| Operation::ProjectsList { after, limit: 100 }).await?;
    let project = project.min(projects.len().saturating_sub(1));
    let id = projects
        .get(project)
        .and_then(|p| p["id"].as_str())
        .map(str::to_owned);
    let mut data = Snapshot {
        health,
        projects,
        project,
        refreshed: "live · refresh every second".into(),
        ..Default::default()
    };
    if let Some(id) = &id {
        data.tasks = pages(dir, |after| Operation::TasksList {
            project_id: id.clone(),
            state: None,
            after,
            limit: 100,
        })
        .await?;
        data.handoffs = pages(dir, |after| Operation::HandoffsList {
            project_id: id.clone(),
            after,
            limit: 100,
        })
        .await?;
    }
    data.workers = pages(dir, |after| Operation::WorkerExecutionsList {
        task_id: None,
        project_id: id.clone(),
        status: None,
        after,
        limit: 100,
    })
    .await?;
    data.personas = pages(dir, |after| Operation::PersonasList {
        status: None,
        project_id: id.clone(),
        after,
        limit: 100,
    })
    .await?;
    data.accounts = pages(dir, |after| Operation::GithubIdentitiesList {
        status: None,
        persona_id: None,
        after,
        limit: 100,
    })
    .await?;
    data.activity = pages(dir, |after| Operation::ActivityGet {
        project_id: id.clone(),
        after,
        limit: 100,
    })
    .await?;
    data.workers.reverse();
    data.activity.reverse();
    Ok(data)
}

fn field(value: &Value, name: &str) -> String {
    match &value[name] {
        Value::String(s) => s.chars().filter(|c| !c.is_control()).take(4096).collect(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => "—".into(),
    }
}

fn heading(label: &str, value: impl std::fmt::Display) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<15}"), Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}

fn worker_detail(worker: &Value) -> Vec<Line<'static>> {
    let provider = &worker["result"]["provider"];
    let mut lines = vec![
        heading("Worker", field(worker, "worker_label")),
        heading("Adapter", field(worker, "adapter")),
        heading("Execution", field(worker, "id")),
        heading("State", field(worker, "status")),
        heading("Task state", field(worker, "task_state")),
        heading("Process", field(&worker["process"], "status")),
        heading("Exit code", field(&worker["process"], "exit_code")),
        Line::raw(""),
        heading("Worktree", field(&worker["workspace"], "path")),
        heading("Input tokens", field(&provider["tokens"], "input")),
        heading("Output tokens", field(&provider["tokens"], "output")),
        heading("Tool calls", field(provider, "tool_calls")),
    ];
    if worker["adapter"] == "codex" {
        lines.push(Line::raw("Codex subscription · no dollar estimate"));
    }
    if let Some(changes) = provider["workspace_changes"].as_array() {
        for change in changes {
            lines.push(Line::raw(format!(
                "  {}  {}",
                field(change, "status"),
                field(change, "path")
            )));
        }
    }
    if let Some(evidence) = worker["process"]["evidence"].as_array() {
        for item in evidence.iter().take(8) {
            lines.push(heading(
                "File evidence",
                format!("{} · {}", field(item, "path"), field(item, "status")),
            ));
            lines.push(heading("SHA-256", field(item, "sha256")));
        }
    }
    lines.extend([
        Line::raw(""),
        Line::styled(
            "Successful exit still requires review.",
            Style::default().fg(Color::Yellow),
        ),
        Line::raw("File evidence does not certify tests or correctness."),
    ]);
    lines
}

fn draw(frame: &mut Frame, data: &Snapshot, tab: usize, selected: usize, scroll: u16) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let online = if data.error.is_some() {
        "OFFLINE / STALE"
    } else {
        "CONNECTED"
    };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " MOTHER ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {online}   •   Codex workers   •   local state")),
    ]))
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(title, outer[0]);
    frame.render_widget(
        Tabs::new(TABS.to_vec()).select(tab).highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        outer[1],
    );
    let project = data
        .projects
        .get(data.project)
        .cloned()
        .unwrap_or(Value::Null);
    let name = if project.is_null() {
        "No project attached".into()
    } else {
        field(&project, "name")
    };
    let area = outer[2];
    let items = match tab {
        1 => &data.tasks,
        2 => &data.workers,
        3 => &data.personas,
        4 => &data.handoffs,
        5 => &data.accounts,
        6 => &data.activity,
        _ => &data.projects,
    };
    if tab == 0 {
        let mut lines = vec![
            Line::styled(
                name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            heading("Repository", field(&project, "repository")),
            Line::raw(""),
            heading("Projects", data.projects.len()),
            heading("Tasks", data.tasks.len()),
            heading("Workers", data.workers.len()),
            heading(
                "Active workers",
                data.workers.iter().filter(|w| w["active"] == true).count(),
            ),
            heading("Personas", data.personas.len()),
            heading("Handoffs", data.handoffs.len()),
            heading("Schema", field(&data.health, "schema_version")),
            Line::raw(""),
            Line::raw("Mother remembers tasks, worktrees, decisions and handoffs."),
            Line::raw("Codex performs work using your existing host login."),
            Line::raw(""),
            Line::styled("Get started", Style::default().fg(Color::Cyan)),
            Line::raw("motherctl setup-codex     Connect this project to Codex"),
            Line::raw("motherctl tasks --help    Create and manage a task"),
            Line::raw("motherctl workers run --adapter codex --task <id> --worker-label codex"),
            Line::raw(""),
            Line::raw("This first dashboard is read-only. Use Codex MCP or CLI to change state."),
        ];
        if let Some(worker) = data.workers.first() {
            lines.push(Line::raw(""));
            lines.push(heading(
                "Latest worker",
                format!("{} / {}", field(worker, "adapter"), field(worker, "status")),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Live workspace "),
                ),
            area,
        );
    } else {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);
        let rows: Vec<ListItem> = items
            .iter()
            .map(|item| {
                let label = match tab {
                    1 => format!("{}  {}", field(item, "state"), field(item, "title")),
                    2 => format!(
                        "{}  {}  {}",
                        field(item, "adapter"),
                        field(item, "status"),
                        field(item, "worker_label")
                    ),
                    3 => format!("{}  {}", field(item, "name"), field(item, "status")),
                    4 => format!("{}  {}", field(item, "id"), field(item, "created_at")),
                    5 => format!("{}  {}", field(item, "login"), field(item, "status")),
                    _ => format!("{}  {}", field(item, "action"), field(item, "created_at")),
                };
                ListItem::new(label)
            })
            .collect();
        let mut state = ListState::default();
        if !items.is_empty() {
            state.select(Some(selected.min(items.len() - 1)));
        }
        frame.render_stateful_widget(
            List::new(rows)
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::Cyan))
                .highlight_symbol("› ")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {name} · {} (cap {PAGE_CAP}) ", items.len())),
                ),
            columns[0],
            &mut state,
        );
        let item = items
            .get(selected.min(items.len().saturating_sub(1)))
            .cloned()
            .unwrap_or(Value::Null);
        let mut lines = if item.is_null() {
            vec![
                Line::raw("No records yet."),
                Line::raw("Create work through Codex MCP or motherctl."),
            ]
        } else if tab == 2 {
            worker_detail(&item)
        } else {
            let keys: &[&str] = match tab {
                1 => &["title", "id", "state", "task_revision", "updated_at"],
                3 => &["name", "id", "role", "status", "revision"],
                4 => &["id", "objective", "source", "created_at", "sha256"],
                5 => &["login", "id", "status", "persona_id", "verified_at"],
                _ => &[
                    "action",
                    "sequence",
                    "actor",
                    "source",
                    "object_id",
                    "result",
                    "created_at",
                ],
            };
            keys.iter()
                .map(|key| heading(key, field(&item, key)))
                .collect()
        };
        if tab == 1
            && let Some(worker) = data.workers.iter().find(|w| w["task_id"] == item["id"])
        {
            lines.push(Line::raw(""));
            lines.extend(worker_detail(worker));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .block(Block::default().borders(Borders::ALL).title(" Details ")),
            columns[1],
        );
    }
    let footer = if let Some(error) = &data.error {
        format!(" {error} · q quit")
    } else {
        format!(
            " Tab/←/→ views · ↑/↓ select · j/k scroll · p project · q quit | {}",
            data.refreshed
        )
    };
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().fg(if data.error.is_some() {
            Color::Yellow
        } else {
            Color::DarkGray
        })),
        outer[3],
    );
}

pub async fn run(dir: PathBuf, once: bool) -> Result<()> {
    let first = snapshot(&dir, 0).await?;
    if once {
        let backend = TestBackend::new(120, 35);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|never| match never {});
        terminal
            .draw(|frame| draw(frame, &first, 0, 0, 0))
            .unwrap_or_else(|never| match never {});
        let buffer = terminal.backend().buffer();
        for y in 0..35 {
            let row: String = (0..120).map(|x| buffer[(x, y)].symbol()).collect();
            println!("{}", row.trim_end());
        }
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::Invalid(
            "ui requires a terminal; use ui --once for a text snapshot".into(),
        ));
    }
    let (tx, rx) = watch::channel(first);
    let (project_tx, mut project_rx) = watch::channel(0usize);
    let poll = tokio::spawn(async move {
        loop {
            let project = *project_rx.borrow_and_update();
            match snapshot(&dir, project).await {
                Ok(data) => {
                    if tx.send(data).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tx.send_modify(|data| {
                        data.error = Some(error.to_string());
                        data.refreshed = "stale".into();
                    });
                }
            }
            tokio::select! {_=tokio::time::sleep(Duration::from_secs(1))=>{}, changed=project_rx.changed()=>{if changed.is_err(){break;}}}
        }
    });
    let result = tokio::task::spawn_blocking(move || interactive(rx, project_tx))
        .await
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    poll.abort();
    result
}

fn interactive(rx: watch::Receiver<Snapshot>, project_tx: watch::Sender<usize>) -> Result<()> {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            ratatui::restore();
        }
    }
    let mut terminal = ratatui::init();
    let _restore = Restore;
    let (mut tab, mut selected, mut scroll) = (0, 0usize, 0u16);
    loop {
        let data = rx.borrow().clone();
        terminal.draw(|frame| draw(frame, &data, tab, selected, scroll))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Tab | KeyCode::Right => {
                    tab = (tab + 1) % TABS.len();
                    selected = 0;
                    scroll = 0;
                }
                KeyCode::BackTab | KeyCode::Left => {
                    tab = (tab + TABS.len() - 1) % TABS.len();
                    selected = 0;
                    scroll = 0;
                }
                KeyCode::Char(c @ '1'..='7') => {
                    tab = (c as usize) - ('1' as usize);
                    selected = 0;
                    scroll = 0;
                }
                KeyCode::Down => {
                    let count = match tab {
                        1 => data.tasks.len(),
                        2 => data.workers.len(),
                        3 => data.personas.len(),
                        4 => data.handoffs.len(),
                        5 => data.accounts.len(),
                        6 => data.activity.len(),
                        _ => 0,
                    };
                    selected = (selected + 1).min(count.saturating_sub(1));
                    scroll = 0;
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    scroll = 0;
                }
                KeyCode::Char('j') => scroll = scroll.saturating_add(1),
                KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                KeyCode::Char('p') => {
                    let _ = project_tx.send((data.project + 1) % data.projects.len().max(1));
                    selected = 0;
                    scroll = 0;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn dashboard_renders_real_worker_state_and_review_boundary() {
        let data = Snapshot {
            projects: vec![json!({"name":"Example","repository":"/example"})],
            workers: vec![
                json!({"adapter":"codex","status":"released","worker_label":"codex","task_state":"awaiting_input","workspace":{"path":"/example/worktree"},"result":{"provider":{"tool_calls":1}}}),
            ],
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
        terminal.draw(|frame| draw(frame, &data, 2, 0, 0)).unwrap();
        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("awaiting_input"));
        assert!(rendered.contains("Successful exit still requires review."));
        assert!(rendered.contains("/example/worktree"));
        // Every view must also render at a narrow terminal size.
        let mut small = Terminal::new(TestBackend::new(30, 10)).unwrap();
        for tab in 0..TABS.len() {
            small.draw(|frame| draw(frame, &data, tab, 0, 0)).unwrap();
        }
    }
    #[test]
    fn operational_text_cannot_inject_terminal_control_sequences() {
        assert_eq!(
            field(&json!({"name":"hello\u{1b}[31m\nworld"}), "name"),
            "hello[31mworld"
        );
    }
}
