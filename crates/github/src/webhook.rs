use std::fmt::Display;

use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{FromRef, FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use decomp_dev_core::{AppError, config::GitHubConfig};
use decomp_dev_db::Database;
use hmac::{Hmac, Mac};
use octocrab::{
    Octocrab,
    models::{
        pulls::PullRequest,
        webhook_events::{
            EventInstallation, WebhookEvent, WebhookEventPayload,
            payload::{
                InstallationWebhookEventAction, PullRequestWebhookEventAction,
                WorkflowRunWebhookEventAction,
            },
        },
        workflows::{Run, WorkFlow},
    },
};
use sha2::Sha256;

use crate::{
    GitHub, ProcessWorkflowRunResult,
    changes::{generate_changes, generate_comment},
    commit_from_head_commit, process_workflow_run,
};

#[derive(Clone)]
pub struct WebhookState {
    pub config: GitHubConfig,
    pub db: Database,
    pub github: GitHub,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RunWithPullRequests {
    #[serde(flatten)]
    pub inner: Run,
    pub pull_requests: Vec<PullRequest>,
}

pub async fn webhook(GitHubEvent { event, state }: GitHubEvent) -> Result<Response, AppError> {
    let Some(installations) = &state.github.installations else {
        tracing::warn!("Received webhook event {:?} with no GitHub app config", event.kind);
        return Ok((StatusCode::OK, "No app config").into_response());
    };
    let mut owner = None;
    if let Some(repository) = event.repository {
        owner = repository.owner.map(|o| o.login.clone());
        if let Some(full_name) = repository.full_name {
            tracing::warn!("Received webhook event {:?} from repository {}", event.kind, full_name);
        } else {
            tracing::warn!(
                "Received webhook event {:?} from repository ID {}",
                event.kind,
                repository.id.0
            );
        }
    } else if let Some(organization) = event.organization {
        owner = Some(organization.login.clone());
        tracing::warn!("Received webhook event {:?} from org {}", event.kind, organization.login);
    } else if let Some(sender) = event.sender {
        tracing::warn!("Received webhook event {:?} from @{}", event.kind, sender.login);
    } else {
        tracing::warn!("Received webhook event {:?} from unknown source", event.kind);
    }
    let installation_id = match event.installation {
        Some(EventInstallation::Full(installation)) => {
            owner = Some(installation.account.login.clone());
            Some(installation.id)
        }
        Some(EventInstallation::Minimal(installation)) => Some(installation.id),
        None => None,
    };
    let client = if let Some(installation_id) = installation_id {
        let mut installations = installations.lock().await;
        installations.client_for_installation(installation_id).await?
    } else {
        state.github.client.clone()
    };
    match event.specific {
        WebhookEventPayload::WorkflowRun(inner) => {
            if inner.action == WorkflowRunWebhookEventAction::Completed {
                let Some(workflow) = inner.workflow else {
                    tracing::error!("Received workflow_run event with no workflow");
                    return Ok((StatusCode::OK, "No workflow run").into_response());
                };
                let workflow: WorkFlow = match serde_json::from_value(workflow) {
                    Ok(workflow) => workflow,
                    Err(e) => {
                        tracing::error!("Received workflow_run event with invalid workflow: {e}");
                        return Ok((StatusCode::OK, "Invalid workflow").into_response());
                    }
                };
                let workflow_run: RunWithPullRequests =
                    match serde_json::from_value(inner.workflow_run) {
                        Ok(workflow_run) => workflow_run,
                        Err(e) => {
                            tracing::error!(
                                "Received workflow_run event with invalid workflow_run: {e}"
                            );
                            return Ok((StatusCode::OK, "Invalid workflow run").into_response());
                        }
                    };
                if let Err(e) =
                    handle_workflow_run_completed(&state, client, workflow, workflow_run).await
                {
                    tracing::error!("Error handling workflow_run event: {e}");
                    return Ok((StatusCode::OK, "Internal error").into_response());
                }
            }
        }
        WebhookEventPayload::PullRequest(inner) => {
            if inner.action == PullRequestWebhookEventAction::Opened
                || inner.action == PullRequestWebhookEventAction::Synchronize
            {
                if let Err(e) = handle_pull_request_update(&state, client, inner.pull_request).await
                {
                    tracing::error!("Error handling pull_request event: {e}");
                    return Ok((StatusCode::OK, "Internal error").into_response());
                }
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

async fn handle_workflow_run_completed(
    state: &WebhookState,
    client: Octocrab,
    _workflow: WorkFlow,
    workflow_run: RunWithPullRequests,
) -> Result<()> {
    let RunWithPullRequests { inner: workflow_run, mut pull_requests } = workflow_run;
    let repository_id = workflow_run.repository.id.into_inner();
    let Some(project_info) = state.db.get_project_info_by_id(repository_id, None).await? else {
        tracing::warn!("No project found for repository ID {}", repository_id);
        return Ok(());
    };
    let repository =
        client.repos_by_id(repository_id).get().await.context("Failed to fetch repository")?;
    let ProcessWorkflowRunResult { artifacts } =
        process_workflow_run(&client, &project_info.project, workflow_run.id).await?;
    tracing::debug!(
        "Processed workflow run {} ({}) (artifacts {})",
        workflow_run.id,
        workflow_run.head_sha,
        artifacts.len()
    );
    if artifacts.is_empty() {
        return Ok(());
    }

    let commit = commit_from_head_commit(&workflow_run.head_commit);
    if workflow_run.event == "push"
        && match &repository.default_branch {
            Some(default_branch) => default_branch == &workflow_run.head_branch,
            None => matches!(workflow_run.head_branch.as_str(), "master" | "main"),
        }
    {
        // Insert reports into the database
        for artifact in artifacts {
            let start = std::time::Instant::now();
            state
                .db
                .insert_report(&project_info.project, &commit, &artifact.version, *artifact.report)
                .await?;
            let duration = start.elapsed();
            tracing::info!(
                "Inserted report {} ({}) in {}ms",
                artifact.version,
                commit.sha,
                duration.as_millis()
            );
        }
    } else if workflow_run.event == "pull_request" {
        if !project_info.project.enable_pr_comments {
            return Ok(());
        }
        // Actions pull_request builds always merge with the latest commit on the base branch.
        // We can't use the base commit from the workflow run or pull request APIs, those are
        // both lies. For simplicity, we'll just always compare against the latest commit that
        // we have stored.
        let Some(base_commit) = project_info.commit else {
            tracing::warn!("No base commit found for repository ID {}", repository_id);
            return Ok(());
        };
        // Fetch any associated pull requests
        if pull_requests.is_empty() {
            let head = if let Some(head_owner) =
                workflow_run.head_repository.as_ref().and_then(|r| r.owner.as_ref())
            {
                format!("{}:{}", head_owner.login, workflow_run.head_branch)
            } else {
                workflow_run.head_branch.clone()
            };
            pull_requests = client
                .all_pages(
                    client
                        .pulls(&project_info.project.owner, &project_info.project.repo)
                        .list()
                        .head(&head)
                        .send()
                        .await?,
                )
                .await?;
            tracing::info!("Found {} pull requests for {}", pull_requests.len(), head);
        }
        for pull_request in pull_requests {
            if pull_request.head.sha != workflow_run.head_sha {
                tracing::warn!(
                    "Pull request {} head SHA {} does not match workflow run head SHA {}",
                    pull_request.id,
                    pull_request.head.sha,
                    workflow_run.head_sha
                );
                continue;
            }
            if repository.default_branch.as_ref().is_none_or(|b| pull_request.base.ref_field != *b)
            {
                tracing::warn!(
                    "Pull request {} base branch {} does not match default branch {}",
                    pull_request.id,
                    pull_request.base.ref_field,
                    repository.default_branch.as_deref().unwrap_or("[unknown]")
                );
                continue;
            }
            tracing::info!("Processing pull request {}", pull_request.id);
            let issues = client.issues_by_id(repository_id);
            // Only fetch first page for now
            let existing_comments = issues.list_comments(pull_request.number).send().await?;
            for artifact in &artifacts {
                let Some(cached_report) = state
                    .db
                    .get_report(
                        &project_info.project.owner,
                        &project_info.project.repo,
                        &base_commit.sha,
                        &artifact.version,
                    )
                    .await?
                else {
                    tracing::warn!(
                        "No report found for pull request {} (base {}) and version {}",
                        pull_request.id,
                        pull_request.base.sha,
                        artifact.version
                    );
                    continue;
                };
                let report_file = state.db.upgrade_report(&cached_report).await?;
                let report = report_file.report.flatten();
                let changes = generate_changes(&report, &artifact.report)?;
                let comment_text = generate_comment(
                    &report,
                    &artifact.report,
                    Some(&report_file.version),
                    Some(&report_file.commit),
                    Some(&commit),
                    changes,
                );
                let existing_comment = existing_comments
                    .items
                    .iter()
                    .find(|comment| {
                        // TODO check author ID
                        comment.body.as_ref().is_some_and(|body| {
                            body.contains(format!("Report for {}", artifact.version).as_str())
                        })
                    })
                    .map(|comment| comment.id);
                if let Some(existing_comment) = existing_comment {
                    issues
                        .update_comment(existing_comment, comment_text)
                        .await
                        .context("Failed to update existing comment")?;
                } else {
                    issues
                        .create_comment(pull_request.number, comment_text)
                        .await
                        .context("Failed to create comment")?;
                }
            }
        }
    }

    Ok(())
}

async fn handle_pull_request_update(
    _state: &WebhookState,
    _client: Octocrab,
    _pull_request: PullRequest,
) -> Result<()> {
    // Handle the pull request update event here
    // tracing::info!("Pull request updated: {:?}", pull_request);
    Ok(())
}

/// Verify and extract GitHub Event Payload.
#[derive(Clone)]
#[must_use]
pub struct GitHubEvent {
    pub event: WebhookEvent,
    pub state: WebhookState,
}

impl<S> FromRequest<S> for GitHubEvent
where
    WebhookState: FromRef<S>,
    S: Send + Sync + Clone,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        fn err(m: impl Display) -> Response {
            tracing::error!("{m}");
            (StatusCode::BAD_REQUEST, m.to_string()).into_response()
        }
        let event = req
            .headers()
            .get("X-GitHub-Event")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| err("X-GitHub-Event header missing"))?
            .to_string();
        let inner = WebhookState::from_ref(state);
        let body = if let Some(app_config) = &inner.config.app {
            let signature_sha256 = req
                .headers()
                .get("X-Hub-Signature-256")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| err("X-Hub-Signature-256 missing"))?
                .strip_prefix("sha256=")
                .ok_or_else(|| err("X-Hub-Signature-256 sha256= prefix missing"))?;
            let signature =
                hex::decode(signature_sha256).map_err(|_| err("X-Hub-Signature-256 malformed"))?;
            let body =
                Bytes::from_request(req, state).await.map_err(|_| err("error reading body"))?;
            let mut mac = Hmac::<Sha256>::new_from_slice(app_config.webhook_secret.as_bytes())
                .expect("HMAC can take key of any size");
            mac.update(&body);
            if mac.verify_slice(&signature).is_err() {
                return Err(err("signature mismatch"));
            }
            body
        } else {
            Bytes::from_request(req, state).await.map_err(|_| err("error reading body"))?
        };
        let value = WebhookEvent::try_from_header_and_body(&event, &body)
            .map_err(|_| err("error parsing body"))?;
        Ok(GitHubEvent { event: value, state: inner })
    }
}
