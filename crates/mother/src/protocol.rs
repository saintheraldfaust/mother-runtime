use crate::continuity::{DecisionInput, HandoffInput};
use crate::personas::{PersonaInput, PersonaStatus, PersonaStatusChange, PersonaUpdate};
use crate::tasks::{CheckpointInput, TaskInput, TaskState, TaskTransition};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 65_536;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub request_id: String,
    pub operation: Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Operation {
    Health,
    ProjectAttach {
        name: String,
        repository: String,
    },
    ProjectsList {
        after: u64,
        limit: u32,
    },
    ProjectGet {
        project_id: String,
    },
    ContextRecord {
        project_id: String,
        kind: ContextKind,
        content: String,
        source: String,
        expected_revision: Option<u64>,
    },
    ContextGet {
        project_id: String,
        after: u64,
        limit: u32,
    },
    ActivityGet {
        project_id: Option<String>,
        after: u64,
        limit: u32,
    },
    DecisionRecord(DecisionInput),
    DecisionsList {
        project_id: String,
        include_superseded: bool,
        after: u64,
        limit: u32,
    },
    DecisionGet {
        decision_id: String,
    },
    HandoffCreate(Box<HandoffInput>),
    HandoffsList {
        project_id: String,
        after: u64,
        limit: u32,
    },
    HandoffGet {
        handoff_id: String,
    },
    HandoffResume {
        handoff_id: String,
        recipient: String,
        expected_revision: u64,
    },
    TaskCreate(TaskInput),
    TasksList {
        project_id: String,
        state: Option<TaskState>,
        after: u64,
        limit: u32,
    },
    TaskGet {
        task_id: String,
    },
    TaskTransition(TaskTransition),
    TaskCheckpoint(CheckpointInput),
    TaskCheckpointsList {
        task_id: String,
        after: u64,
        limit: u32,
    },
    TaskTransitionsList {
        task_id: String,
        after: u64,
        limit: u32,
    },
    PersonaCreate(PersonaInput),
    PersonaUpdate(PersonaUpdate),
    PersonaSetStatus(PersonaStatusChange),
    PersonaGet {
        persona_id: String,
    },
    PersonasList {
        status: Option<PersonaStatus>,
        project_id: Option<String>,
        after: u64,
        limit: u32,
    },
    PersonaHistory {
        persona_id: String,
        after: u64,
        limit: u32,
    },
}

impl Operation {
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            Self::ProjectAttach { .. }
                | Self::ContextRecord { .. }
                | Self::DecisionRecord(_)
                | Self::HandoffCreate(_)
                | Self::HandoffResume { .. }
                | Self::TaskCreate(_)
                | Self::TaskTransition(_)
                | Self::TaskCheckpoint(_)
                | Self::PersonaCreate(_)
                | Self::PersonaUpdate(_)
                | Self::PersonaSetStatus(_)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Fact,
    Observation,
    Failure,
    Blocker,
    Objective,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Response {
    pub version: u32,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

impl Response {
    pub fn success(id: String, value: Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id: id,
            result: Some(value),
            error: None,
        }
    }
    pub fn failure(id: String, code: &str, message: &str) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id: id,
            result: None,
            error: Some(Failure {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}
