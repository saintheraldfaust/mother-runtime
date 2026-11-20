//! Submitted continuity data never grants permissions or verifies claimed work.
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionInput {
    pub project_id: String,
    pub title: String,
    pub decision: String,
    pub rationale: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
    pub source: String,
    pub supersedes: Option<String>,
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HandoffInput {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub objective: String,
    pub source: String,
    pub originating_host: Option<String>,
    pub originating_session: Option<String>,
    #[serde(default)]
    pub work_completed: Vec<String>,
    #[serde(default)]
    pub work_remaining: Vec<String>,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub git_references: Vec<String>,
    #[serde(default)]
    pub test_results: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub unresolved_questions: Vec<String>,
    #[serde(default)]
    pub hypotheses: Vec<String>,
    #[serde(default)]
    pub provenance_references: Vec<String>,
    pub next_action: String,
    /// Routing label, not an authenticated principal or an authorization grant.
    pub expected_recipient: Option<String>,
    pub expected_revision: Option<u64>,
}
