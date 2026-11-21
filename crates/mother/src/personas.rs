//! Durable personas define engineering responsibilities, not model prompts or workers.
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    #[serde(rename = "context.record")]
    ContextRecord,
    #[serde(rename = "decision.record")]
    DecisionRecord,
    #[serde(rename = "handoff.create")]
    HandoffCreate,
    #[serde(rename = "handoff.claim")]
    HandoffClaim,
    #[serde(rename = "task.create")]
    TaskCreate,
    #[serde(rename = "task.transition")]
    TaskTransition,
    #[serde(rename = "task.checkpoint")]
    TaskCheckpoint,
    #[serde(rename = "persona.manage")]
    PersonaManage,
    #[serde(rename = "github.read")]
    GithubRead,
    #[serde(rename = "github.write")]
    GithubWrite,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContextRecord => "context.record",
            Self::DecisionRecord => "decision.record",
            Self::HandoffCreate => "handoff.create",
            Self::HandoffClaim => "handoff.claim",
            Self::TaskCreate => "task.create",
            Self::TaskTransition => "task.transition",
            Self::TaskCheckpoint => "task.checkpoint",
            Self::PersonaManage => "persona.manage",
            Self::GithubRead => "github.read",
            Self::GithubWrite => "github.write",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum PersonaStatus {
    Active,
    Disabled,
    Archived,
}

impl PersonaStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PersonaInput {
    pub name: String,
    pub role: String,
    pub description: String,
    #[serde(default)]
    pub responsibilities: Vec<String>,
    #[serde(default)]
    pub prohibitions: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    pub project_id: Option<String>,
    pub preferred_runtime: Option<String>,
    /// Self-reported label only; it is not authenticated against a GitHub account.
    pub github_identity: Option<String>,
    pub context_retrieval: Option<String>,
    #[serde(default)]
    pub review_required: bool,
    #[serde(default = "default_concurrency")]
    pub max_concurrency: u32,
    pub budget_units: Option<u64>,
}

fn default_concurrency() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PersonaUpdate {
    pub persona_id: String,
    pub expected_revision: u64,
    pub input: PersonaInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PersonaStatusChange {
    pub persona_id: String,
    pub status: PersonaStatus,
    pub expected_revision: u64,
}

/// A runtime assertion of which persona policy authorizes a protected operation.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PersonaClaim {
    pub persona_id: String,
    pub expected_revision: u64,
}
