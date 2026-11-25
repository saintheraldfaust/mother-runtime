//! Owner-invoked dashboard actions; persistence stays behind IPC and execution behind the CLI supervisor.
use super::{call, call_request};
use crate::{
    Error, Result, config,
    protocol::{Operation, PROTOCOL_VERSION, Request},
    tasks::{TaskState, TaskTransition},
};
use serde_json::Value;
use std::{
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::sync::mpsc;

pub(super) enum Action {
    Submit(Box<Request>),
    Ready {
        task_id: String,
        revision: u64,
        state: TaskState,
    },
    Run {
        task_id: String,
        revision: u64,
        timeout: u64,
        steps: u32,
        model: String,
    },
    Stop {
        worker_label: String,
    },
}

pub(super) struct CommandRequest {
    pub id: String,
    pub action: Action,
}

#[derive(Default)]
pub(super) struct Effect {
    pub message: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
}

pub(super) struct Feedback {
    pub id: Option<String>,
    pub result: Result<Effect>,
}

struct Job {
    child: Child,
    task_id: String,
    label: String,
    output: PathBuf,
    diagnostics: PathBuf,
    stopping: bool,
}

pub(super) fn ready_states(state: TaskState) -> Result<Vec<TaskState>> {
    use TaskState::*;
    match state {
        Created | Failed | Blocked => Ok(vec![Triaged, Ready]),
        Triaged | AwaitingDependency | AwaitingInput => Ok(vec![Ready]),
        _ => Err(Error::Invalid(
            "select a created, triaged, blocked, failed or waiting task to mark ready".into(),
        )),
    }
}

fn response_effect(operation: &Operation, value: Value) -> Effect {
    match operation {
        Operation::ProjectAttach { .. } => Effect {
            message: "Project registered. Use n to create a task.".into(),
            project_id: value["id"].as_str().map(str::to_owned),
            ..Default::default()
        },
        Operation::TaskCreate(_) => Effect {
            message: "Task created. Use t to mark it ready, then r to start Codex.".into(),
            task_id: value["id"].as_str().map(str::to_owned),
            ..Default::default()
        },
        _ => Effect {
            message: "State updated.".into(),
            ..Default::default()
        },
    }
}

async fn apply(dir: &Path, command: &CommandRequest, jobs: &mut Vec<Job>) -> Result<Effect> {
    match &command.action {
        Action::Submit(request) => {
            let value = call_request(dir, request).await?;
            Ok(response_effect(&request.operation, value))
        }
        Action::Ready {
            task_id,
            revision,
            state,
        } => {
            // The same command ID/revisions are reused on retry. Each step has its own retry key.
            let states = ready_states(*state)?;
            let mut revision = *revision;
            for (index, state) in states.into_iter().enumerate() {
                let value = call_request(
                    dir,
                    &Request {
                        version: PROTOCOL_VERSION,
                        request_id: format!("{}/{}", command.id, index),
                        operation: Operation::TaskTransition(TaskTransition {
                            task_id: task_id.clone(),
                            state,
                            source: "tui".into(),
                            reason: "Operator marked task ready in the dashboard".into(),
                            owner_label: None,
                            persona: None,
                            expected_task_revision: revision,
                            expected_project_revision: None,
                        }),
                    },
                )
                .await?;
                revision = value["task_revision"]
                    .as_u64()
                    .ok_or_else(|| Error::Integrity("task response lacks a revision".into()))?;
            }
            Ok(Effect {
                message: "Task is ready. Use r to configure and start Codex.".into(),
                task_id: Some(task_id.clone()),
                ..Default::default()
            })
        }
        Action::Run {
            task_id,
            revision,
            timeout,
            steps,
            model,
        } => {
            // Do not spawn twice when a launch confirmation is retried in this session.
            let label = format!("tui-{}", command.id);
            if jobs.iter().any(|job| job.label == label) {
                return Ok(Effect {
                    message: "This worker launch is already being supervised.".into(),
                    ..Default::default()
                });
            }
            let task = call(
                dir,
                Operation::TaskGet {
                    task_id: task_id.clone(),
                },
            )
            .await?;
            if task["state"] != "ready" || task["task_revision"].as_u64() != Some(*revision) {
                return Err(Error::Conflict(
                    "task changed or is not ready; close this form and review its current state"
                        .into(),
                ));
            }
            let project = call(
                dir,
                Operation::ProjectGet {
                    project_id: task["project_id"].as_str().unwrap_or_default().into(),
                },
            )
            .await?;
            let repository = PathBuf::from(
                project["repository"]
                    .as_str()
                    .ok_or_else(|| Error::Integrity("project has no repository".into()))?,
            );
            // Read-only preflight avoids acquiring a lease for an empty Git repository.
            let committed = tokio::task::spawn_blocking(move || {
                Command::new("git")
                    .args([
                        "--no-optional-locks",
                        "-c",
                        "core.fsmonitor=false",
                        "-c",
                        "core.hooksPath=/dev/null",
                        "rev-parse",
                        "--verify",
                        "HEAD",
                    ])
                    .current_dir(repository)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
            })
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))??;
            if !committed.success() {
                return Err(Error::Invalid("worker execution requires a Git repository with an initial commit; commit the project baseline, then retry".into()));
            }
            let output = dir.join(format!("tui-{}.json", command.id));
            let diagnostics = dir.join(format!("tui-{}.log", command.id));
            let mut process = Command::new(std::env::current_exe()?);
            process
                .arg("--data-dir")
                .arg(dir)
                .arg("--json")
                .args(["workers", "run", "--adapter", "codex", "--task"])
                .arg(task_id)
                .arg("--worker-label")
                .arg(&label)
                .arg("--expected-task-revision")
                .arg(revision.to_string())
                .arg("--timeout-seconds")
                .arg(timeout.to_string())
                .arg("--max-steps")
                .arg(steps.to_string());
            if !model.trim().is_empty() {
                process.arg("--model").arg(model.trim());
            }
            let child = process
                .stdin(Stdio::null())
                .stdout(config::private_file(&output)?)
                .stderr(config::private_file(&diagnostics)?)
                .process_group(0)
                .spawn()?;
            jobs.push(Job {
                child,
                task_id: task_id.clone(),
                label,
                output,
                diagnostics,
                stopping: false,
            });
            Ok(Effect {message:"Codex supervisor started. Watch Tasks and Workers for the canonical execution state.".into(),task_id:Some(task_id.clone()),..Default::default()})
        }
        Action::Stop { worker_label } => {
            let job = jobs
                .iter_mut()
                .find(|j| j.label == *worker_label)
                .ok_or_else(|| {
                    Error::Invalid(
                        "only workers launched by this dashboard session can be stopped here"
                            .into(),
                    )
                })?;
            if job.child.try_wait()?.is_some() {
                return Err(Error::Conflict(
                    "the supervisor has already exited; refresh the worker details".into(),
                ));
            }
            if job.stopping {
                return Ok(Effect {
                    message: "Stop already requested; waiting for process termination.".into(),
                    ..Default::default()
                });
            }
            // This is a child we own and have not reaped; no PID from stored metadata is signaled.
            if unsafe { libc::kill(job.child.id() as i32, libc::SIGTERM) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            job.stopping = true;
            Ok(Effect {message:"Stop requested. Waiting for the supervisor to terminate its process group and record the result.".into(),..Default::default()})
        }
    }
}

fn finished(job: &Job, status: std::process::ExitStatus) -> Effect {
    let message=(|| -> Result<String> {
        let mut text=String::new();
        config::read_private_file(&job.output)?.take(1024*1024).read_to_string(&mut text)?;
        let value:Value=serde_json::from_str(&text)?;
        let worker=&value["result"];
        if worker["worker_label"]!=job.label || worker["task_id"]!=job.task_id {return Err(Error::Integrity("supervisor result does not match the launch".into()));}
        let outcome=super::field(&worker["process"],"status");
        let state=super::field(worker,"task_state");
        Ok(format!("Codex process {outcome}; task {state}. Worktree retained for inspection."))
    })().unwrap_or_else(|_|format!("Supervisor exited ({status}); inspect Workers and the private diagnostic file {}. Refresh task state before retrying.",job.diagnostics.display()));
    Effect {
        message,
        task_id: Some(job.task_id.clone()),
        ..Default::default()
    }
}

pub(super) async fn serve(
    dir: PathBuf,
    mut commands: mpsc::Receiver<CommandRequest>,
    feedback: mpsc::UnboundedSender<Feedback>,
) {
    let mut jobs: Vec<Job> = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            command=commands.recv()=>{
                let Some(command)=command else {break;};
                let result=apply(&dir,&command,&mut jobs).await;
                let _=feedback.send(Feedback {id:Some(command.id),result});
            }
            _=tick.tick()=>{
                let mut index=0;
                while index<jobs.len() {
                    match jobs[index].child.try_wait() {
                        Ok(Some(status))=>{let job=jobs.remove(index);let _=feedback.send(Feedback {id:None,result:Ok(finished(&job,status))});}
                        Ok(None)=>index+=1,
                        Err(error)=>{let job=jobs.remove(index);let _=feedback.send(Feedback {id:None,result:Err(Error::Unavailable(format!("cannot observe supervisor for {}: {error}",job.task_id)))});}
                    }
                }
            }
        }
    }
    // Dropping Child handles does not kill processes. Supervisors have separate process groups
    // and private file output, so explicit dashboard detachment leaves them operational.
}
