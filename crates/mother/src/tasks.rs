//! Host-tracked tasks. Labels and reported tests do not authenticate people or prove execution.
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Created,
    Triaged,
    Ready,
    Assigned,
    Running,
    AwaitingDependency,
    AwaitingInput,
    Validating,
    AwaitingReview,
    ChangesRequested,
    ReadyForHumanApproval,
    Completed,
    Failed,
    Cancelled,
    Blocked,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Triaged => "triaged",
            Self::Ready => "ready",
            Self::Assigned => "assigned",
            Self::Running => "running",
            Self::AwaitingDependency => "awaiting_dependency",
            Self::AwaitingInput => "awaiting_input",
            Self::Validating => "validating",
            Self::AwaitingReview => "awaiting_review",
            Self::ChangesRequested => "changes_requested",
            Self::ReadyForHumanApproval => "ready_for_human_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
    pub fn permits(self, next: Self) -> bool {
        use TaskState::*;
        match self {
            Created => matches!(next, Triaged | Cancelled),
            Triaged => matches!(
                next,
                Ready | AwaitingDependency | AwaitingInput | Blocked | Cancelled
            ),
            Ready => matches!(next, Assigned | AwaitingDependency | Blocked | Cancelled),
            Assigned => matches!(next, Running | Ready | Blocked | Cancelled),
            Running => matches!(
                next,
                AwaitingDependency | AwaitingInput | Validating | Failed | Blocked | Cancelled
            ),
            AwaitingDependency => matches!(next, Ready | Blocked | Cancelled),
            AwaitingInput => matches!(next, Ready | Running | Blocked | Cancelled),
            Validating => matches!(
                next,
                AwaitingReview | ChangesRequested | Completed | Failed | Blocked | Cancelled
            ),
            AwaitingReview => matches!(
                next,
                ChangesRequested | ReadyForHumanApproval | Blocked | Cancelled
            ),
            ChangesRequested => matches!(next, Assigned | Blocked | Cancelled),
            ReadyForHumanApproval => matches!(next, ChangesRequested | Blocked | Cancelled),
            Blocked => matches!(next, Triaged | Cancelled),
            Failed => matches!(next, Triaged | Cancelled),
            Completed | Cancelled => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCriterion {
    /// Normalized path relative to the registered repository, without symlinks.
    pub path: String,
    /// When absent, the contract checks presence and unchanged checkpoint content only.
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    pub project_id: String,
    pub title: String,
    pub objective: String,
    pub source: String,
    pub acceptance_criteria: Vec<ArtifactCriterion>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub review_required: bool,
    pub expected_project_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskTransition {
    pub task_id: String,
    pub state: TaskState,
    pub source: String,
    pub reason: String,
    pub owner_label: Option<String>,
    pub expected_task_revision: u64,
    pub expected_project_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckpointInput {
    pub task_id: String,
    pub summary: String,
    pub source: String,
    #[serde(default)]
    pub work_remaining: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub reported_test_results: Vec<String>,
    pub expected_task_revision: u64,
    pub expected_project_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ArtifactObservation {
    pub path: String,
    pub status: String,
    pub sha256: Option<String>,
    pub bytes: Option<u64>,
}
