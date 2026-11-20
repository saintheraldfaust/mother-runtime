use clap::{Parser, Subcommand};
use mother_runtime::{
    Error, Result, config,
    continuity::{DecisionInput, HandoffInput},
    mcp,
    protocol::{ContextKind, Operation, PROTOCOL_VERSION, Request},
    runtime,
    tasks::{CheckpointInput, TaskInput, TaskState, TaskTransition},
};
use std::path::PathBuf;

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
                    expected_task_revision,
                    expected_project_revision,
                }) => Operation::TaskTransition(TaskTransition {
                    task_id: task,
                    state,
                    source,
                    reason,
                    owner_label,
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
                _ => unreachable!(),
            };
            let request = Request {
                version: PROTOCOL_VERSION,
                request_id: cli
                    .request_id
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                operation,
            };
            let response = runtime::call(&dir, &request).await?;
            let ok = response.error.is_none();
            output(&response, cli.json)?;
            Ok(ok)
        }
    }
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
