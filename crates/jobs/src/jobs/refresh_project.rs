use anyhow::Result;
use apalis::prelude::*;
use decomp_dev_github::refresh_project;
use serde::{Deserialize, Serialize};

use crate::JobContext;

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

/// Process a project refresh job.
///
/// This calls the existing refresh_project function to fetch workflow runs,
/// download artifacts, and insert reports.
pub async fn process_refresh_project_job(
    job: RefreshProjectJob,
    ctx: Data<JobContext>,
) -> Result<()> {
    tracing::info!(
        "Processing refresh project job: repo={} full_refresh={}",
        job.repository_id,
        job.full_refresh
    );

    match refresh_project(&ctx.github, &ctx.db, job.repository_id, None, job.full_refresh).await {
        Ok(count) => {
            tracing::info!("Refreshed project {} with {} new reports", job.repository_id, count);
            Ok(())
        }
        Err(e) => {
            tracing::error!("Failed to refresh project {}: {:?}", job.repository_id, e);
            Err(e)
        }
    }
}
