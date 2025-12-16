use anyhow::{Context, Result};
use apalis::prelude::*;
use octocrab::models::InstallationId;

use crate::{JobContext, ProcessWorkflowRunJob, RefreshProjectJob};

/// Process a completed workflow run job.
///
/// This handles:
/// - Fetching the project info from the database
/// - Getting the appropriate GitHub client (installation or personal token)
/// - Processing workflow run artifacts
/// - Inserting reports for push events
/// - Generating and posting PR comments for pull request events
pub async fn process_workflow_run_job(
    job: ProcessWorkflowRunJob,
    ctx: Data<JobContext>,
) -> Result<()> {
    tracing::info!(
        "Processing workflow run job: repo={} run={} event={}",
        job.repository_id,
        job.run_id,
        job.event
    );

    // Look up the project
    let Some(project_info) = ctx
        .db
        .get_project_info_by_id(job.repository_id, None)
        .await
        .context("Failed to fetch project info")?
    else {
        tracing::warn!("No project found for repository ID {}", job.repository_id);
        return Ok(());
    };

    // Get the appropriate GitHub client
    let client = if let Some(installation_id) = job.installation_id {
        if let Some(installations) = &ctx.github.installations {
            let mut installations = installations.lock().await;
            installations
                .client_for_installation(InstallationId(installation_id))
                .await
                .context("Failed to get installation client")?
        } else {
            ctx.github.client.clone()
        }
    } else {
        ctx.github.client_for(job.repository_id).await.context("Failed to get GitHub client")?
    };

    // Fetch the repository to get the default branch
    let repository =
        client.repos_by_id(job.repository_id).get().await.context("Failed to fetch repository")?;

    // Process the workflow run to get artifacts
    let result =
        decomp_dev_github::process_workflow_run(&client, &project_info.project, job.run_id.into())
            .await
            .context("Failed to process workflow run")?;

    tracing::debug!(
        "Processed workflow run {} ({}) (artifacts {})",
        job.run_id,
        job.head_sha,
        result.artifacts.len()
    );

    if result.artifacts.is_empty() {
        return Ok(());
    }

    // Create commit info
    let commit = decomp_dev_core::models::Commit {
        sha: job.head_sha.clone(),
        timestamp: time::UtcDateTime::now(),
        message: None,
    };

    // Handle push events - insert reports into database
    if job.event == "push" {
        let is_default_branch = match &repository.default_branch {
            Some(default_branch) => default_branch == &job.head_branch,
            None => matches!(job.head_branch.as_str(), "master" | "main"),
        };

        if is_default_branch {
            for artifact in result.artifacts {
                let start = std::time::Instant::now();
                ctx.db
                    .insert_report(
                        &project_info.project,
                        &commit,
                        &artifact.version,
                        *artifact.report,
                    )
                    .await
                    .context("Failed to insert report")?;
                let duration = start.elapsed();
                tracing::info!(
                    "Inserted report {} ({}) in {}ms",
                    artifact.version,
                    commit.sha,
                    duration.as_millis()
                );
            }
        }
    } else if matches!(job.event.as_str(), "pull_request" | "pull_request_target") {
        // Handle pull request events - generate and post comments
        if !project_info.project.enable_pr_comments {
            return Ok(());
        }

        let Some(base_commit) = project_info.commit else {
            tracing::warn!("No base commit found for repository ID {}", job.repository_id);
            return Ok(());
        };

        // Get all versions that exist on the base branch
        let base_versions = ctx
            .db
            .get_versions_for_commit(
                &project_info.project.owner,
                &project_info.project.repo,
                &base_commit.sha,
            )
            .await
            .context("Failed to get base versions")?;

        let mut version_comments = Vec::new();

        // Process existing artifacts from PR
        for artifact in &result.artifacts {
            let cached_report = ctx
                .db
                .get_report(
                    &project_info.project.owner,
                    &project_info.project.repo,
                    &base_commit.sha,
                    &artifact.version,
                )
                .await
                .context("Failed to get cached report")?;

            if let Some(cached_report) = cached_report {
                let report_file = ctx
                    .db
                    .upgrade_report(&cached_report)
                    .await
                    .context("Failed to upgrade report")?;
                let report = report_file.report.flatten();
                let changes =
                    decomp_dev_github::changes::generate_changes(&report, &artifact.report)
                        .context("Failed to generate changes")?;
                version_comments.push(decomp_dev_github::changes::generate_comment(
                    &report,
                    &artifact.report,
                    Some(&report_file.version),
                    Some(&report_file.commit),
                    Some(&commit),
                    changes,
                ));
            } else {
                tracing::warn!(
                    "No base report found for version {} (base {})",
                    artifact.version,
                    base_commit.sha
                );
                version_comments.push(decomp_dev_github::changes::generate_missing_report_comment(
                    &artifact.version,
                    Some(&base_commit),
                    Some(&commit),
                ));
            }
        }

        // Check for versions that exist on base but are missing from PR
        for base_version in &base_versions {
            if !result.artifacts.iter().any(|a| a.version == *base_version) {
                version_comments.push(decomp_dev_github::changes::generate_missing_report_comment(
                    base_version,
                    Some(&base_commit),
                    Some(&commit),
                ));
            }
        }

        if !version_comments.is_empty() {
            let combined_comment =
                decomp_dev_github::changes::generate_combined_comment(version_comments);

            // Post/update comments for each associated PR
            for pr_number in &job.pull_request_numbers {
                post_pr_comment(
                    &client,
                    &project_info.project,
                    &repository,
                    *pr_number,
                    &combined_comment,
                )
                .await
                .context("Failed to post PR comment")?;
            }
        }
    }

    Ok(())
}

/// Post or update a PR comment with the report.
async fn post_pr_comment(
    client: &octocrab::Octocrab,
    project: &decomp_dev_core::models::Project,
    repository: &octocrab::models::Repository,
    pr_number: u64,
    combined_comment: &str,
) -> Result<()> {
    use decomp_dev_core::models::PullReportStyle;

    if project.pr_report_style == PullReportStyle::Description {
        let pulls = client.pulls(&project.owner, &project.repo);
        let pull = pulls.get(pr_number).await.context("Failed to get pull request")?;
        let start_marker = "<!-- decomp.dev report start -->";
        let end_marker = "<!-- decomp.dev report end -->";
        let new_section = format!("{start_marker}\n{combined_comment}\n{end_marker}");
        let existing_body = pull.body.unwrap_or_default();
        let new_body = if let Some(start_idx) = existing_body.find(start_marker) {
            if let Some(end_rel) = existing_body[start_idx..].find(end_marker) {
                let end_idx = start_idx + end_rel + end_marker.len();
                format!(
                    "{}{}{}",
                    &existing_body[..start_idx],
                    new_section,
                    &existing_body[end_idx..]
                )
            } else {
                format!("{existing_body}\n\n---\n\n{new_section}")
            }
        } else if existing_body.trim().is_empty() {
            new_section
        } else {
            format!("{}\n\n---\n\n{}", existing_body.trim(), new_section)
        };

        pulls
            .update(pr_number)
            .body(new_body)
            .send()
            .await
            .context("Failed to update pull request body")?;
    } else {
        let repository_id = repository.id.into_inner();
        let issues = client.issues_by_id(repository_id);
        // Only fetch first page for now
        let existing_comments = issues.list_comments(pr_number).send().await?;

        // Find existing report comments
        let existing_report_comments: Vec<_> = existing_comments
            .items
            .iter()
            .filter(|comment| {
                comment.body.as_ref().is_some_and(|body| body.contains("### Report for "))
            })
            .collect();

        if let Some(first_comment) = existing_report_comments.first() {
            // Update the first comment
            issues
                .update_comment(first_comment.id, combined_comment.to_string())
                .await
                .context("Failed to update existing comment")?;

            // Delete any additional report comments
            for comment in existing_report_comments.iter().skip(1) {
                if let Err(e) = issues.delete_comment(comment.id).await {
                    tracing::warn!("Failed to delete old comment {}: {}", comment.id, e);
                }
            }
        } else {
            // Create new comment
            issues
                .create_comment(pr_number, combined_comment.to_string())
                .await
                .context("Failed to create comment")?;
        }
    }

    Ok(())
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

    match decomp_dev_github::refresh_project(
        &ctx.github,
        &ctx.db,
        job.repository_id,
        None, // No client override for background jobs
        job.full_refresh,
    )
    .await
    {
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
