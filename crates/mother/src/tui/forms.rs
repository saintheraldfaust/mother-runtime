//! Terminal forms hold a stable operation ID and the revisions the operator reviewed.
use super::{
    actions::{Action, CommandRequest, ready_states},
    clean, heading,
};
use crate::{
    Error, Result,
    protocol::{Operation, PROTOCOL_VERSION, Request},
    tasks::{AcceptanceCriterion, ArtifactCriterion, TaskInput, TaskState},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

pub(super) enum Kind {
    Project,
    Task {
        project_id: String,
    },
    Ready {
        task_id: String,
        revision: u64,
        state: TaskState,
    },
    Run {
        task_id: String,
        revision: u64,
    },
    Stop {
        label: String,
    },
    Quit,
}

struct Input {
    label: &'static str,
    value: String,
    cursor: usize,
    multiline: bool,
    limit: usize,
}

impl Input {
    fn new(label: &'static str, value: String, multiline: bool, limit: usize) -> Self {
        Self {
            label,
            cursor: value.len(),
            value,
            multiline,
            limit,
        }
    }
    fn insert(&mut self, text: &str) -> bool {
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || (self.multiline && *c == '\n'))
            .collect();
        if text.is_empty() || self.value.len() + text.len() > self.limit {
            return false;
        }
        self.value.insert_str(self.cursor, &text);
        self.cursor += text.len();
        true
    }
    fn left(&mut self) {
        self.cursor = self.value[..self.cursor]
            .char_indices()
            .last()
            .map_or(0, |(index, _)| index);
    }
    fn right(&mut self) {
        if let Some(c) = self.value[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }
    fn key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Enter if self.multiline => self.insert("\n"),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let changed = !self.value.is_empty();
                self.value.clear();
                self.cursor = 0;
                changed
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let end = self.cursor;
                self.left();
                self.value.replace_range(self.cursor..end, "");
                true
            }
            KeyCode::Delete if self.cursor < self.value.len() => {
                let start = self.cursor;
                self.right();
                let end = self.cursor;
                self.cursor = start;
                self.value.replace_range(start..end, "");
                true
            }
            KeyCode::Left => {
                self.left();
                false
            }
            KeyCode::Right => {
                self.right();
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.value.len();
                false
            }
            _ => false,
        }
    }
}

pub(super) struct Form {
    pub kind: Kind,
    pub id: String,
    pub pending: bool,
    pub error: Option<String>,
    title: &'static str,
    context: String,
    help: &'static str,
    fields: Vec<Input>,
    focused: usize,
    advanced: bool,
}

impl Form {
    fn new(
        kind: Kind,
        title: &'static str,
        context: String,
        help: &'static str,
        fields: Vec<Input>,
    ) -> Self {
        Self {
            kind,
            id: uuid::Uuid::new_v4().to_string(),
            pending: false,
            error: None,
            title,
            context,
            help,
            fields,
            focused: 0,
            advanced: false,
        }
    }
    pub fn project() -> Self {
        let path = std::env::current_dir().unwrap_or_default();
        Self::new(
            Kind::Project,
            "Register project",
            "Register an existing directory; files and Git history are preserved.".into(),
            "Use an absolute directory path. Registration does not configure Codex MCP or create a Git commit.",
            vec![
                Input::new(
                    "Name",
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    false,
                    256,
                ),
                Input::new(
                    "Directory",
                    path.to_string_lossy().into_owned(),
                    false,
                    4096,
                ),
            ],
        )
    }
    pub fn task(project_id: String, name: String) -> Self {
        Self::new(
            Kind::Task { project_id },
            "Create task",
            format!("Project: {name}. The task starts in created state."),
            "Describe the desired result. The agent chooses implementation files. Ctrl-O opens optional file constraints and dependencies; neither is required.",
            vec![
                Input::new("Title", String::new(), false, 256),
                Input::new("Objective", String::new(), true, 4096),
                Input::new("Acceptance criteria (optional)", String::new(), true, 8192),
                Input::new(
                    "File constraints (optional, advanced)",
                    String::new(),
                    true,
                    8192,
                ),
                Input::new("Dependency task IDs (optional)", String::new(), true, 4096),
            ],
        )
    }
    pub fn ready(task_id: String, revision: u64, state: TaskState, title: String) -> Result<Self> {
        let steps = ready_states(state)?
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" → ");
        Ok(Self::new(
            Kind::Ready {
                task_id,
                revision,
                state,
            },
            "Mark task ready",
            format!("Task: {title}\n{} → {steps}", state.as_str()),
            "The service checks dependencies and task revisions. This action does not launch a worker or approve the result.",
            vec![],
        ))
    }
    pub fn run(task_id: String, revision: u64, title: String) -> Self {
        Self::new(
            Kind::Run { task_id, revision },
            "Start Codex worker",
            format!("Task: {title}\nThis uses your existing Codex login and subscription."),
            "Codex edits a new worktree from committed HEAD, using workspace-write sandboxing. Put the full implementation brief in the task objective. Success requires later inspection; it does not complete the task.",
            vec![
                Input::new("Timeout in seconds (1–3600)", "300".into(), false, 4),
                Input::new("Step / command limit (1–200)", "20".into(), false, 3),
                Input::new(
                    "Model (optional; leave empty for Codex default)",
                    String::new(),
                    false,
                    256,
                ),
            ],
        )
    }
    pub fn stop(label: String) -> Self {
        Self::new(
            Kind::Stop {
                label: label.clone(),
            },
            "Stop worker",
            format!("Worker: {label}\nStop this execution from the current dashboard session."),
            "The supervisor terminates its worker process group and records a killed process. Its task becomes failed and can be triaged for retry. Files remain in the retained worktree.",
            vec![],
        )
    }
    pub fn quit() -> Self {
        Self::new(
            Kind::Quit,
            "Close dashboard",
            "Worker supervisors launched here continue after you quit.".into(),
            "To stop a worker first, press Esc, select it in Workers and press s. Enter detaches this dashboard and leaves the runtime and supervisors running.",
            vec![],
        )
    }
    fn visible_fields(&self) -> usize {
        if matches!(self.kind, Kind::Task { .. }) && !self.advanced {
            3
        } else {
            self.fields.len()
        }
    }
    pub fn key(&mut self, key: KeyEvent) {
        if self.pending {
            return;
        }
        if matches!(self.kind, Kind::Task { .. })
            && key.code == KeyCode::Char('o')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.advanced = !self.advanced;
            self.focused = self.focused.min(self.visible_fields().saturating_sub(1));
            return;
        }
        let count = self.visible_fields();
        if count == 0 {
            return;
        }
        match key.code {
            KeyCode::Tab => self.focused = (self.focused + 1) % count,
            KeyCode::BackTab => self.focused = (self.focused + count - 1) % count,
            KeyCode::Enter if !self.fields[self.focused].multiline => {
                self.focused = (self.focused + 1) % count
            }
            _ => {
                if self.fields[self.focused].key(key) {
                    self.edited();
                }
            }
        }
    }
    fn edited(&mut self) {
        self.id = uuid::Uuid::new_v4().to_string();
        self.error = None;
    }
    pub fn paste(&mut self, text: &str) {
        if !self.pending && !self.fields.is_empty() && self.fields[self.focused].insert(text) {
            self.edited();
        }
    }
    pub fn submit_key(&self, key: KeyEvent) -> bool {
        !self.pending
            && ((key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL))
                || (self.fields.is_empty() && key.code == KeyCode::Enter))
    }
    pub fn command(&self) -> Result<CommandRequest> {
        let value = |index: usize| self.fields[index].value.trim().to_owned();
        let operation = match &self.kind {
            Kind::Project => {
                let repository = value(1);
                if value(0).is_empty() || !std::path::Path::new(&repository).is_absolute() {
                    return Err(Error::Invalid(
                        "enter a project name and an absolute directory path".into(),
                    ));
                }
                Some(Operation::ProjectAttach {
                    name: value(0),
                    repository,
                })
            }
            Kind::Task { project_id } => {
                if value(0).is_empty() || value(1).is_empty() {
                    return Err(Error::Invalid(
                        "title and objective are required; filenames are chosen by the agent"
                            .into(),
                    ));
                }
                let outcomes: Vec<_> = value(2)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect();
                if outcomes.len() > 16 || outcomes.iter().any(|criterion| criterion.len() > 1024) {
                    return Err(Error::Invalid(
                        "provide at most 16 acceptance criteria, each up to 1024 bytes".into(),
                    ));
                }
                let files: Vec<_> = value(3)
                    .lines()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned)
                    .collect();
                if files.len() > 16 || files.iter().any(|path| !crate::evidence::valid_path(path)) {
                    return Err(Error::Invalid(
                        "optional file constraints must be normalized relative paths, at most 16"
                            .into(),
                    ));
                }
                let dependencies: Vec<_> = value(4)
                    .lines()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect();
                if dependencies
                    .iter()
                    .any(|id| uuid::Uuid::parse_str(id).is_err())
                {
                    return Err(Error::Invalid(
                        "dependencies must be task IDs, one per line".into(),
                    ));
                }
                Some(Operation::TaskCreate(TaskInput {
                    project_id: project_id.clone(),
                    title: value(0),
                    objective: value(1),
                    source: "tui".into(),
                    acceptance_criteria: outcomes
                        .into_iter()
                        .map(AcceptanceCriterion::Outcome)
                        .collect(),
                    artifact_constraints: files
                        .into_iter()
                        .map(|path| ArtifactCriterion {
                            path,
                            expected_sha256: None,
                        })
                        .collect(),
                    dependencies,
                    review_required: false,
                    persona: None,
                    expected_project_revision: None,
                }))
            }
            _ => None,
        };
        let action = if let Some(operation) = operation {
            Action::Submit(Box::new(Request {
                version: PROTOCOL_VERSION,
                request_id: self.id.clone(),
                operation,
            }))
        } else {
            match &self.kind {
                Kind::Ready {
                    task_id,
                    revision,
                    state,
                } => Action::Ready {
                    task_id: task_id.clone(),
                    revision: *revision,
                    state: *state,
                },
                Kind::Run { task_id, revision } => {
                    let timeout = value(0)
                        .parse::<u64>()
                        .ok()
                        .filter(|v| (1..=3600).contains(v))
                        .ok_or_else(|| Error::Invalid("timeout must be 1–3600 seconds".into()))?;
                    let steps = value(1)
                        .parse::<u32>()
                        .ok()
                        .filter(|v| (1..=200).contains(v))
                        .ok_or_else(|| Error::Invalid("step limit must be 1–200".into()))?;
                    Action::Run {
                        task_id: task_id.clone(),
                        revision: *revision,
                        timeout,
                        steps,
                        model: value(2),
                    }
                }
                Kind::Stop { label } => Action::Stop {
                    worker_label: label.clone(),
                },
                _ => {
                    return Err(Error::Invalid(
                        "this dialog does not submit a runtime action".into(),
                    ));
                }
            }
        };
        Ok(CommandRequest {
            id: self.id.clone(),
            action,
        })
    }
    pub fn render(&self, frame: &mut Frame) {
        let screen = frame.area();
        let area = Rect::new(
            screen.x + screen.width / 20,
            screen.y + screen.height / 20,
            screen.width.saturating_sub(screen.width / 10),
            screen.height.saturating_sub(screen.height / 10),
        );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", self.title))
                .border_style(Style::default().fg(Color::Cyan)),
            area,
        );
        let inner = area.inner(ratatui::layout::Margin::new(2, 1));
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(self.visible_fields() as u16),
                Constraint::Min(3),
                Constraint::Length(4),
                Constraint::Length(3),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(clean(&self.context)).wrap(Wrap { trim: false }),
            sections[0],
        );
        let labels = self
            .fields
            .iter()
            .take(self.visible_fields())
            .enumerate()
            .map(|(index, input)| {
                ListItem::new(format!(
                    "{} {}  {}",
                    if index == self.focused { "›" } else { " " },
                    input.label,
                    clean(&input.value).chars().take(36).collect::<String>()
                ))
                .style(if index == self.focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                })
            })
            .collect::<Vec<_>>();
        frame.render_widget(List::new(labels), sections[1]);
        if let Some(input) = self.fields.get(self.focused) {
            let mut shown = input.value.clone();
            shown.insert(input.cursor, '▌');
            let line = input.value[..input.cursor]
                .chars()
                .filter(|c| *c == '\n')
                .count() as u16;
            frame.render_widget(
                Paragraph::new(shown)
                    .wrap(Wrap { trim: false })
                    .scroll((line.saturating_sub(sections[2].height.saturating_sub(3)), 0))
                    .block(Block::default().borders(Borders::ALL).title(input.label)),
                sections[2],
            );
        } else {
            frame.render_widget(
                Paragraph::new(self.help).wrap(Wrap { trim: false }),
                sections[2],
            );
        }
        if !self.fields.is_empty() {
            frame.render_widget(
                Paragraph::new(self.help)
                    .style(Style::default().fg(Color::DarkGray))
                    .wrap(Wrap { trim: false }),
                sections[3],
            );
        }
        let notice = if self.pending {
            "Submitting… waiting for the canonical result.".into()
        } else if let Some(error) = &self.error {
            format!(
                "{} · Esc closes this form; edits get a new retry key.",
                clean(error)
            )
        } else if self.fields.is_empty() {
            "Enter confirms · Esc returns".into()
        } else {
            "Tab fields · Ctrl-O advanced · Enter newline / next · Ctrl-S submit · Esc cancel"
                .into()
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    notice,
                    Style::default()
                        .fg(if self.error.is_some() {
                            Color::Yellow
                        } else {
                            Color::Cyan
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                heading("", ""),
            ])
            .wrap(Wrap { trim: false }),
            sections[4],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn multiline_task_form_retains_project_and_retry_key_until_edited() {
        let mut form = Form::task("project-a".into(), "Example".into());
        form.paste("Landing page");
        form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        form.paste("Build a responsive page.\nInclude navigation and a footer.");
        form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        form.paste("Works on mobile\nIncludes a footer");
        let id = form.id.clone();
        let first = form.command().unwrap();
        let retry = form.command().unwrap();
        assert_eq!(first.id, retry.id);
        let Action::Submit(request) = first.action else {
            panic!("expected task request")
        };
        let Operation::TaskCreate(input) = request.operation else {
            panic!("expected task creation")
        };
        assert_eq!(input.project_id, "project-a");
        assert!(input.objective.contains("\nInclude navigation"));
        assert_eq!(input.acceptance_criteria.len(), 2);
        assert!(input.artifact_constraints.is_empty());
        assert!(!input.review_required);
        form.pending = true;
        form.paste("ignored while pending");
        assert_eq!(form.id, id);
        form.pending = false;
        form.paste("\nSupports keyboard navigation");
        assert_ne!(form.id, id);
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(|frame| form.render(frame)).unwrap();
        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Create task"));
        assert!(rendered.contains("Acceptance criteria"));
        assert!(!rendered.contains("File constraints"));
    }

    #[test]
    fn unicode_edits_remain_on_character_boundaries_and_exclude_control_sequences() {
        let mut input = Input::new("Objective", String::new(), true, 64);
        input.insert("hé🙂\nnext");
        input.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        input.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        input.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        input.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.value, "h🙂\nnext");
        input.insert("\u{1b}[31m\u{7}");
        assert!(!input.value.contains('\u{1b}'));
        assert!(!input.value.contains('\u{7}'));
        assert!(input.value.is_char_boundary(input.cursor));
    }
    #[test]
    fn default_task_flow_needs_only_title_and_objective_and_hides_file_constraints() {
        let mut form = Form::task("project-a".into(), "Example".into());
        form.paste("Build a landing page");
        form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        form.paste("Create a responsive product landing page");
        assert_eq!(form.visible_fields(), 3);
        let Action::Submit(request) = form.command().unwrap().action else {
            panic!("expected submission")
        };
        let Operation::TaskCreate(input) = request.operation else {
            panic!("expected task")
        };
        assert!(input.acceptance_criteria.is_empty());
        assert!(input.artifact_constraints.is_empty());
        form.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(form.visible_fields(), 5);
        form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        form.paste("Works on mobile");
        form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        form.paste("README.md");
        let Action::Submit(request) = form.command().unwrap().action else {
            panic!("expected submission")
        };
        let Operation::TaskCreate(input) = request.operation else {
            panic!("expected task")
        };
        assert!(
            matches!(&input.acceptance_criteria[0],AcceptanceCriterion::Outcome(text) if text=="Works on mobile")
        );
        assert_eq!(input.artifact_constraints[0].path, "README.md");
    }
}
