use anyhow::Context;
use apalis::prelude::TaskSink;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use decomp_dev_core::AppError;
use decomp_dev_github::webhook::GitHubEvent;
use decomp_dev_jobs::ProcessWorkflowRunJob;
use octocrab::models::{
    webhook_events::{
        EventInstallation, WebhookEventPayload,
        payload::{InstallationWebhookEventAction, WorkflowRunWebhookEventAction},
    },
    workflows::Run,
};

use crate::AppState;

/// Webhook handler that enqueues jobs for processing instead of handling synchronously.
pub async fn webhook(
    State(state): State<AppState>,
    GitHubEvent { event }: GitHubEvent,
) -> Result<Response, AppError> {
    let Some(installations) = &state.github.installations else {
        tracing::warn!("Received webhook event {:?} with no GitHub app config", event.kind);
        return Ok((StatusCode::OK, "No app config").into_response());
    };

    // Log the event source
    let mut owner = None;
    if let Some(repository) = event.repository {
        owner = repository.owner.map(|o| o.login.clone());
        if let Some(full_name) = repository.full_name {
            tracing::info!("Received webhook event {:?} from repository {}", event.kind, full_name);
        } else {
            tracing::info!(
                "Received webhook event {:?} from repository ID {}",
                event.kind,
                repository.id.0
            );
        }
    } else if let Some(organization) = event.organization {
        owner = Some(organization.login.clone());
        tracing::info!("Received webhook event {:?} from org {}", event.kind, organization.login);
    } else if let Some(sender) = event.sender {
        tracing::info!("Received webhook event {:?} from @{}", event.kind, sender.login);
    } else {
        tracing::info!("Received webhook event {:?} from unknown source", event.kind);
    }

    let installation_id = match event.installation {
        Some(EventInstallation::Full(installation)) => {
            owner = Some(installation.account.login.clone());
            Some(installation.id)
        }
        Some(EventInstallation::Minimal(installation)) => Some(installation.id),
        None => None,
    };

    match &event.specific {
        WebhookEventPayload::WorkflowRun(inner) => {
            if inner.action == WorkflowRunWebhookEventAction::Completed {
                let workflow_run: Run = match serde_json::from_value(inner.workflow_run.clone()) {
                    Ok(workflow_run) => workflow_run,
                    Err(e) => {
                        tracing::error!(
                            "Received workflow_run event with invalid workflow_run: {e}"
                        );
                        return Ok((StatusCode::OK, "Invalid workflow run").into_response());
                    }
                };

                // Enqueue the job
                let mut storage = state.jobs.workflow_run();
                storage
                    .push(ProcessWorkflowRunJob::from(&workflow_run))
                    .await
                    .context("Failed to enqueue workflow run job")?;

                tracing::info!("Enqueued workflow run {} for processing", workflow_run.id);
            }
        }
        WebhookEventPayload::Installation(inner) => {
            tracing::info!(
                "Installation {:?} for {}",
                inner.action,
                owner.as_deref().unwrap_or("[unknown]")
            );
            match inner.action {
                InstallationWebhookEventAction::Created => {
                    // Installation client is already created
                }
                InstallationWebhookEventAction::Deleted => {
                    // Remove the installation client
                    let mut installations = installations.lock().await;
                    if let Some(installation_id) = installation_id {
                        installations.repo_to_installation.retain(|_, v| *v != installation_id);
                        installations.clients.remove(&installation_id);
                    } else {
                        tracing::warn!(
                            "Received installation deleted event with no installation ID"
                        );
                    }
                }
                _ => {}
            }
        }
        WebhookEventPayload::InstallationRepositories(inner) => {
            tracing::info!(
                "Installation {:?} for {} repositories changed",
                inner.action,
                owner.as_deref().unwrap_or("[unknown]")
            );
            let Some(installation_id) = installation_id else {
                tracing::warn!("Received installation_repositories event with no installation ID");
                return Ok((StatusCode::OK, "No installation ID").into_response());
            };
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
        }
        _ => {}
    }

    Ok((StatusCode::OK, "Event processed").into_response())
}
