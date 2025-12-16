use serde::{Deserialize, Serialize};

/// Job to process a completed GitHub Actions workflow run.
///
/// This job downloads artifacts from the workflow run, parses report files,
/// inserts them into the database (for push events), and posts/updates
/// PR comments (for pull request events).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessWorkflowRunJob {
    /// The repository ID (used to look up the project).
    pub repository_id: u64,
    /// The workflow run ID to process.
    pub run_id: u64,
    /// The SHA of the commit that triggered the workflow.
    pub head_sha: String,
    /// The branch that triggered the workflow.
    pub head_branch: String,
    /// The event that triggered the workflow ("push", "pull_request", etc.).
    pub event: String,
    /// PR numbers associated with this workflow run (empty for push events).
    pub pull_request_numbers: Vec<u64>,
    /// The GitHub App installation ID, if this was triggered via an installation.
    pub installation_id: Option<u64>,
}
