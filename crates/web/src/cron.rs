use anyhow::Result;
use apalis::prelude::TaskSink;
use decomp_dev_jobs::RefreshProjectJob;
use tokio_cron_scheduler::{Job, JobScheduler};
use tower_sessions::ExpiredDeletion;
use tracing::log;

use crate::AppState;

pub type Scheduler = JobScheduler;

pub async fn create(
    state: AppState,
    session_store: impl ExpiredDeletion + Clone,
) -> Result<Scheduler> {
    let sched = JobScheduler::new().await?;

    // Every 5 minutes: Queue partial refresh jobs for projects without active installations
    {
        let state = state.clone();
        sched
            .add(Job::new_async("every 5 minutes", move |_uuid, _l| {
                let state = state.clone();
                Box::pin(async move {
                    if let Err(e) = queue_refresh_jobs(&state, false).await {
                        log::error!("Failed to queue refresh jobs: {:?}", e);
                    }
                })
            })?)
            .await?;
    }

    // Every 12 hours: Queue full refresh jobs for all projects
    {
        let state = state.clone();
        sched
            .add(Job::new_async("every 12 hours", move |_uuid, _l| {
                let state = state.clone();
                Box::pin(async move {
                    if let Err(e) = queue_refresh_jobs(&state, true).await {
                        log::error!("Failed to queue full refresh jobs: {:?}", e);
                    }
                })
            })?)
            .await?;
    }

    // At midnight: Cleanup report units and images
    {
        sched
            .add(Job::new_async("at midnight", move |_uuid, _l| {
                let state = state.clone();
                Box::pin(async move {
                    if let Err(e) = state.db.cleanup_report_units().await {
                        log::error!("Failed to clean up report units: {:?}", e);
                    }
                    if let Err(e) = state.db.cleanup_images().await {
                        log::error!("Failed to clean up images: {:?}", e);
                    }
                })
            })?)
            .await?;
    }

    // Every 1 minute: Delete expired sessions
    {
        sched
            .add(Job::new_async("every 1 minute", move |_uuid, _l| {
                let session_store = session_store.clone();
                Box::pin(async move {
                    if let Err(e) = session_store.delete_expired().await {
                        log::error!("Failed to delete expired sessions: {:?}", e);
                    }
                })
            })?)
            .await?;
    }

    sched.start().await?;
    Ok(sched)
}

/// Queue refresh jobs for all enabled projects.
async fn queue_refresh_jobs(state: &AppState, full_refresh: bool) -> Result<()> {
    let mut queued = 0;
    for project_info in state.db.get_projects().await? {
        if !project_info.project.enabled {
            log::debug!(
                "Skipping disabled project {}/{}",
                project_info.project.owner,
                project_info.project.repo
            );
            continue;
        }
        if !full_refresh {
            // Skip projects with active app installations (they get updates via webhooks)
            if let Some(installations) = &state.github.installations {
                let installations = installations.lock().await;
                if installations.repo_to_installation.contains_key(&project_info.project.id) {
                    continue;
                }
            }
        }

        let job = RefreshProjectJob { repository_id: project_info.project.id, full_refresh };

        let mut storage = state.jobs.refresh_project();
        if let Err(e) = storage.push(job).await {
            log::error!(
                "Failed to queue refresh job for {}/{}: {:?}",
                project_info.project.owner,
                project_info.project.repo,
                e
            );
        } else {
            queued += 1;
        }
    }

    if queued > 0 {
        log::info!("Queued {} refresh jobs (full_refresh={})", queued, full_refresh);
    }

    Ok(())
}
