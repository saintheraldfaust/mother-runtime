//! Live dashboard with owner-invoked forms and supervised execution controls.
mod actions;
mod forms;
use crate::{
    Error, Result,
    protocol::{Operation, PROTOCOL_VERSION, Request},
    runtime,
};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use forms::{Form, Kind};
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
use tokio::sync::{mpsc, watch};

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
    task_detail: Value,
    project: usize,
    error: Option<String>,
    refreshed: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Selection {
    project: usize,
    project_id: Option<String>,
    task_id: Option<String>,
}

async fn call(dir: &Path, operation: Operation) -> Result<Value> {
    call_request(
        dir,
        &Request {
            version: PROTOCOL_VERSION,
            request_id: uuid::Uuid::new_v4().to_string(),
            operation,
        },
    )
    .await
}

async fn call_request(dir: &Path, request: &Request) -> Result<Value> {
    let response = runtime::call(dir, request).await?;
    if let Some(error) = response.error {
        return Err(match error.code.as_str() {
            "invalid_request" => Error::Invalid(error.message),
            "conflict" => Error::Conflict(error.message),
            "not_found" => Error::NotFound(error.message),
            "integrity_error" => Error::Integrity(error.message),
            _ => Error::Unavailable(error.message),
        });
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

async fn snapshot(dir: &Path, selection: &Selection) -> Result<Snapshot> {
    let health = call(dir, Operation::Health).await?;
    let projects = pages(dir, |after| Operation::ProjectsList { after, limit: 100 }).await?;
    let project = selection
        .project_id
        .as_ref()
        .and_then(|id| {
            projects
                .iter()
                .position(|p| p["id"].as_str() == Some(id.as_str()))
        })
        .unwrap_or(selection.project)
        .min(projects.len().saturating_sub(1));
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
        if let Some(task_id) = selection
            .task_id
            .as_deref()
            .or_else(|| data.tasks.first().and_then(|t| t["id"].as_str()))
            && data.tasks.iter().any(|t| t["id"] == task_id)
        {
            data.task_detail = call(
                dir,
                Operation::TaskGet {
                    task_id: task_id.into(),
                },
            )
            .await?;
        }
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

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(8192)
        .collect()
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
    if let Some(artifacts) = worker["result"]["discovered_artifacts"].as_array() {
        for artifact in artifacts.iter().take(16) {
            lines.push(heading(
                "Discovered file",
                format!(
                    "{} · {}",
                    field(artifact, "path"),
                    field(artifact, "status")
                ),
            ));
            lines.push(heading("SHA-256", field(artifact, "sha256")));
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
            Line::raw("Describe a task objective and acceptance criteria, then mark it ready."),
            Line::raw("Start Codex from Tasks. Inspect retained output in Workers."),
            Line::raw(""),
            Line::raw(
                "a Register project · n Create task · t Mark ready · r Start Codex · s Stop owned worker",
            ),
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
        let mut item = items
            .get(selected.min(items.len().saturating_sub(1)))
            .cloned()
            .unwrap_or(Value::Null);
        if tab == 1 && data.task_detail["id"] == item["id"] && !item.is_null() {
            item = data.task_detail.clone();
        }
        let mut lines = if item.is_null() {
            vec![
                Line::raw("No records yet."),
                Line::raw("Create work through Codex MCP or motherctl."),
            ]
        } else if tab == 2 {
            worker_detail(&item)
        } else {
            let keys: &[&str] = match tab {
                1 => &[
                    "title",
                    "id",
                    "objective",
                    "state",
                    "task_revision",
                    "updated_at",
                ],
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
        if tab == 1 {
            if let Some(criteria) = item["outcome_criteria"].as_array() {
                for criterion in criteria {
                    if let Some(text) = criterion.as_str() {
                        lines.push(heading("Acceptance", clean(text)));
                    }
                }
                if !criteria.is_empty() {
                    lines.push(Line::styled(
                        "Behavioral criteria: verification pending",
                        Style::default().fg(Color::Yellow),
                    ));
                }
            }
            if let Some(constraints) = item["artifact_constraints"].as_array() {
                for constraint in constraints {
                    lines.push(heading("File constraint", field(constraint, "path")));
                }
            }
        }
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
        format!(" {} · q quit", clean(error))
    } else {
        format!(
            " a project · n task · t ready · r run · s stop | Tab views · p switch project · q quit | {}",
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
    let first = snapshot(&dir, &Selection::default()).await?;
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
    let (project_tx, mut project_rx) = watch::channel(Selection::default());
    let (command_tx, command_rx) = mpsc::channel(8);
    let (feedback_tx, feedback_rx) = mpsc::unbounded_channel();
    let action_task = tokio::spawn(actions::serve(dir.clone(), command_rx, feedback_tx));
    let poll = tokio::spawn(async move {
        loop {
            let selection = project_rx.borrow_and_update().clone();
            match snapshot(&dir, &selection).await {
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
    let result =
        tokio::task::spawn_blocking(move || interactive(rx, project_tx, command_tx, feedback_rx))
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))?;
    poll.abort();
    let _ = action_task.await;
    result
}

fn selected_task(data: &Snapshot, tab: usize, selected: usize) -> Option<Value> {
    if tab == 1 {
        data.tasks.get(selected).cloned()
    } else {
        None
    }
}

fn interactive(
    rx: watch::Receiver<Snapshot>,
    project_tx: watch::Sender<Selection>,
    command_tx: mpsc::Sender<actions::CommandRequest>,
    mut feedback: mpsc::UnboundedReceiver<actions::Feedback>,
) -> Result<()> {
    use crossterm::{
        event::{DisableBracketedPaste, EnableBracketedPaste},
        execute,
    };
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = execute!(io::stdout(), DisableBracketedPaste);
            ratatui::restore();
        }
    }
    let mut terminal = ratatui::init();
    let _restore = Restore;
    execute!(io::stdout(), EnableBracketedPaste)?;
    let (mut tab, mut selected, mut scroll) = (0, 0usize, 0u16);
    let mut form: Option<Form> = None;
    let mut notice = String::new();
    let mut focus_task: Option<String> = None;
    let mut requested_project: Option<String> = None;
    loop {
        while let Ok(update) = feedback.try_recv() {
            let matches = form
                .as_ref()
                .is_some_and(|f| update.id.as_deref() == Some(f.id.as_str()));
            match update.result {
                Ok(effect) => {
                    notice = effect.message;
                    if let Some(id) = effect.project_id {
                        requested_project = Some(id);
                        selected = 0;
                        tab = 0;
                    }
                    if update.id.is_some()
                        && let Some(id) = effect.task_id
                    {
                        focus_task = Some(id);
                        tab = 1;
                        scroll = 0;
                    }
                    if matches {
                        form = None;
                    }
                }
                Err(error) => {
                    notice = error.to_string();
                    if matches && let Some(form) = &mut form {
                        form.pending = false;
                        form.error = Some(error.to_string());
                    }
                }
            }
        }
        let data = rx.borrow().clone();
        if let Some(id) = requested_project.take() {
            let _ = project_tx.send(Selection {
                project: 0,
                project_id: Some(id),
                task_id: None,
            });
        }
        if tab == 1
            && let Some(id) = &focus_task
            && let Some(index) = data
                .tasks
                .iter()
                .position(|t| t["id"].as_str() == Some(id.as_str()))
        {
            selected = index;
            focus_task = None;
        }
        terminal.draw(|frame| {
            draw(frame, &data, tab, selected, scroll);
            if !notice.is_empty() {
                let area = frame.area();
                frame.render_widget(
                    Paragraph::new(clean(&notice)).style(Style::default().fg(Color::Yellow)),
                    ratatui::layout::Rect::new(
                        area.x,
                        area.bottom().saturating_sub(1),
                        area.width,
                        1,
                    ),
                );
            }
            if let Some(form) = &form {
                form.render(frame);
            }
        })?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let event = event::read()?;
        if let Some(dialog) = &mut form {
            match event {
                Event::Paste(text) => dialog.paste(&text),
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if (key.code == KeyCode::Esc
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)))
                        && !dialog.pending
                    {
                        form = None;
                        continue;
                    }
                    if dialog.submit_key(key) {
                        if matches!(dialog.kind, Kind::Quit) {
                            break;
                        }
                        match dialog.command() {
                            Ok(command) => match command_tx.try_send(command) {
                                Ok(()) => {
                                    dialog.pending = true;
                                    dialog.error = None;
                                }
                                Err(_) => {
                                    dialog.error =
                                        Some("action queue unavailable; try again".into())
                                }
                            },
                            Err(error) => dialog.error = Some(error.to_string()),
                        }
                    } else {
                        dialog.key(key);
                    }
                }
                _ => {}
            }
            continue;
        }
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let opening_action = matches!(key.code, KeyCode::Char('a' | 'n' | 't' | 'r'));
        if opening_action && data.error.is_some() {
            notice = "Runtime is unavailable; wait for reconnection before changing state.".into();
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => form = Some(Form::quit()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                form = Some(Form::quit())
            }
            KeyCode::Char('a') => form = Some(Form::project()),
            KeyCode::Char('n') => {
                if let Some(project) = data.projects.get(data.project) {
                    if let Some(id) = project["id"].as_str() {
                        form = Some(Form::task(id.into(), field(project, "name")));
                    }
                } else {
                    notice = "Register a project with a before creating a task.".into();
                }
            }
            KeyCode::Char('t' | 'r') => {
                if let Some(task) = selected_task(&data, tab, selected) {
                    let id = task["id"].as_str().unwrap_or_default().to_owned();
                    let revision = task["task_revision"].as_u64().unwrap_or_default();
                    if key.code == KeyCode::Char('t') {
                        match serde_json::from_value::<crate::tasks::TaskState>(
                            task["state"].clone(),
                        ) {
                            Ok(state) => {
                                match Form::ready(id, revision, state, field(&task, "title")) {
                                    Ok(dialog) => form = Some(dialog),
                                    Err(error) => notice = error.to_string(),
                                }
                            }
                            Err(_) => notice = "Cannot read the selected task state.".into(),
                        }
                    } else if task["state"] == "ready" {
                        form = Some(Form::run(id, revision, field(&task, "title")));
                    } else {
                        notice =
                            "Select a ready task, or press t to prepare an eligible task first."
                                .into();
                    }
                } else {
                    notice = "Select a task in Tasks first.".into();
                }
            }
            KeyCode::Char('s') => {
                let worker = if tab == 2 {
                    data.workers.get(selected)
                } else {
                    selected_task(&data, tab, selected).and_then(|task| {
                        data.workers
                            .iter()
                            .find(|w| w["task_id"] == task["id"] && w["active"] == true)
                    })
                };
                if let Some(worker) = worker.filter(|w| w["process"]["status"] == "running") {
                    form = Some(Form::stop(field(worker, "worker_label")));
                } else {
                    notice = "Select a running worker to request a stop.".into();
                }
            }
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
                let index = (data.project + 1) % data.projects.len().max(1);
                let _ = project_tx.send(Selection {
                    project: index,
                    project_id: data
                        .projects
                        .get(index)
                        .and_then(|p| p["id"].as_str())
                        .map(str::to_owned),
                    task_id: None,
                });
                selected = 0;
                scroll = 0;
                focus_task = None;
            }
            _ => {}
        }
        if tab == 1 {
            let task_id = data
                .tasks
                .get(selected)
                .and_then(|t| t["id"].as_str())
                .map(str::to_owned);
            project_tx.send_if_modified(|selection| {
                if selection.task_id != task_id {
                    selection.task_id = task_id;
                    true
                } else {
                    false
                }
            });
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
