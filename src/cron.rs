use anyhow::Result;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::log;

use crate::{github, AppState};

pub type Scheduler = JobScheduler;

pub async fn create(state: AppState) -> Result<Scheduler> {
    let sched = JobScheduler::new().await?;
    sched
        .add(Job::new_async("0 0/5 * * * *", move |_uuid, _l| {
            let mut state = state.clone();
            Box::pin(async move {
                refresh_projects(&mut state).await.expect("Failed to refresh projects");
            })
        })?)
        .await?;
    sched.start().await?;
    Ok(sched)
}

pub async fn refresh_projects(state: &mut AppState) -> Result<()> {
    for project_info in state.db.get_projects().await? {
        if let Err(e) =
            github::run(state, &project_info.project.owner, &project_info.project.repo, 0).await
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
