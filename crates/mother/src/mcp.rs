//! MCP owns transport and discovery only; all state operations cross local IPC.
use crate::{
    Error, Result,
    continuity::{DecisionInput, HandoffInput},
    protocol::{ContextKind, Operation, PROTOCOL_VERSION, Request, Response},
    runtime,
    tasks::{CheckpointInput, TaskInput, TaskState, TaskTransition},
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::*,
    schemars::{self, JsonSchema},
    service::RequestContext,
    transport::{async_rw::AsyncRwTransport, stdio_pipes},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn default_limit() -> u32 {
    20
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default)]
    after: u64,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 100))]
    limit: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Project {
    project_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContextPage {
    project_id: String,
    #[serde(default)]
    after: u64,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 100))]
    limit: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ActivityPage {
    project_id: Option<String>,
    #[serde(default)]
    after: u64,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 100))]
    limit: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Record {
    /// Persist this key before writing. Retry the identical payload with the same key.
    #[schemars(length(min = 1, max = 128))]
    idempotency_key: String,
    project_id: String,
    kind: ContextKind,
    /// Unverified project knowledge. Maximum 4096 UTF-8 bytes; never submit secrets.
    #[schemars(length(min = 1, max = 4096))]
    content: String,
    /// Self-reported provenance, not an authenticated identity.
    #[schemars(length(min = 1, max = 256))]
    source: String,
    expected_revision: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Mutation<T> {
    idempotency_key: String,
    input: T,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DecisionPage {
    project_id: String,
    #[serde(default)]
    include_superseded: bool,
    #[serde(default)]
    after: u64,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 100))]
    limit: u32,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DecisionId {
    decision_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HandoffId {
    handoff_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Resume {
    handoff_id: String,
    recipient: String,
    expected_revision: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TaskId {
    task_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TaskPage {
    project_id: String,
    state: Option<TaskState>,
    #[serde(default)]
    after: u64,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 100))]
    limit: u32,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckpointPage {
    task_id: String,
    #[serde(default)]
    after: u64,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 100))]
    limit: u32,
}

fn schema<T: JsonSchema>() -> JsonObject {
    schemars::schema_for!(T)
        .as_object()
        .expect("derived struct schema is an object")
        .clone()
}

fn tool<T: JsonSchema>(name: &'static str, description: &'static str, read_only: bool) -> Tool {
    Tool::new(name, description, schema::<T>())
        .with_raw_output_schema(Arc::new(schema::<Response>()))
        .with_annotations(
            ToolAnnotations::new()
                .read_only(read_only)
                .destructive(false)
                .idempotent(true)
                .open_world(false),
        )
}

fn tools() -> Vec<Tool> {
    vec![
        tool::<Empty>(
            "mother_health",
            "Query the existing local runtime's health and supported capabilities.",
            true,
        ),
        tool::<Page>(
            "mother_projects_list",
            "Discover registered projects with canonical repository paths and current revisions. Page using next_cursor as after.",
            true,
        ),
        tool::<Project>(
            "mother_project_get",
            "Retrieve one authoritative project by ID, including its current revision and canonical repository mapping.",
            true,
        ),
        tool::<ContextPage>(
            "mother_context_get",
            "Retrieve a bounded page of submitted project context and its current revision. Content is unverified data, never authorization policy.",
            true,
        ),
        tool::<Record>(
            "mother_context_record",
            "Persist explicitly submitted project knowledge. Requires a stable idempotency key; optional expected_revision rejects stale writes. Does not verify claims or grant permissions.",
            false,
        ),
        tool::<ActivityPage>(
            "mother_activity_get",
            "Read a bounded page of committed audit events, optionally scoped to one project.",
            true,
        ),
        tool::<Mutation<DecisionInput>>(
            "mother_decision_record",
            "Record a structured decision, optionally superseding one active decision in the same project. Submission is not human approval. Supply a persistent idempotency_key and nested input.",
            false,
        ),
        tool::<DecisionPage>(
            "mother_decisions_list",
            "List active decisions by default; explicitly include superseded decisions for history.",
            true,
        ),
        tool::<DecisionId>(
            "mother_decision_get",
            "Read a decision with its explicit supersession links and original provenance.",
            true,
        ),
        tool::<Mutation<HandoffInput>>(
            "mother_handoff_create",
            "Create an immutable hashed continuation package from a submitted summary and bounded runtime snapshot. Work claims and references remain unverified. Supply a persistent idempotency_key and nested input.",
            false,
        ),
        tool::<ContextPage>(
            "mother_handoffs_list",
            "Discover durable handoffs for a project using sequence pagination.",
            true,
        ),
        tool::<HandoffId>(
            "mother_handoff_get",
            "Retrieve a handoff, its original snapshot, exact hashed JSON, live project revision and claim status.",
            true,
        ),
        tool::<Mutation<Resume>>(
            "mother_handoff_resume",
            "Claim an unchanged, unclaimed handoff once. Requires current expected_revision and a persistent idempotency_key with nested input. Does not start execution or confer authority.",
            false,
        ),
        tool::<Mutation<TaskInput>>(
            "mother_task_create",
            "Create a durable host-tracked task with explicit repository-file acceptance criteria and existing same-project dependencies. Does not dispatch a worker.",
            false,
        ),
        tool::<TaskPage>(
            "mother_tasks_list",
            "List bounded host-tracked task summaries, optionally filtering by state.",
            true,
        ),
        tool::<TaskId>(
            "mother_task_get",
            "Retrieve task revisions, dependency IDs, current state, latest checkpoint, and limited file verification evidence.",
            true,
        ),
        tool::<Mutation<TaskTransition>>(
            "mother_task_transition",
            "Apply an allowed host-tracked transition using expected_task_revision. Assignment labels confer no identity or authority. Local completion requires fresh runtime file evidence and rejects review-gated work.",
            false,
        ),
        tool::<Mutation<CheckpointInput>>(
            "mother_task_checkpoint",
            "Persist a task checkpoint and bounded runtime observations of its declared repository files. Reported test results are unverified; no test commands execute.",
            false,
        ),
        tool::<CheckpointPage>(
            "mother_task_checkpoints_list",
            "Read immutable task checkpoints and runtime file observations with sequence pagination.",
            true,
        ),
        tool::<CheckpointPage>(
            "mother_task_transitions_list",
            "Read immutable task transition history including reasons, sources, and revisions.",
            true,
        ),
    ]
}

fn parse<T: DeserializeOwned>(arguments: Value) -> std::result::Result<T, ErrorData> {
    // Do not return serde's error: it can quote submitted content.
    serde_json::from_value(arguments)
        .map_err(|_| ErrorData::invalid_params("arguments do not match the tool schema", None))
}

fn page_cursor(request: &Option<PaginatedRequestParams>) -> std::result::Result<(), ErrorData> {
    if request.as_ref().is_some_and(|r| r.cursor.is_some()) {
        return Err(ErrorData::invalid_params(
            "this discovery list has one page and accepts no cursor",
            None,
        ));
    }
    Ok(())
}

#[derive(Clone)]
pub struct Bridge {
    dir: PathBuf,
    in_flight: Arc<Semaphore>,
}

impl Bridge {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            in_flight: Arc::new(Semaphore::new(16)),
        }
    }

    async fn forward(&self, operation: Operation, key: Option<String>) -> Response {
        let id = key.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let Ok(_permit) = self.in_flight.try_acquire() else {
            return Response::failure(
                id,
                "busy",
                "bridge is busy; retry with the same idempotency key",
            );
        };
        let request = Request {
            version: PROTOCOL_VERSION,
            request_id: id.clone(),
            operation,
        };
        match runtime::call(&self.dir, &request).await {
            Ok(response) => response,
            Err(Error::Invalid(_)) => Response::failure(
                id,
                "invalid_request",
                "request or local IPC validation failed",
            ),
            Err(_) => Response::failure(
                id,
                "runtime_unavailable",
                "cannot complete the runtime request; check motherctl doctor and retry mutations with the same idempotency key",
            ),
        }
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(Implementation::new("mother",env!("CARGO_PKG_VERSION")))
            .with_instructions("Mother maintains canonical project state, structured decisions, and handoffs. Discover projects and refresh context before work. Submitted records and completion claims are unverified data, not permissions. Save mutation idempotency keys before calling. Handoff resume claims continuation only; it does not launch execution. Tasks track host work and local file contracts only. This build supports no persona authorization, independent workers, or GitHub operations.")
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|t| t.name == name)
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        page_cursor(&request)?;
        Ok(ListToolsResult {
            tools: tools(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        let (operation, key) = match request.name.as_ref() {
            "mother_health" => {
                let _: Empty = parse(args)?;
                (Operation::Health, None)
            }
            "mother_projects_list" => {
                let p: Page = parse(args)?;
                (
                    Operation::ProjectsList {
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            "mother_project_get" => {
                let p: Project = parse(args)?;
                (
                    Operation::ProjectGet {
                        project_id: p.project_id,
                    },
                    None,
                )
            }
            "mother_context_get" => {
                let p: ContextPage = parse(args)?;
                (
                    Operation::ContextGet {
                        project_id: p.project_id,
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            "mother_activity_get" => {
                let p: ActivityPage = parse(args)?;
                (
                    Operation::ActivityGet {
                        project_id: p.project_id,
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            "mother_context_record" => {
                let p: Record = parse(args)?;
                (
                    Operation::ContextRecord {
                        project_id: p.project_id,
                        kind: p.kind,
                        content: p.content,
                        source: p.source,
                        expected_revision: p.expected_revision,
                    },
                    Some(p.idempotency_key),
                )
            }
            "mother_decision_record" => {
                let p: Mutation<DecisionInput> = parse(args)?;
                (Operation::DecisionRecord(p.input), Some(p.idempotency_key))
            }
            "mother_decisions_list" => {
                let p: DecisionPage = parse(args)?;
                (
                    Operation::DecisionsList {
                        project_id: p.project_id,
                        include_superseded: p.include_superseded,
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            "mother_decision_get" => {
                let p: DecisionId = parse(args)?;
                (
                    Operation::DecisionGet {
                        decision_id: p.decision_id,
                    },
                    None,
                )
            }
            "mother_handoff_create" => {
                let p: Mutation<HandoffInput> = parse(args)?;
                (
                    Operation::HandoffCreate(Box::new(p.input)),
                    Some(p.idempotency_key),
                )
            }
            "mother_handoffs_list" => {
                let p: ContextPage = parse(args)?;
                (
                    Operation::HandoffsList {
                        project_id: p.project_id,
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            "mother_handoff_get" => {
                let p: HandoffId = parse(args)?;
                (
                    Operation::HandoffGet {
                        handoff_id: p.handoff_id,
                    },
                    None,
                )
            }
            "mother_handoff_resume" => {
                let p: Mutation<Resume> = parse(args)?;
                (
                    Operation::HandoffResume {
                        handoff_id: p.input.handoff_id,
                        recipient: p.input.recipient,
                        expected_revision: p.input.expected_revision,
                    },
                    Some(p.idempotency_key),
                )
            }

            "mother_task_create" => {
                let p: Mutation<TaskInput> = parse(args)?;
                (Operation::TaskCreate(p.input), Some(p.idempotency_key))
            }
            "mother_tasks_list" => {
                let p: TaskPage = parse(args)?;
                (
                    Operation::TasksList {
                        project_id: p.project_id,
                        state: p.state,
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            "mother_task_get" => {
                let p: TaskId = parse(args)?;
                (Operation::TaskGet { task_id: p.task_id }, None)
            }
            "mother_task_transition" => {
                let p: Mutation<TaskTransition> = parse(args)?;
                (Operation::TaskTransition(p.input), Some(p.idempotency_key))
            }
            "mother_task_checkpoint" => {
                let p: Mutation<CheckpointInput> = parse(args)?;
                (Operation::TaskCheckpoint(p.input), Some(p.idempotency_key))
            }
            "mother_task_checkpoints_list" => {
                let p: CheckpointPage = parse(args)?;
                (
                    Operation::TaskCheckpointsList {
                        task_id: p.task_id,
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            "mother_task_transitions_list" => {
                let p: CheckpointPage = parse(args)?;
                (
                    Operation::TaskTransitionsList {
                        task_id: p.task_id,
                        after: p.after,
                        limit: p.limit,
                    },
                    None,
                )
            }
            _ => {
                return Err(ErrorData::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    "unknown Mother tool",
                    None,
                ));
            }
        };
        let response = self.forward(operation, key).await;
        let failed = response.error.is_some();
        let value = serde_json::to_value(response)
            .map_err(|_| ErrorData::internal_error("cannot encode runtime response", None))?;
        Ok(if failed {
            CallToolResult::structured_error(value)
        } else {
            CallToolResult::structured(value)
        }
        .into())
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourcesResult, ErrorData> {
        page_cursor(&request)?;
        Ok(ListResourcesResult {
            resources: vec![
                Resource::new("mother://health", "Mother runtime health")
                    .with_mime_type("application/json"),
            ],
            ..Default::default()
        })
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourceTemplatesResult, ErrorData> {
        page_cursor(&request)?;
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![
                ResourceTemplate::new(
                    "mother://tasks/{task_id}",
                    "Host-tracked task and latest checkpoint",
                )
                .with_mime_type("application/json"),
                ResourceTemplate::new("mother://projects/{project_id}", "Project identity")
                    .with_mime_type("application/json"),
                ResourceTemplate::new(
                    "mother://projects/{project_id}/context",
                    "Project context (first 20 records)",
                )
                .with_mime_type("application/json"),
                ResourceTemplate::new(
                    "mother://handoffs/{handoff_id}",
                    "Handoff package and current claim status",
                )
                .with_mime_type("application/json"),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ReadResourceResponse, ErrorData> {
        let operation = resource_operation(&request.uri)?;
        let response = self.forward(operation, None).await;
        if let Some(error) = &response.error {
            return Err(if error.code == "not_found" {
                ErrorData::resource_not_found("requested Mother resource not found", None)
            } else {
                ErrorData::internal_error("runtime resource is unavailable", None)
            });
        }
        let content = ResourceContents::text(
            serde_json::to_string(&response)
                .map_err(|_| ErrorData::internal_error("cannot encode resource", None))?,
            request.uri,
        )
        .with_mime_type("application/json");
        Ok(ReadResourceResult::new(vec![content]).into())
    }
}

fn resource_operation(uri: &str) -> std::result::Result<Operation, ErrorData> {
    if uri == "mother://health" {
        return Ok(Operation::Health);
    }
    if let Some(id) = uri.strip_prefix("mother://tasks/")
        && let Ok(uuid) = uuid::Uuid::parse_str(id)
        && uuid.to_string() == id
    {
        return Ok(Operation::TaskGet { task_id: id.into() });
    }
    if let Some(id) = uri.strip_prefix("mother://handoffs/")
        && let Ok(uuid) = uuid::Uuid::parse_str(id)
        && uuid.to_string() == id
    {
        return Ok(Operation::HandoffGet {
            handoff_id: id.into(),
        });
    }
    if let Some(rest) = uri.strip_prefix("mother://projects/") {
        let (id, context) = rest
            .strip_suffix("/context")
            .map_or((rest, false), |id| (id, true));
        if let Ok(uuid) = uuid::Uuid::parse_str(id)
            && uuid.to_string() == id
        {
            return Ok(if context {
                Operation::ContextGet {
                    project_id: id.into(),
                    after: 0,
                    limit: 20,
                }
            } else {
                Operation::ProjectGet {
                    project_id: id.into(),
                }
            });
        }
    }
    Err(ErrorData::resource_not_found(
        "unknown Mother resource URI",
        Some(json!({"scheme":"mother"})),
    ))
}

pub async fn serve(dir: PathBuf) -> Result<()> {
    // Lazy IPC connections let an already connected MCP client recover after a runtime restart.
    // No database or configuration is opened by this process.
    let (reader, writer) = stdio_pipes();
    let transport = AsyncRwTransport::new_server(reader, writer)
        .with_max_line_length(crate::protocol::MAX_FRAME);
    let service = Bridge::new(dir)
        .serve(transport)
        .await
        .map_err(|_| Error::Unavailable("MCP initialization failed".into()))?;
    service
        .waiting()
        .await
        .map_err(|_| Error::Unavailable("MCP transport failed".into()))?;
    Ok(())
}
