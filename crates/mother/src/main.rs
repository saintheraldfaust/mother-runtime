use clap::{Parser, Subcommand};
use mother_runtime::{
    Error, Result, config,
    continuity::{DecisionInput, HandoffInput},
    github::{
        self, GithubIdentityAttach, GithubIdentityRevoke, GithubIdentityStatus, GithubOperation,
    },
    mcp,
    personas::{PersonaClaim, PersonaInput, PersonaStatus, PersonaStatusChange, PersonaUpdate},
    protocol::{ContextKind, Operation, PROTOCOL_VERSION, Request},
    runtime,
    tasks::{CheckpointInput, TaskInput, TaskState, TaskTransition},
    workers::{
        WorkerAbandonInput, WorkerExecutionStatus, WorkerHeartbeatInput, WorkerLeaseInput,
        WorkerMockInput, WorkerProcessAttachInput, WorkerProcessFinishInput, WorkerProcessOutcome,
        WorkerReleaseInput, WorkerReleaseOutcome, WorkerWorkspaceCleanupInput,
        WorkerWorkspaceInput,
    },
};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    name = "motherctl",
    version,
    about = "Persistent local engineering context"
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Emit compact JSON. The default is indented JSON.
    #[arg(long, global = true)]
    json: bool,
    /// Stable retry key for mutations. Reuse only with the identical command.
    #[arg(long, global = true)]
    request_id: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize private configuration and database; safe to repeat offline.
    Init,
    /// Run the canonical local service in the foreground.
    Serve,
    /// Serve MCP over stdio, forwarding requests to the existing runtime.
    Mcp,
    /// Check configuration, database integrity and service availability.
    Doctor,
    /// Query the running service.
    Status,
    #[command(subcommand)]
    Projects(Projects),
    #[command(subcommand)]
    Context(Context),
    #[command(subcommand)]
    Decisions(Decisions),
    #[command(subcommand)]
    Handoff(Handoff),
    #[command(subcommand)]
    Tasks(Tasks),
    #[command(subcommand)]
    Personas(Personas),
    #[command(subcommand)]
    Github(Github),
    #[command(subcommand)]
    Workers(Workers),
    /// Internal deterministic helper launched by `workers run`; not a public adapter.
    #[command(name = "__mock-worker", hide = true)]
    MockWorker(MockWorkerArgs),
    History {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum Projects {
    Inspect {
        #[arg(long)]
        project: String,
    },
    Attach {
        #[arg(long)]
        name: String,
        #[arg(long)]
        repository: PathBuf,
    },
    List {
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum Context {
    Record {
        #[arg(long)]
        project: String,
        #[arg(long, value_enum)]
        kind: ContextKind,
        #[arg(long)]
        content: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Get {
        #[arg(long)]
        project: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum Decisions {
    Record {
        #[arg(long)]
        project: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        decision: String,
        #[arg(long)]
        rationale: String,
        #[arg(long = "alternative")]
        alternatives: Vec<String>,
        #[arg(long)]
        source: String,
        #[arg(long)]
        supersedes: Option<String>,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    List {
        #[arg(long)]
        project: String,
        #[arg(long)]
        include_superseded: bool,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Inspect {
        #[arg(long)]
        decision: String,
    },
}

#[derive(Subcommand)]
enum Handoff {
    /// Read a structured, bounded handoff submission from JSON.
    Create {
        #[arg(long)]
        file: PathBuf,
    },
    Show {
        #[arg(long)]
        handoff: String,
    },
    List {
        #[arg(long)]
        project: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Claim one unchanged handoff for continuation; does not start a worker.
    Resume {
        #[arg(long)]
        handoff: String,
        #[arg(long)]
        recipient: String,
        #[arg(long)]
        expected_revision: u64,
    },
}

#[derive(Subcommand)]
enum Tasks {
    Create {
        #[arg(long)]
        file: PathBuf,
    },
    Inspect {
        #[arg(long)]
        task: String,
    },
    List {
        #[arg(long)]
        project: String,
        #[arg(long, value_enum)]
        state: Option<TaskState>,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Change host-tracked state; does not launch or cancel a worker process.
    Transition {
        #[arg(long)]
        task: String,
        #[arg(long, value_enum)]
        state: TaskState,
        #[arg(long)]
        source: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        owner_label: Option<String>,
        #[arg(long)]
        expected_task_revision: u64,
        #[arg(long)]
        persona: Option<String>,
        #[arg(long)]
        expected_persona_revision: Option<u64>,
        #[arg(long)]
        expected_project_revision: Option<u64>,
    },
    /// Record progress and observe the task contract's repository files.
    Checkpoint {
        #[arg(long)]
        file: PathBuf,
    },
    History {
        #[arg(long)]
        task: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    Checkpoints {
        #[arg(long)]
        task: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum Personas {
    /// Create a persona from a bounded JSON definition.
    Create {
        #[arg(long)]
        file: PathBuf,
    },
    List {
        #[arg(long, value_enum)]
        status: Option<PersonaStatus>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Inspect {
        #[arg(long)]
        persona: String,
    },
    /// Replace a persona definition using its current revision.
    Edit {
        #[arg(long)]
        persona: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        file: PathBuf,
    },
    Disable {
        #[arg(long)]
        persona: String,
        #[arg(long)]
        expected_revision: u64,
    },
    Enable {
        #[arg(long)]
        persona: String,
        #[arg(long)]
        expected_revision: u64,
    },
    Archive {
        #[arg(long)]
        persona: String,
        #[arg(long)]
        expected_revision: u64,
    },
    History {
        #[arg(long)]
        persona: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum Github {
    /// Create an isolated, empty GitHub CLI profile directory.
    Profile {
        #[arg(long, default_value = "github.com")]
        host: String,
    },
    /// Verify a completed gh login read-only and record its authorized identity.
    Connect {
        #[arg(long)]
        profile: String,
        #[arg(long, default_value = "github.com")]
        host: String,
        #[arg(long)]
        persona: String,
        #[arg(long)]
        expected_persona_revision: u64,
    },
    List {
        #[arg(long, value_enum)]
        status: Option<GithubIdentityStatus>,
        #[arg(long)]
        persona: Option<String>,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Inspect {
        #[arg(long)]
        identity: String,
    },
    /// Stop future operations for an identity without deleting history.
    Revoke {
        #[arg(long)]
        identity: String,
        #[arg(long)]
        expected_revision: u64,
    },
    History {
        #[arg(long)]
        identity: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum Workers {
    /// List ready tasks whose dependencies are completed and that have no unexpired lease.
    Eligible {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    List {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, value_enum)]
        status: Option<WorkerExecutionStatus>,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Inspect {
        #[arg(long)]
        execution: String,
    },
    History {
        #[arg(long)]
        execution: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Acquire a coordination lease; this does not start or supervise a process.
    Lease {
        #[arg(long)]
        task: String,
        #[arg(long)]
        worker_label: String,
        #[arg(long, default_value_t = 120)]
        lease_seconds: u32,
        #[arg(long, default_value_t = 4)]
        max_active: u32,
        #[arg(long)]
        expected_task_revision: Option<u64>,
        #[arg(long)]
        expected_project_revision: Option<u64>,
    },
    Heartbeat {
        #[arg(long)]
        execution: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, default_value_t = 120)]
        lease_seconds: u32,
    },
    /// Record a deterministic mock checkpoint without editing files or completing the task.
    Mock {
        #[arg(long)]
        execution: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, default_value_t = 120)]
        lease_seconds: u32,
        #[arg(long)]
        summary: String,
        #[arg(long = "work-remaining")]
        work_remaining: Vec<String>,
        #[arg(long = "blocker")]
        blockers: Vec<String>,
    },
    Release {
        #[arg(long)]
        execution: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, value_enum)]
        outcome: WorkerReleaseOutcome,
        #[arg(long)]
        summary: Option<String>,
    },
    /// Acquire a lease, create an isolated worktree, and supervise the deterministic helper.
    Run {
        #[arg(long)]
        task: String,
        #[arg(long)]
        worker_label: String,
        #[arg(long, default_value_t = 120)]
        lease_seconds: u32,
        #[arg(long, default_value_t = 4)]
        max_active: u32,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
        #[arg(long, default_value_t = 100)]
        helper_delay_ms: u64,
        #[arg(long, default_value_t = 0)]
        helper_exit_code: i32,
    },
    /// Mark a crashed supervisor's active process as lost and block its task.
    Abandon {
        #[arg(long)]
        execution: String,
        /// Abandon even when the recorded PID still exists.
        #[arg(long)]
        force: bool,
    },
    /// Remove an inactive worker worktree and mark its workspace cleaned up.
    Cleanup {
        #[arg(long)]
        execution: String,
    },
    /// Expire overdue leases and return their assigned/running tasks to the queue.
    Recover,
}

#[derive(clap::Args)]
struct MockWorkerArgs {
    #[arg(long)]
    parent_pid: u32,
    #[arg(long)]
    execution: String,
    #[arg(long, default_value_t = 100)]
    delay_ms: u64,
    #[arg(long, default_value_t = 0)]
    exit_code: i32,
}

fn persona_claim(
    persona: Option<String>,
    expected_revision: Option<u64>,
) -> Result<Option<PersonaClaim>> {
    match (persona, expected_revision) {
        (None, None) => Ok(None),
        (Some(persona_id), Some(expected_revision)) => Ok(Some(PersonaClaim {
            persona_id,
            expected_revision,
        })),
        _ => Err(Error::Invalid(
            "persona and expected_persona_revision must be supplied together".into(),
        )),
    }
}

fn handoff_file(path: PathBuf) -> Result<HandoffInput> {
    submission_file(path, 32_768)
}

fn submission_file<T: serde::de::DeserializeOwned>(path: PathBuf, limit: u64) -> Result<T> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Invalid(format!(
            "submission JSON file exceeds {limit} bytes"
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::Invalid("JSON does not match the submission schema".into()))
}

fn output(value: &impl serde::Serialize, compact: bool) -> Result<()> {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    if compact {
        serde_json::to_writer(&mut stdout, value)?;
    } else {
        serde_json::to_writer_pretty(&mut stdout, value)?;
    }
    writeln!(stdout)?;
    Ok(())
}

async fn run(cli: Cli) -> Result<bool> {
    let dir = match cli.data_dir {
        Some(path) => path,
        None => config::default_dir()?,
    };
    match cli.command {
        Command::Init => {
            config::initialize(&dir)?;
            output(
                &serde_json::json!({"initialized":true,"data_dir":dir,"schema_version":mother_runtime::store::SCHEMA_VERSION}),
                cli.json,
            )?;
            Ok(true)
        }
        Command::Serve => {
            runtime::serve(&dir).await?;
            Ok(true)
        }
        Command::Mcp => {
            mcp::serve(dir).await?;
            Ok(true)
        }
        Command::Doctor => {
            let result = runtime::doctor(&dir).await;
            let ok = result["ok"] == true;
            output(&result, cli.json)?;
            Ok(ok)
        }
        Command::Github(Github::Profile { host }) => {
            if !github::valid_host(&host) {
                return Err(Error::Invalid(
                    "GitHub host must be a lowercase DNS hostname without a scheme or path".into(),
                ));
            }
            let profile_id = uuid::Uuid::new_v4().to_string();
            let profile_dir = config::github_profile(&dir, &profile_id)?;
            output(
                &serde_json::json!({
                    "profile_id":profile_id,
                    "host":host,
                    "config_dir":profile_dir,
                    "credentials_stored":false,
                    "next_action":"Set GH_CONFIG_DIR to config_dir, run gh auth login --hostname host yourself, then run motherctl github connect with this profile ID."
                }),
                cli.json,
            )?;
            Ok(true)
        }
        Command::Github(Github::Connect {
            profile,
            host,
            persona,
            expected_persona_revision,
        }) => {
            let profile_dir = config::existing_github_profile(&dir, &profile)?;
            let account = github::verify(&profile_dir, &host, std::path::Path::new("gh"))?;
            let operation = Operation::GithubIdentityAttach(GithubIdentityAttach {
                profile_id: profile,
                host,
                login: account.login,
                account_id: account.account_id,
                display_name: account.display_name,
                persona_id: persona,
                expected_persona_revision,
                operations: vec![GithubOperation::Read],
            });
            submit(&dir, cli.request_id, operation, cli.json).await
        }
        Command::Workers(Workers::Run {
            task,
            worker_label,
            lease_seconds,
            max_active,
            timeout_seconds,
            helper_delay_ms,
            helper_exit_code,
        }) => {
            supervised_run(
                &dir,
                task,
                worker_label,
                lease_seconds,
                max_active,
                timeout_seconds,
                helper_delay_ms,
                helper_exit_code,
                cli.json,
            )
            .await
        }
        Command::Workers(Workers::Abandon { execution, force }) => {
            abandon_execution(&dir, execution, force, cli.json).await
        }
        Command::Workers(Workers::Cleanup { execution }) => {
            cleanup_workspace(&dir, execution, cli.json).await
        }
        Command::MockWorker(args) => {
            mock_worker(args)?;
            Ok(true)
        }
        command => {
            let operation = match command {
                Command::Status => Operation::Health,
                Command::Projects(Projects::Inspect { project }) => Operation::ProjectGet {
                    project_id: project,
                },
                Command::Projects(Projects::Attach { name, repository }) => {
                    let path = std::fs::canonicalize(repository)?;
                    Operation::ProjectAttach {
                        name,
                        repository: path
                            .to_str()
                            .ok_or_else(|| Error::Invalid("repository path must be UTF-8".into()))?
                            .into(),
                    }
                }
                Command::Projects(Projects::List { after, limit }) => {
                    Operation::ProjectsList { after, limit }
                }
                Command::Context(Context::Record {
                    project,
                    kind,
                    content,
                    source,
                    expected_revision,
                }) => Operation::ContextRecord {
                    project_id: project,
                    kind,
                    content,
                    source,
                    expected_revision,
                },
                Command::Context(Context::Get {
                    project,
                    after,
                    limit,
                }) => Operation::ContextGet {
                    project_id: project,
                    after,
                    limit,
                },
                Command::History {
                    project,
                    after,
                    limit,
                } => Operation::ActivityGet {
                    project_id: project,
                    after,
                    limit,
                },
                Command::Decisions(Decisions::Record {
                    project,
                    title,
                    decision,
                    rationale,
                    alternatives,
                    source,
                    supersedes,
                    expected_revision,
                }) => Operation::DecisionRecord(DecisionInput {
                    project_id: project,
                    title,
                    decision,
                    rationale,
                    alternatives,
                    source,
                    supersedes,
                    expected_revision,
                }),
                Command::Decisions(Decisions::List {
                    project,
                    include_superseded,
                    after,
                    limit,
                }) => Operation::DecisionsList {
                    project_id: project,
                    include_superseded,
                    after,
                    limit,
                },
                Command::Decisions(Decisions::Inspect { decision }) => Operation::DecisionGet {
                    decision_id: decision,
                },
                Command::Handoff(Handoff::Create { file }) => {
                    Operation::HandoffCreate(Box::new(handoff_file(file)?))
                }
                Command::Handoff(Handoff::Show { handoff }) => Operation::HandoffGet {
                    handoff_id: handoff,
                },
                Command::Handoff(Handoff::List {
                    project,
                    after,
                    limit,
                }) => Operation::HandoffsList {
                    project_id: project,
                    after,
                    limit,
                },
                Command::Handoff(Handoff::Resume {
                    handoff,
                    recipient,
                    expected_revision,
                }) => Operation::HandoffResume {
                    handoff_id: handoff,
                    recipient,
                    expected_revision,
                },
                Command::Tasks(Tasks::Create { file }) => {
                    Operation::TaskCreate(submission_file::<TaskInput>(file, 24 * 1024)?)
                }
                Command::Tasks(Tasks::Inspect { task }) => Operation::TaskGet { task_id: task },
                Command::Tasks(Tasks::List {
                    project,
                    state,
                    after,
                    limit,
                }) => Operation::TasksList {
                    project_id: project,
                    state,
                    after,
                    limit,
                },
                Command::Tasks(Tasks::Transition {
                    task,
                    state,
                    source,
                    reason,
                    owner_label,
                    persona,
                    expected_persona_revision,
                    expected_task_revision,
                    expected_project_revision,
                }) => Operation::TaskTransition(TaskTransition {
                    task_id: task,
                    state,
                    source,
                    reason,
                    owner_label,
                    persona: persona_claim(persona, expected_persona_revision)?,
                    expected_task_revision,
                    expected_project_revision,
                }),
                Command::Tasks(Tasks::Checkpoint { file }) => {
                    Operation::TaskCheckpoint(submission_file::<CheckpointInput>(file, 24 * 1024)?)
                }
                Command::Tasks(Tasks::Checkpoints { task, after, limit }) => {
                    Operation::TaskCheckpointsList {
                        task_id: task,
                        after,
                        limit,
                    }
                }
                Command::Tasks(Tasks::History { task, after, limit }) => {
                    Operation::TaskTransitionsList {
                        task_id: task,
                        after,
                        limit,
                    }
                }
                Command::Personas(Personas::Create { file }) => {
                    Operation::PersonaCreate(submission_file::<PersonaInput>(file, 16 * 1024)?)
                }
                Command::Personas(Personas::List {
                    status,
                    project,
                    after,
                    limit,
                }) => Operation::PersonasList {
                    status,
                    project_id: project,
                    after,
                    limit,
                },
                Command::Personas(Personas::Inspect { persona }) => Operation::PersonaGet {
                    persona_id: persona,
                },
                Command::Personas(Personas::Edit {
                    persona,
                    expected_revision,
                    file,
                }) => Operation::PersonaUpdate(PersonaUpdate {
                    persona_id: persona,
                    expected_revision,
                    input: submission_file::<PersonaInput>(file, 16 * 1024)?,
                }),
                Command::Personas(Personas::Disable {
                    persona,
                    expected_revision,
                }) => Operation::PersonaSetStatus(PersonaStatusChange {
                    persona_id: persona,
                    status: PersonaStatus::Disabled,
                    expected_revision,
                }),
                Command::Personas(Personas::Enable {
                    persona,
                    expected_revision,
                }) => Operation::PersonaSetStatus(PersonaStatusChange {
                    persona_id: persona,
                    status: PersonaStatus::Active,
                    expected_revision,
                }),
                Command::Personas(Personas::Archive {
                    persona,
                    expected_revision,
                }) => Operation::PersonaSetStatus(PersonaStatusChange {
                    persona_id: persona,
                    status: PersonaStatus::Archived,
                    expected_revision,
                }),
                Command::Personas(Personas::History {
                    persona,
                    after,
                    limit,
                }) => Operation::PersonaHistory {
                    persona_id: persona,
                    after,
                    limit,
                },
                Command::Github(Github::List {
                    status,
                    persona,
                    after,
                    limit,
                }) => Operation::GithubIdentitiesList {
                    status,
                    persona_id: persona,
                    after,
                    limit,
                },
                Command::Github(Github::Inspect { identity }) => Operation::GithubIdentityGet {
                    identity_id: identity,
                },
                Command::Github(Github::Revoke {
                    identity,
                    expected_revision,
                }) => Operation::GithubIdentityRevoke(GithubIdentityRevoke {
                    identity_id: identity,
                    expected_revision,
                }),
                Command::Github(Github::History {
                    identity,
                    after,
                    limit,
                }) => Operation::GithubIdentityHistory {
                    identity_id: identity,
                    after,
                    limit,
                },
                Command::Workers(Workers::Eligible { project, limit }) => {
                    Operation::WorkerEligible {
                        project_id: project,
                        limit,
                    }
                }
                Command::Workers(Workers::List {
                    task,
                    project,
                    status,
                    after,
                    limit,
                }) => Operation::WorkerExecutionsList {
                    task_id: task,
                    project_id: project,
                    status,
                    after,
                    limit,
                },
                Command::Workers(Workers::Inspect { execution }) => Operation::WorkerExecutionGet {
                    execution_id: execution,
                },
                Command::Workers(Workers::History {
                    execution,
                    after,
                    limit,
                }) => Operation::WorkerExecutionHistory {
                    execution_id: execution,
                    after,
                    limit,
                },
                Command::Workers(Workers::Lease {
                    task,
                    worker_label,
                    lease_seconds,
                    max_active,
                    expected_task_revision,
                    expected_project_revision,
                }) => Operation::WorkerLease(WorkerLeaseInput {
                    task_id: task,
                    worker_label,
                    lease_seconds,
                    max_active,
                    expected_task_revision,
                    expected_project_revision,
                }),
                Command::Workers(Workers::Heartbeat {
                    execution,
                    expected_revision,
                    lease_seconds,
                }) => Operation::WorkerHeartbeat(WorkerHeartbeatInput {
                    execution_id: execution,
                    expected_revision,
                    lease_seconds,
                }),
                Command::Workers(Workers::Mock {
                    execution,
                    expected_revision,
                    lease_seconds,
                    summary,
                    work_remaining,
                    blockers,
                }) => Operation::WorkerMockRun(WorkerMockInput {
                    execution_id: execution,
                    expected_revision,
                    lease_seconds,
                    summary,
                    work_remaining,
                    blockers,
                }),
                Command::Workers(Workers::Release {
                    execution,
                    expected_revision,
                    outcome,
                    summary,
                }) => Operation::WorkerRelease(WorkerReleaseInput {
                    execution_id: execution,
                    expected_revision,
                    outcome,
                    summary,
                }),
                Command::Workers(Workers::Recover) => Operation::WorkerRecover,
                _ => unreachable!(),
            };
            submit(&dir, cli.request_id, operation, cli.json).await
        }
    }
}

async fn request_result(dir: &Path, operation: Operation) -> Result<serde_json::Value> {
    let request = Request {
        version: PROTOCOL_VERSION,
        request_id: uuid::Uuid::new_v4().to_string(),
        operation,
    };
    let response = runtime::call(dir, &request).await?;
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
        .ok_or_else(|| Error::Unavailable("runtime returned no result".into()))
}

async fn best_effort_abandon(dir: &Path, execution_id: &str) {
    let Ok(value) = request_result(
        dir,
        Operation::WorkerExecutionGet {
            execution_id: execution_id.into(),
        },
    )
    .await
    else {
        return;
    };
    if value["active"].as_bool() != Some(true) && value["process"]["status"] != "running" {
        return;
    }
    let _ = request_result(
        dir,
        Operation::WorkerAbandon(WorkerAbandonInput {
            execution_id: execution_id.into(),
            expected_revision: value["revision"].as_u64().unwrap(),
        }),
    )
    .await;
}

fn process_exists(pid: u32) -> bool {
    if unsafe { libc::kill(pid as i32, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn valid_commit(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64)
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn git(repository: &Path, arguments: &[&str]) -> Result<std::process::Output> {
    let output = ProcessCommand::new("git")
        .current_dir(repository)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(arguments)
        .output()?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        let detail = if detail.len() > 4096 {
            let mut end = 4096;
            while !detail.is_char_boundary(end) {
                end -= 1;
            }
            &detail[..end]
        } else {
            detail
        };
        return Err(Error::Conflict(format!(
            "git {} failed: {detail}",
            arguments.join(" ")
        )));
    }
    Ok(output)
}

fn prepare_worktree(dir: &Path, execution_id: &str, repository: &Path) -> Result<String> {
    let root = config::worker_execution_dir(dir, execution_id)?;
    let worktree = root.join("worktree");
    match std::fs::symlink_metadata(&worktree) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(Error::Conflict(
                "worker worktree path already exists; clean it up or retry with a new lease".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let head = git(repository, &["rev-parse", "--verify", "HEAD"])?;
    let commit = String::from_utf8(head.stdout)
        .map_err(|_| Error::Invalid("Git HEAD is not UTF-8".into()))?;
    let commit = commit.trim().to_owned();
    if !valid_commit(&commit) {
        return Err(Error::Conflict(
            "repository HEAD is not a supported Git object ID".into(),
        ));
    }
    let worktree_text = worktree
        .to_str()
        .ok_or_else(|| Error::Invalid("worker worktree path must be UTF-8".into()))?;
    git(
        repository,
        &["worktree", "add", "--detach", worktree_text, &commit],
    )?;
    let metadata = std::fs::symlink_metadata(&worktree)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Integrity(
            "git did not create a real worker worktree directory".into(),
        ));
    }
    Ok(commit)
}

fn capture(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        const LIMIT: usize = 64 * 1024;
        let mut result = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let room = LIMIT.saturating_sub(result.len());
                    result.extend_from_slice(&chunk[..read.min(room)]);
                }
            }
        }
        result
    })
}

fn write_log(dir: &Path, execution_id: &str, stdout: &[u8], stderr: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let root = config::worker_execution_dir(dir, execution_id)?;
    let path = root.join("worker.log");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(b"--- stdout ---\n")?;
    file.write_all(stdout)?;
    file.write_all(b"\n--- stderr ---\n")?;
    file.write_all(stderr)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn terminate(child: &mut std::process::Child) -> Result<std::process::ExitStatus> {
    let pid = child.id() as i32;
    // The child starts in its own process group, so terminate the whole group.
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    for _ in 0..20 {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    Ok(child.wait()?)
}

#[allow(clippy::too_many_arguments)]
async fn supervised_run(
    dir: &Path,
    task: String,
    worker_label: String,
    lease_seconds: u32,
    max_active: u32,
    timeout_seconds: u64,
    helper_delay_ms: u64,
    helper_exit_code: i32,
    compact: bool,
) -> Result<bool> {
    if !(3..=3600).contains(&lease_seconds) {
        return Err(Error::Invalid(
            "lease_seconds must be in 3..=3600 for supervised runs".into(),
        ));
    }
    if !(1..=3600).contains(&timeout_seconds) {
        return Err(Error::Invalid("timeout_seconds must be in 1..=3600".into()));
    }
    if helper_delay_ms > 600_000 {
        return Err(Error::Invalid(
            "helper_delay_ms must be at most 600000".into(),
        ));
    }
    if !(0..=255).contains(&helper_exit_code) {
        return Err(Error::Invalid("helper_exit_code must be in 0..=255".into()));
    }
    let leased = request_result(
        dir,
        Operation::WorkerLease(WorkerLeaseInput {
            task_id: task,
            worker_label,
            lease_seconds,
            max_active,
            expected_task_revision: None,
            expected_project_revision: None,
        }),
    )
    .await?;
    let execution_id = leased["id"].as_str().unwrap().to_owned();
    let project_id = leased["project_id"].as_str().unwrap().to_owned();
    let project = match request_result(
        dir,
        Operation::ProjectGet {
            project_id: project_id.clone(),
        },
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            best_effort_abandon(dir, &execution_id).await;
            return Err(error);
        }
    };
    let repository = PathBuf::from(project["repository"].as_str().unwrap());
    let base_commit = match prepare_worktree(dir, &execution_id, &repository) {
        Ok(value) => value,
        Err(error) => {
            best_effort_abandon(dir, &execution_id).await;
            return Err(error);
        }
    };
    let workspace = match request_result(
        dir,
        Operation::WorkerWorkspaceStart(WorkerWorkspaceInput {
            execution_id: execution_id.clone(),
            expected_revision: leased["revision"].as_u64().unwrap(),
            base_commit,
        }),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            best_effort_abandon(dir, &execution_id).await;
            return Err(error);
        }
    };
    let worktree = match config::worker_execution_dir(dir, &execution_id) {
        Ok(path) => path.join("worktree"),
        Err(error) => {
            best_effort_abandon(dir, &execution_id).await;
            return Err(error);
        }
    };
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            best_effort_abandon(dir, &execution_id).await;
            return Err(error.into());
        }
    };
    let mut child = match ProcessCommand::new(executable)
        .arg("__mock-worker")
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--execution")
        .arg(&execution_id)
        .arg("--delay-ms")
        .arg(helper_delay_ms.to_string())
        .arg("--exit-code")
        .arg(helper_exit_code.to_string())
        .current_dir(&worktree)
        .env_clear()
        .env("HOME", &worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            best_effort_abandon(dir, &execution_id).await;
            return Err(error.into());
        }
    };
    let stdout = child.stdout.take().map(capture);
    let stderr = child.stderr.take().map(capture);
    let attached = match request_result(
        dir,
        Operation::WorkerProcessAttach(WorkerProcessAttachInput {
            execution_id: execution_id.clone(),
            expected_revision: workspace["revision"].as_u64().unwrap(),
            lease_seconds,
            pid: child.id(),
        }),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            let _ = terminate(&mut child);
            best_effort_abandon(dir, &execution_id).await;
            return Err(error);
        }
    };
    let mut revision = attached["revision"].as_u64().unwrap();
    let interval = Duration::from_secs((lease_seconds / 3).clamp(1, 5) as u64);
    let started = Instant::now();
    let mut heartbeat = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            timed_out = true;
            break terminate(&mut child)?;
        }
        if heartbeat.elapsed() >= interval {
            let value = request_result(
                dir,
                Operation::WorkerHeartbeat(WorkerHeartbeatInput {
                    execution_id: execution_id.clone(),
                    expected_revision: revision,
                    lease_seconds,
                }),
            )
            .await;
            match value {
                Ok(value) => {
                    revision = value["revision"].as_u64().unwrap();
                    heartbeat = Instant::now();
                }
                Err(error) => {
                    let _ = terminate(&mut child);
                    let current = request_result(
                        dir,
                        Operation::WorkerExecutionGet {
                            execution_id: execution_id.clone(),
                        },
                    )
                    .await
                    .ok()
                    .filter(|value| value["process"]["status"] == "running");
                    if let Some(current) = current
                        && request_result(
                            dir,
                            Operation::WorkerProcessFinish(WorkerProcessFinishInput {
                                execution_id: execution_id.clone(),
                                expected_revision: current["revision"].as_u64().unwrap(),
                                outcome: WorkerProcessOutcome::Killed,
                                exit_code: None,
                            }),
                        )
                        .await
                        .is_err()
                    {
                        best_effort_abandon(dir, &execution_id).await;
                    }
                    return Err(error);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let stdout = stdout
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = stderr
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    if let Err(error) = write_log(dir, &execution_id, &stdout, &stderr) {
        eprintln!("worker process log could not be written: {error}");
    }
    let outcome = if timed_out {
        WorkerProcessOutcome::Killed
    } else if status.success() {
        WorkerProcessOutcome::Exited
    } else if status.code().is_some() {
        WorkerProcessOutcome::Failed
    } else {
        WorkerProcessOutcome::Killed
    };
    let finished = match request_result(
        dir,
        Operation::WorkerProcessFinish(WorkerProcessFinishInput {
            execution_id: execution_id.clone(),
            expected_revision: revision,
            outcome,
            exit_code: status.code(),
        }),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            best_effort_abandon(dir, &execution_id).await;
            return Err(error);
        }
    };
    output(&serde_json::json!({"result":finished}), compact)?;
    Ok(true)
}

async fn abandon_execution(
    dir: &Path,
    execution: String,
    force: bool,
    compact: bool,
) -> Result<bool> {
    let value = request_result(
        dir,
        Operation::WorkerExecutionGet {
            execution_id: execution.clone(),
        },
    )
    .await?;
    let process = &value["process"];
    if process["status"] == "running" && !force {
        let pid = process["pid"]
            .as_u64()
            .ok_or_else(|| Error::Integrity("running process has no PID".into()))?;
        if process_exists(pid as u32) {
            return Err(Error::Conflict(
                "recorded process PID still exists; stop it first or pass --force".into(),
            ));
        }
    }
    let value = request_result(
        dir,
        Operation::WorkerAbandon(WorkerAbandonInput {
            execution_id: execution,
            expected_revision: value["revision"].as_u64().unwrap(),
        }),
    )
    .await?;
    output(&serde_json::json!({"result":value}), compact)?;
    Ok(true)
}

async fn cleanup_workspace(dir: &Path, execution: String, compact: bool) -> Result<bool> {
    let value = request_result(
        dir,
        Operation::WorkerExecutionGet {
            execution_id: execution.clone(),
        },
    )
    .await?;
    if value["active"].as_bool() == Some(true) || value["process"]["status"] == "running" {
        return Err(Error::Conflict(
            "cannot clean up an active worker execution or running process".into(),
        ));
    }
    let path = value["workspace"]["path"]
        .as_str()
        .ok_or_else(|| Error::Conflict("worker execution has no workspace".into()))?;
    let project = request_result(
        dir,
        Operation::ProjectGet {
            project_id: value["project_id"].as_str().unwrap().into(),
        },
    )
    .await?;
    let repository = PathBuf::from(project["repository"].as_str().unwrap());
    if std::fs::symlink_metadata(path).is_ok() {
        git(&repository, &["worktree", "remove", "--force", path])?;
    }
    git(&repository, &["worktree", "prune"])?;
    let value = request_result(
        dir,
        Operation::WorkerWorkspaceCleanup(WorkerWorkspaceCleanupInput {
            execution_id: execution,
            expected_revision: value["revision"].as_u64().unwrap(),
        }),
    )
    .await?;
    output(&serde_json::json!({"result":value}), compact)?;
    Ok(true)
}

fn mock_worker(args: MockWorkerArgs) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if uuid::Uuid::parse_str(&args.execution).map_or(true, |id| id.to_string() != args.execution) {
        return Err(Error::Invalid(
            "execution must be a canonical lowercase UUID".into(),
        ));
    }
    if args.delay_ms > 600_000 || !(0..=255).contains(&args.exit_code) {
        return Err(Error::Invalid(
            "delay_ms must be <=600000 and exit_code must be in 0..=255".into(),
        ));
    }
    let deadline = Instant::now() + Duration::from_millis(args.delay_ms);
    while Instant::now() < deadline {
        let alive = unsafe { libc::kill(args.parent_pid as i32, 0) };
        if alive != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Err(Error::Unavailable("supervisor exited".into()));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    if args.exit_code != 0 {
        eprintln!(
            "deterministic mock worker exiting with code {}",
            args.exit_code
        );
        std::process::exit(args.exit_code);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open("worker-output.txt")?;
    writeln!(file, "mock worker execution {}", args.execution)?;
    file.sync_all()?;
    serde_json::to_writer_pretty(
        std::io::stdout(),
        &serde_json::json!({
            "execution_id":args.execution,
            "artifact":"worker-output.txt",
            "tests_executed":false,
            "completion":false
        }),
    )?;
    println!();
    Ok(())
}

async fn submit(
    dir: &std::path::Path,
    request_id: Option<String>,
    operation: Operation,
    compact: bool,
) -> Result<bool> {
    let request = Request {
        version: PROTOCOL_VERSION,
        request_id: request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        operation,
    };
    let response = runtime::call(dir, &request).await?;
    let ok = response.error.is_none();
    output(&response, compact)?;
    Ok(ok)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let compact = cli.json;
    let mcp_stdio = matches!(cli.command, Command::Mcp);
    let code = match run(cli).await {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(error) => {
            // Avoid logging request bodies or database errors containing submitted text.
            let value =
                serde_json::json!({"error":{"code":"command_failed","message":error.to_string()}});
            if mcp_stdio {
                eprintln!("motherctl MCP bridge failed; check runtime and data directory");
            } else {
                let _ = output(&value, compact);
            }
            1
        }
    };
    std::process::exit(code);
}
