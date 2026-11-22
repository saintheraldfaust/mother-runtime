//! Durable execution leases for deterministic external runners.
//!
//! A lease records coordination intent. It does not prove execution, grant
//! identity, supervise a process, or authorize repository writes.
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkerExecutionStatus {
    Leased,
    Running,
    Released,
    Failed,
    Expired,
    Completed,
}

impl WorkerExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Released => "released",
            Self::Failed => "failed",
            Self::Expired => "expired",
            Self::Completed => "completed",
        }
    }

    pub fn active(self) -> bool {
        matches!(self, Self::Leased | Self::Running)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkerReleaseOutcome {
    Released,
    Failed,
    Completed,
}

impl WorkerReleaseOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }

    pub fn status(self) -> WorkerExecutionStatus {
        match self {
            Self::Released => WorkerExecutionStatus::Released,
            Self::Failed => WorkerExecutionStatus::Failed,
            Self::Completed => WorkerExecutionStatus::Completed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerLeaseInput {
    pub task_id: String,
    pub worker_label: String,
    /// Requested lease duration in seconds.
    #[schemars(range(min = 1, max = 3600))]
    pub lease_seconds: u32,
    /// Maximum unexpired active leases allowed across all projects.
    #[schemars(range(min = 1, max = 64))]
    pub max_active: u32,
    pub expected_task_revision: Option<u64>,
    pub expected_project_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerHeartbeatInput {
    pub execution_id: String,
    pub expected_revision: u64,
    #[schemars(range(min = 1, max = 3600))]
    pub lease_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerMockInput {
    pub execution_id: String,
    pub expected_revision: u64,
    #[schemars(range(min = 1, max = 3600))]
    pub lease_seconds: u32,
    pub summary: String,
    #[serde(default)]
    pub work_remaining: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerReleaseInput {
    pub execution_id: String,
    pub expected_revision: u64,
    pub outcome: WorkerReleaseOutcome,
    pub summary: Option<String>,
}
