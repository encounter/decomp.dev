use std::{cmp::Ordering, fmt::Display, mem::take};

use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{FromRef, FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use objdiff_core::bindings::report::{
    ChangeItem, ChangeItemInfo, ChangeUnit, Changes, Report, ReportItem, ReportUnit,
};
use octocrab::{
    models::{
        pulls::PullRequest,
        webhook_events::{
            payload::{
                InstallationWebhookEventAction, PullRequestWebhookEventAction,
                WorkflowRunWebhookEventAction,
            },
            EventInstallation, WebhookEvent, WebhookEventPayload,
        },
        workflows::{Run, WorkFlow},
    },
    Octocrab,
};
use sha2::Sha256;

use crate::{
    github::{process_workflow_run, ProcessArtifactResult, ProcessWorkflowRunResult},
    handlers::AppError,
    models::{Commit, FullReportFile},
    AppState,
};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RunWithPullRequests {
    #[serde(flatten)]
    pub inner: Run,
    pub pull_requests: Vec<PullRequest>,
}

pub async fn webhook(
    GitHubEvent { event, state }: GitHubEvent<AppState>,
) -> Result<Response, AppError> {
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
        installations.client_for_installation(installation_id, owner.as_deref())?
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
                    if let Some(owner) = &owner {
                        installations.owner_to_installation.remove(owner);
                    } else {
                        tracing::warn!("Received installation deleted event with no owner");
                    }
                    if let Some(installation_id) = installation_id {
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
        _ => {}
    }
    Ok((StatusCode::OK, "Event processed").into_response())
}

async fn handle_workflow_run_completed(
    state: &AppState,
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

    let commit = Commit::from(&workflow_run.head_commit);
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
                        &pull_request.base.sha,
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
                let changes = changes(&report, &artifact.report)?;
                let comment_text = generate_comment(&report_file, artifact, &commit, changes);
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
    _state: &AppState,
    _client: Octocrab,
    _pull_request: PullRequest,
) -> Result<()> {
    // Handle the pull request update event here
    // tracing::info!("Pull request updated: {:?}", pull_request);
    Ok(())
}

/// Verify and extract GitHub Event Payload.
#[derive(Debug, Clone)]
#[must_use]
pub struct GitHubEvent<S> {
    pub event: WebhookEvent,
    pub state: S,
}

impl<S> FromRequest<S> for GitHubEvent<S>
where
    AppState: FromRef<S>,
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
        let app_state = AppState::from_ref(state);
        let body = if let Some(app_config) = &app_state.config.github.app {
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
        Ok(GitHubEvent { event: value, state: state.clone() })
    }
}

fn changes(previous: &Report, current: &Report) -> Result<Changes> {
    let mut changes = Changes { from: previous.measures, to: current.measures, units: vec![] };
    for prev_unit in &previous.units {
        let curr_unit = current.units.iter().find(|u| u.name == prev_unit.name);
        let sections = process_items(prev_unit, curr_unit, |u| &u.sections);
        let functions = process_items(prev_unit, curr_unit, |u| &u.functions);

        let prev_measures = prev_unit.measures;
        let curr_measures = curr_unit.and_then(|u| u.measures);
        if !functions.is_empty() || prev_measures != curr_measures {
            changes.units.push(ChangeUnit {
                name: prev_unit.name.clone(),
                from: prev_measures,
                to: curr_measures,
                sections,
                functions,
                metadata: curr_unit
                    .as_ref()
                    .and_then(|u| u.metadata.clone())
                    .or_else(|| prev_unit.metadata.clone()),
            });
        }
    }
    for curr_unit in &current.units {
        if !previous.units.iter().any(|u| u.name == curr_unit.name) {
            changes.units.push(ChangeUnit {
                name: curr_unit.name.clone(),
                from: None,
                to: curr_unit.measures,
                sections: process_new_items(&curr_unit.sections),
                functions: process_new_items(&curr_unit.functions),
                metadata: curr_unit.metadata.clone(),
            });
        }
    }
    Ok(changes)
}

fn process_items<F: Fn(&ReportUnit) -> &Vec<ReportItem>>(
    prev_unit: &ReportUnit,
    curr_unit: Option<&ReportUnit>,
    getter: F,
) -> Vec<ChangeItem> {
    let prev_items = getter(prev_unit);
    let mut items = vec![];
    if let Some(curr_unit) = curr_unit {
        let curr_items = getter(curr_unit);
        for prev_func in prev_items {
            let prev_func_info = ChangeItemInfo::from(prev_func);
            let prev_func_address = prev_func.metadata.as_ref().and_then(|m| m.virtual_address);
            let curr_func = curr_items.iter().find(|f| {
                f.name == prev_func.name
                    || prev_func_address.is_some_and(|a| {
                        f.metadata.as_ref().and_then(|m| m.virtual_address).is_some_and(|b| a == b)
                    })
            });
            if let Some(curr_func) = curr_func {
                let curr_func_info = ChangeItemInfo::from(curr_func);
                if prev_func_info != curr_func_info {
                    items.push(ChangeItem {
                        name: curr_func.name.clone(),
                        from: Some(prev_func_info),
                        to: Some(curr_func_info),
                        metadata: curr_func.metadata.clone(),
                    });
                }
            } else {
                items.push(ChangeItem {
                    name: prev_func.name.clone(),
                    from: Some(prev_func_info),
                    to: None,
                    metadata: prev_func.metadata.clone(),
                });
            }
        }
        for curr_func in curr_items {
            let curr_func_address = curr_func.metadata.as_ref().and_then(|m| m.virtual_address);
            if !prev_items.iter().any(|f| {
                f.name == curr_func.name
                    || curr_func_address.is_some_and(|a| {
                        f.metadata.as_ref().and_then(|m| m.virtual_address).is_some_and(|b| a == b)
                    })
            }) {
                items.push(ChangeItem {
                    name: curr_func.name.clone(),
                    from: None,
                    to: Some(ChangeItemInfo::from(curr_func)),
                    metadata: curr_func.metadata.clone(),
                });
            }
        }
    } else {
        for prev_func in prev_items {
            items.push(ChangeItem {
                name: prev_func.name.clone(),
                from: Some(ChangeItemInfo::from(prev_func)),
                to: None,
                metadata: prev_func.metadata.clone(),
            });
        }
    }
    items
}

fn process_new_items(items: &[ReportItem]) -> Vec<ChangeItem> {
    items
        .iter()
        .map(|item| ChangeItem {
            name: item.name.clone(),
            from: None,
            to: Some(ChangeItemInfo::from(item)),
            metadata: item.metadata.clone(),
        })
        .collect()
}

fn measure_line_matched(
    name: &str,
    from: u64,
    from_percent: f32,
    to: u64,
    to_percent: f32,
) -> String {
    let emoji = if to > from { "📈" } else { "📉" };
    let percent_diff = to_percent - from_percent;
    let percent_str = if percent_diff < 0.0 {
        format!("{percent_diff:.2}%")
    } else {
        format!("+{percent_diff:.2}%")
    };
    let bytes_diff = to as i64 - from as i64;
    let bytes_str = match bytes_diff.cmp(&0) {
        Ordering::Less => bytes_diff.to_string(),
        Ordering::Equal | Ordering::Greater => format!("+{bytes_diff}"),
    };
    format!("{emoji} **{name}**: {to_percent:.2}% ({percent_str}, {bytes_str} bytes)\n")
}

fn measure_line_bytes(name: &str, from: u64, to: u64) -> String {
    let diff = to as i64 - from as i64;
    let diff_str = match diff.cmp(&0) {
        Ordering::Less => diff.to_string(),
        Ordering::Equal | Ordering::Greater => format!("+{diff}"),
    };
    format!("**{name}**: {to} bytes ({diff_str} bytes)\n")
}

fn measure_line_simple(name: &str, from: u64, to: u64) -> String {
    let diff = to as i64 - from as i64;
    let diff_str = match diff.cmp(&0) {
        Ordering::Less => diff.to_string(),
        Ordering::Equal | Ordering::Greater => format!("+{diff}"),
    };
    format!("**{name}**: {to} ({diff_str})\n")
}

fn generate_comment(
    from: &FullReportFile,
    to: &ProcessArtifactResult,
    to_commit: &Commit,
    changes: Changes,
) -> String {
    let mut comment = format!(
        "### Report for {} ({} - {})\n\n",
        to.version,
        &from.commit.sha[..7],
        &to_commit.sha[..7]
    );
    let mut measure_written = false;
    let from_measures = from.report.measures;
    let to_measures = to.report.measures.unwrap_or_default();
    if from_measures.total_code != to_measures.total_code {
        comment.push_str(&measure_line_bytes(
            "Total code",
            from_measures.total_code,
            to_measures.total_code,
        ));
        measure_written = true;
    }
    if from_measures.total_functions != to_measures.total_functions {
        comment.push_str(&measure_line_simple(
            "Total functions",
            from_measures.total_functions as u64,
            to_measures.total_functions as u64,
        ));
        measure_written = true;
    }
    if from_measures.matched_code != to_measures.matched_code {
        comment.push_str(&measure_line_matched(
            "Matched code",
            from_measures.matched_code,
            from_measures.matched_code_percent,
            to_measures.matched_code,
            to_measures.matched_code_percent,
        ));
        measure_written = true;
    }
    if from_measures.complete_code != to_measures.complete_code {
        comment.push_str(&measure_line_matched(
            "Linked code",
            from_measures.complete_code,
            from_measures.complete_code_percent,
            to_measures.complete_code,
            to_measures.complete_code_percent,
        ));
        measure_written = true;
    }
    if measure_written {
        comment.push('\n');
    }
    let mut total_changes = 0;
    let mut iter = changes.units.into_iter().flat_map(|mut unit| {
        let functions = take(&mut unit.functions);
        functions.into_iter().map(move |f| (unit.clone(), f))
    });
    for (unit, item) in iter.by_ref() {
        let (from, to) = match (item.from, item.to) {
            (Some(from), Some(to)) => (from, to),
            (None, Some(to)) => (ChangeItemInfo::default(), to),
            (Some(from), None) => (from, ChangeItemInfo::default()),
            (None, None) => continue,
        };
        let emoji = if to.fuzzy_match_percent == 100.0 {
            "✅"
        } else if to.fuzzy_match_percent > from.fuzzy_match_percent {
            "📈"
        } else {
            "📉"
        };
        let from_bytes = ((from.fuzzy_match_percent as f64 / 100.0) * from.size as f64) as u64;
        let to_bytes = ((to.fuzzy_match_percent as f64 / 100.0) * to.size as f64) as u64;
        let bytes_diff = to_bytes as i64 - from_bytes as i64;
        let bytes_str = match bytes_diff.cmp(&0) {
            Ordering::Less => bytes_diff.to_string(),
            Ordering::Equal => "0".to_string(),
            Ordering::Greater => format!("+{}", bytes_diff),
        };
        let name =
            item.metadata.as_ref().and_then(|m| m.demangled_name.as_deref()).unwrap_or(&item.name);
        comment.push_str(&format!(
            "{emoji} `{} | {}` {} bytes -> {:.2}%\n",
            unit.name, name, bytes_str, to.fuzzy_match_percent
        ));
        total_changes += 1;
        if total_changes >= 30 {
            break;
        }
    }
    let remaining = iter.count();
    if remaining > 0 {
        comment.push_str(&format!("...and {} more items\n", remaining));
    } else if total_changes == 0 {
        comment.push_str("No changes\n");
    }
    comment
}

#[allow(unused)]
fn platform_name(platform: &str) -> &str {
    match platform {
        "gc" => "GameCube",
        "wii" => "Wii",
        "n64" => "Nintendo 64",
        "switch" => "Nintendo Switch",
        "3ds" => "Nintendo 3DS",
        "nds" => "Nintendo DS",
        "gba" => "Game Boy Advance",
        "gbc" => "Game Boy Color",
        "ps" => "PlayStation",
        "ps2" => "PlayStation 2",
        _ => platform,
    }
}
