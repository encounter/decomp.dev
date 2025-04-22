use anyhow::Result;
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
    {
        let state = state.clone();
        sched
            .add(Job::new_async("0 0/5 * * * *", move |_uuid, _l| {
                let state = state.clone();
                Box::pin(async move {
                    refresh_projects(&state).await.expect("Failed to refresh projects");
                })
            })?)
            .await?;
    }
    {
        sched
            .add(Job::new_async("0 0 0/24 * * *", move |_uuid, _l| {
                let state = state.clone();
                Box::pin(async move {
                    state.db.cleanup_report_units().await.expect("Failed to clean up report units");
                })
            })?)
            .await?;
    }
    {
        sched
            .add(Job::new_async("0 0/1 * * * *", move |_uuid, _l| {
                let session_store = session_store.clone();
                Box::pin(async move {
                    session_store
                        .delete_expired()
                        .await
                        .expect("Failed to delete expired sessions");
                })
            })?)
            .await?;
    }
    sched.start().await?;
    Ok(sched)
}

pub async fn refresh_projects(state: &AppState) -> Result<()> {
    for project_info in state.db.get_projects().await? {
        // Skip projects with active app installations
        if let Some(installations) = &state.github.installations {
            let installations = installations.lock().await;
            if installations.owner_to_installation.contains_key(&project_info.project.owner) {
                continue;
            }
        }
        if let Err(e) =
            decomp_dev_github::run(&state.github, &state.db, project_info.project.id, 0).await
        {
            log::error!(
                "Failed to refresh {}/{}: {:?}",
                project_info.project.owner,
                project_info.project.repo,
                e
            );
        }
    }
    Ok(())
}
