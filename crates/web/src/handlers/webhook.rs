use anyhow::Context;
use apalis::prelude::TaskSink;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use decomp_dev_core::AppError;
use decomp_dev_github::webhook::{GitHubEvent, RunWithPullRequests};
use decomp_dev_jobs::ProcessWorkflowRunJob;
use octocrab::models::{
    webhook_events::{
        EventInstallation, WebhookEventPayload,
        payload::{InstallationWebhookEventAction, WorkflowRunWebhookEventAction},
    },
    workflows::WorkFlow,
};

use crate::AppState;

/// Webhook handler that enqueues jobs for processing instead of handling synchronously.
pub async fn webhook(
    State(state): State<AppState>,
    event: GitHubEvent,
) -> Result<Response, AppError> {
    let Some(_installations) = &event.state.github.installations else {
        tracing::warn!("Received webhook event {:?} with no GitHub app config", event.event.kind);
        return Ok((StatusCode::OK, "No app config").into_response());
    };

    // Log the event source
    let mut owner = None;
    if let Some(repository) = &event.event.repository {
        owner = repository.owner.as_ref().map(|o| o.login.clone());
        if let Some(full_name) = &repository.full_name {
            tracing::info!(
                "Received webhook event {:?} from repository {}",
                event.event.kind,
                full_name
            );
        } else {
            tracing::info!(
                "Received webhook event {:?} from repository ID {}",
                event.event.kind,
                repository.id.0
            );
        }
    } else if let Some(organization) = &event.event.organization {
        owner = Some(organization.login.clone());
        tracing::info!(
            "Received webhook event {:?} from org {}",
            event.event.kind,
            organization.login
        );
    } else if let Some(sender) = &event.event.sender {
        tracing::info!("Received webhook event {:?} from @{}", event.event.kind, sender.login);
    } else {
        tracing::info!("Received webhook event {:?} from unknown source", event.event.kind);
    }

    let installation_id = match &event.event.installation {
        Some(EventInstallation::Full(installation)) => Some(installation.id),
        Some(EventInstallation::Minimal(installation)) => Some(installation.id),
        None => None,
    };

    match &event.event.specific {
        WebhookEventPayload::WorkflowRun(inner) => {
            if inner.action == WorkflowRunWebhookEventAction::Completed {
                let Some(workflow) = &inner.workflow else {
                    tracing::error!("Received workflow_run event with no workflow");
                    return Ok((StatusCode::OK, "No workflow").into_response());
                };
                let _workflow: WorkFlow = match serde_json::from_value(workflow.clone()) {
                    Ok(workflow) => workflow,
                    Err(e) => {
                        tracing::error!("Received workflow_run event with invalid workflow: {e}");
                        return Ok((StatusCode::OK, "Invalid workflow").into_response());
                    }
                };
                let workflow_run: RunWithPullRequests =
                    match serde_json::from_value(inner.workflow_run.clone()) {
                        Ok(workflow_run) => workflow_run,
                        Err(e) => {
                            tracing::error!(
                                "Received workflow_run event with invalid workflow_run: {e}"
                            );
                            return Ok((StatusCode::OK, "Invalid workflow run").into_response());
                        }
                    };

                // Create the job
                let job = ProcessWorkflowRunJob {
                    repository_id: workflow_run.inner.repository.id.into_inner(),
                    run_id: workflow_run.inner.id.into_inner(),
                    head_sha: workflow_run.inner.head_sha.clone(),
                    head_branch: workflow_run.inner.head_branch.clone(),
                    event: workflow_run.inner.event.clone(),
                    pull_request_numbers: workflow_run
                        .pull_requests
                        .iter()
                        .map(|pr| pr.number)
                        .collect(),
                    installation_id: installation_id.map(|id| id.into_inner()),
                };

                // Enqueue the job
                let mut storage = state.jobs.workflow_run();
                storage.push(job).await.context("Failed to enqueue workflow run job")?;

                tracing::info!("Enqueued workflow run {} for processing", workflow_run.inner.id);
            }
        }
        WebhookEventPayload::PullRequest(_inner) => {
            // Pull request events are handled as part of workflow_run events
        }
        WebhookEventPayload::Installation(inner) => {
            tracing::info!(
                "Installation {:?} for {}",
                inner.action,
                owner.as_deref().unwrap_or("[unknown]")
            );
            if let Some(installation_id) = installation_id {
                let installations = event.state.github.installations.as_ref().unwrap();
                match inner.action {
                    InstallationWebhookEventAction::Created => {
                        // Installation client is already created by the extractor
                    }
                    InstallationWebhookEventAction::Deleted => {
                        // Remove the installation client
                        let mut installations = installations.lock().await;
                        installations.repo_to_installation.retain(|_, v| *v != installation_id);
                        installations.clients.remove(&installation_id);
                    }
                    _ => {}
                }
            } else {
                tracing::warn!("Received installation event with no installation ID");
            }
        }
        WebhookEventPayload::InstallationRepositories(inner) => {
            tracing::info!(
                "Installation {:?} for {} repositories changed",
                inner.action,
                owner.as_deref().unwrap_or("[unknown]")
            );
            if let Some(installation_id) = installation_id {
                let installations = event.state.github.installations.as_ref().unwrap();
                let mut installations = installations.lock().await;
                for repository in &inner.repositories_added {
                    tracing::info!("Added repository {}", repository.full_name);
                    installations
                        .repo_to_installation
                        .insert(repository.id.into_inner(), installation_id);
                }
                if !inner.repositories_removed.is_empty() {
                    for repository in &inner.repositories_removed {
                        tracing::info!("Removed repository {}", repository.full_name);
                    }
                    installations.repo_to_installation.retain(|repo, id| {
                        if *id != installation_id {
                            return true;
                        }
                        inner.repositories_removed.iter().any(|r| r.id.into_inner() == *repo)
                    });
                }
            } else {
                tracing::warn!("Received installation_repositories event with no installation ID");
            }
        }
        _ => {}
    }

    Ok((StatusCode::OK, "Event queued").into_response())
}
