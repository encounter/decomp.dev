use serde::{Deserialize, Serialize};

/// Job to refresh a project's reports from GitHub Actions.
///
/// This job fetches workflow runs from GitHub, downloads artifacts,
/// parses reports, and inserts them into the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshProjectJob {
    /// The repository ID to refresh.
    pub repository_id: u64,
    /// Whether to do a full refresh (fetch all runs) or partial (stop at known commit).
    pub full_refresh: bool,
}
