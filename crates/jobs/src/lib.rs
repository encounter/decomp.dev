mod handlers;
mod jobs;

use std::time::Duration;

use anyhow::Result;
use apalis::prelude::*;
use apalis_sqlite::{CompactType, SqliteStorage, fetcher::SqliteFetcher};
use decomp_dev_core::config::Config;
use decomp_dev_db::Database;
use decomp_dev_github::GitHub;
pub use handlers::{process_refresh_project_job, process_workflow_run_job};
pub use jobs::{ProcessWorkflowRunJob, RefreshProjectJob};
use sqlx::sqlite::SqlitePool;

/// Shared context available to all job handlers.
#[derive(Clone)]
pub struct JobContext {
    pub config: Config,
    pub db: Database,
    pub github: GitHub,
}

/// Type alias for the default codec used by SqliteStorage.
type DefaultCodec = apalis::prelude::json::JsonCodec<CompactType>;

/// Type alias for workflow run storage.
pub type WorkflowRunStorage = SqliteStorage<ProcessWorkflowRunJob, DefaultCodec, SqliteFetcher>;

/// Type alias for refresh project storage.
pub type RefreshProjectStorage = SqliteStorage<RefreshProjectJob, DefaultCodec, SqliteFetcher>;

/// Storage handles for pushing jobs from request handlers.
#[derive(Clone)]
pub struct JobStorage {
    workflow_run: WorkflowRunStorage,
    refresh_project: RefreshProjectStorage,
}

impl JobStorage {
    /// Set up job storage tables and create storage instances.
    pub async fn setup(pool: &SqlitePool) -> Result<Self> {
        SqliteStorage::setup(pool).await?;

        Ok(Self {
            workflow_run: SqliteStorage::new(pool),
            refresh_project: SqliteStorage::new(pool),
        })
    }

    /// Get a clone of the workflow run storage for pushing jobs.
    pub fn workflow_run(&self) -> WorkflowRunStorage { self.workflow_run.clone() }

    /// Get a clone of the refresh project storage for pushing jobs.
    pub fn refresh_project(&self) -> RefreshProjectStorage { self.refresh_project.clone() }
}

/// Configuration for job workers.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Maximum concurrent workflow run jobs.
    pub workflow_run_concurrency: usize,
    /// Maximum concurrent refresh project jobs.
    pub refresh_project_concurrency: usize,
    /// Number of retry attempts for failed jobs.
    pub retry_attempts: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { workflow_run_concurrency: 5, refresh_project_concurrency: 3, retry_attempts: 3 }
    }
}

/// Create the job monitor with all workers.
pub fn create_monitor(storage: JobStorage, context: JobContext, config: WorkerConfig) -> Monitor {
    let ctx1 = context.clone();
    let ctx2 = context;
    let config1 = config.clone();
    let config2 = config;

    Monitor::new()
        .register(move |_| {
            WorkerBuilder::new("workflow-run-worker")
                .backend(storage.workflow_run.clone())
                .concurrency(config1.workflow_run_concurrency)
                .data(ctx1.clone())
                .build(process_workflow_run_job)
        })
        .register(move |_| {
            WorkerBuilder::new("refresh-project-worker")
                .backend(storage.refresh_project.clone())
                .concurrency(config2.refresh_project_concurrency)
                .data(ctx2.clone())
                .build(process_refresh_project_job)
        })
        .shutdown_timeout(Duration::from_secs(30))
}
