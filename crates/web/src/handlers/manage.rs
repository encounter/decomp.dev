use std::time::Instant;

use anyhow::Result;
use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use decomp_dev_auth::CurrentUser;
use decomp_dev_core::{
    AppError,
    models::{ALL_PLATFORMS, Project, ProjectInfo},
};
use decomp_dev_github::{check_for_reports, graphql::RepositoryPermission, refresh_project};
use itertools::Itertools;
use maud::{DOCTYPE, Markup, html};
use serde::Deserialize;

use crate::{
    AppState,
    handlers::common::{chunks, footer, header, nav_links},
};

pub async fn manage(
    State(state): State<AppState>,
    current_user: CurrentUser,
) -> Result<Markup, AppError> {
    let start = Instant::now();

    let projects = state
        .db
        .get_projects()
        .await?
        .into_iter()
        .filter(|p| current_user.can_manage_repo(p.project.id))
        .sorted_by(|a, b| a.project.name().cmp(&b.project.name()))
        .collect::<Vec<_>>();

    Ok(html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "Manage • decomp.dev" }
                (header())
                (chunks("main", true).await)
                (chunks("manage", true).await)
            }
            body {
                header {
                    nav {
                        ul {
                            li {
                                a href="https://decomp.dev" { strong { "decomp.dev" } }
                            }
                            li {
                                a href="/manage" { "Manage" }
                            }
                        }
                        (nav_links())
                    }
                }
                main {
                    h3 { "Projects" }
                    p { a href="/manage/new" role="button" { "Add New" } }
                    @if projects.is_empty() {
                        article { "No projects found." }
                    }
                    @for project in projects {
                        (project_fragment(&project))
                    }
                }
            }
            (footer(start, Some(&current_user)))
        }
    })
}

fn project_fragment(info: &ProjectInfo) -> Markup {
    let project_path = format!("/manage/{}/{}", info.project.owner, info.project.repo);
    html! {
        article .project {
            .project-header {
                h3 .project-title {
                    a href=(project_path) { (info.project.name()) }
                }
            }
            .project-info {
                a href=(info.project.repo_url()) { (info.project.repo_url()) }
            }
        }
    }
}

pub async fn new(
    State(state): State<AppState>,
    current_user: CurrentUser,
) -> Result<Response, AppError> {
    render_new(&state, &current_user, None, None).await
}

async fn render_new(
    state: &AppState,
    current_user: &CurrentUser,
    message: Option<&str>,
    prefill: Option<&Project>,
) -> Result<Response, AppError> {
    let start = Instant::now();

    let projects = state.db.get_projects().await?;

    let repos = current_user
        .data
        .repositories
        .iter()
        .filter(|r| r.permission == RepositoryPermission::Admin)
        .map(|r| {
            (r.id, format!("{}/{}", r.owner, r.name), projects.iter().any(|p| p.project.id == r.id))
        })
        .collect::<Vec<_>>();

    let current_name = prefill.as_ref().and_then(|p| p.name.as_deref()).unwrap_or("");
    let current_short_name = prefill.as_ref().and_then(|p| p.short_name.as_deref()).unwrap_or("");
    let current_platform = prefill.as_ref().and_then(|p| p.platform.as_deref());

    let repo_options = html! {
        @for (id, repo, exists) in repos {
            @if exists {
                option value=(id) disabled { (repo) }
            } @else if prefill.is_some_and(|p| p.id == id) {
                option value=(id) selected { (repo) }
            } @else {
                option value=(id) { (repo) }
            }
        }
    };

    Ok(html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "New Project • decomp.dev" }
                (header())
                (chunks("main", true).await)
                (chunks("manage", true).await)
            }
            body {
                header {
                    nav {
                        ul {
                            li {
                                a href="https://decomp.dev" { strong { "decomp.dev" } }
                            }
                            li {
                                a href="/manage" { "Manage" }
                            }
                            li {
                                a href="/manage/new" { "New" }
                            }
                        }
                        (nav_links())
                    }
                }
                main {
                    h3 { "Add New" }
                    form method="post" data-loading="Processing..." {
                        fieldset {
                            label {
                                "Repository"
                                @if let Some(message) = message {
                                    select name="repo" aria-invalid="true" { (repo_options) }
                                    small { (message) }
                                } @else {
                                    select name="repo" { (repo_options) }
                                    small { "Repository must be public. Admin permissions are required." }
                                }
                            }
                            label {
                                "Game name"
                                input name="name" required value=(current_name);
                                small { "Please use the full name of the game, e.g. \"The Legend of Zelda: Ocarina of Time\"." }
                            }
                            label {
                                "Short name "
                                small { "(optional)" }
                                input name="short_name" value=(current_short_name);
                                small { "If the game has a long prefix, e.g. \"The Legend of Zelda\", the short name will be \"Ocarina of Time\"." }
                            }
                            label {
                                "Platform"
                                select name="platform" required { (platform_options(current_platform)) }
                                small { "Platform not listed? Please open an issue on GitHub." }
                            }
                        }
                        button type="submit" { "Add" }
                    }
                }
            }
            (footer(start, Some(current_user)))
        }
    }.into_response())
}

fn platform_options(current_platform: Option<&str>) -> Markup {
    html! {
        @if current_platform.is_none() {
            option value="" disabled selected { "Select one" }
        }
        @for platform in ALL_PLATFORMS {
            @let platform_str = platform.to_str();
            @if current_platform == Some(platform_str) {
                option value=(platform_str) selected { (platform.name()) }
            } @else {
                option value=(platform_str) { (platform.name()) }
            }
        }
    }
}

#[derive(Deserialize)]
pub struct NewForm {
    repo: u64,
    name: String,
    short_name: String,
    platform: String,
}

pub async fn new_save(
    State(state): State<AppState>,
    current_user: CurrentUser,
    Form(form): Form<NewForm>,
) -> Result<Response, AppError> {
    if let Some(existing) = state.db.get_project_info_by_id(form.repo, None).await? {
        return Ok(Redirect::to(&format!("/{}/{}", existing.project.owner, existing.project.repo))
            .into_response());
    }
    let Some(platform) = ALL_PLATFORMS.iter().find(|p| p.to_str() == form.platform) else {
        return Err(AppError::Status(StatusCode::BAD_REQUEST));
    };
    let client = current_user.client()?;
    let repo = match client.repos_by_id(form.repo).get().await {
        Ok(repo) => repo,
        Err(e) => {
            tracing::error!("Failed to fetch repository: {:?}", e);
            return render_new(
                &state,
                &current_user,
                Some("Failed to fetch repository information."),
                None,
            )
            .await;
        }
    };

    let name = form.name.trim();
    let short_name = form.short_name.trim();
    let mut project = Project {
        id: repo.id.into_inner(),
        owner: repo.owner.as_ref().map(|o| o.login.clone()).unwrap_or_default(),
        repo: repo.name.clone(),
        name: (!name.is_empty()).then_some(name.to_string()),
        short_name: (!short_name.is_empty()).then_some(short_name.to_string()),
        platform: Some(platform.to_str().to_string()),
        ..Default::default()
    };
    if repo.permissions.as_ref().is_none_or(|p| !p.admin) {
        return render_new(
            &state,
            &current_user,
            Some("You do not have admin permissions on this repository."),
            Some(&project),
        )
        .await;
    }

    let workflow_id = match check_for_reports(&client, &project, &repo).await {
        Ok(workflow_id) => workflow_id,
        Err(e) => {
            let message = e.to_string();
            return render_new(&state, &current_user, Some(&message), Some(&project)).await;
        }
    };
    project.workflow_id = Some(workflow_id);
    state.db.create_project(&project).await?;
    refresh_project(&state.github, &state.db, project.id, Some(&client), true).await?;
    Ok(Redirect::to(&format!("/{}/{}", project.owner, project.repo)).into_response())
}

pub async fn manage_project(
    Path(params): Path<ProjectParams>,
    State(state): State<AppState>,
    current_user: CurrentUser,
) -> Result<Markup, AppError> {
    let start = Instant::now();
    let Some(project_info) = state.db.get_project_info(&params.owner, &params.repo, None).await?
    else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    if !current_user.can_manage_repo(project_info.project.id) {
        return Err(AppError::Status(StatusCode::FORBIDDEN));
    }

    Ok(render_manage_project(start, &state, &project_info, &current_user, Message::None).await)
}

enum Message {
    None,
    Info(String),
    Error(String),
}

fn render_message(message: &Message) -> Markup {
    match message {
        Message::None => Markup::default(),
        Message::Info(msg) => html! {
            article .info-card { (msg) }
        },
        Message::Error(msg) => html! {
            article .error-card { (msg) }
        },
    }
}

async fn render_manage_project(
    start: Instant,
    state: &AppState,
    project_info: &ProjectInfo,
    current_user: &CurrentUser,
    message: Message,
) -> Markup {
    let project_short_name = project_info.project.short_name();
    let project_manage_path =
        format!("/manage/{}/{}", project_info.project.owner, project_info.project.repo);
    let refresh_path =
        format!("/manage/{}/{}/refresh", project_info.project.owner, project_info.project.repo);
    let default_version = project_info.default_version();

    let current_name = project_info.project.name.as_deref().unwrap_or("");
    let current_short_name = project_info.project.short_name.as_deref().unwrap_or("");
    let current_platform = project_info.project.platform.as_deref();
    let current_workflow_id = project_info.project.workflow_id.as_deref().unwrap_or("");

    let installation_id = if let Some(installations) = &state.github.installations {
        let installations = installations.lock().await;
        installations.repo_to_installation.get(&project_info.project.id).cloned()
    } else {
        None
    };

    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { (project_short_name) " • Manage" }
                (header())
                (chunks("main", true).await)
                (chunks("manage", true).await)
            }
            body {
                header {
                    nav {
                        ul {
                            li {
                                a href="https://decomp.dev" { strong { "decomp.dev" } }
                            }
                            li {
                                a href="/manage" { "Manage" }
                            }
                            li {
                                a href=(project_manage_path) { (project_short_name) }
                            }
                        }
                        (nav_links())
                    }
                }
                main {
                    h3 { "Edit " (project_short_name) }
                    (render_message(&message))
                    form method="post" data-loading="Saving..." {
                        fieldset {
                            label {
                                "Repository"
                                input type="text" readonly disabled value=(project_info.project.repo_url());
                            }
                            label {
                                "Game name"
                                input name="name" required value=(current_name);
                                small { "Please use the full name of the game, e.g. \"The Legend of Zelda: Ocarina of Time\"." }
                            }
                            label {
                                "Short name "
                                small { "(optional)" }
                                input name="short_name" value=(current_short_name) placeholder=(current_name);
                                small { "If the game has a long prefix, e.g. \"The Legend of Zelda\", the short name will be \"Ocarina of Time\"." }
                            }
                            label {
                                "Platform"
                                select name="platform" { (platform_options(current_platform)) }
                                small { "Platform not listed? Please open an issue on GitHub." }
                            }
                            label {
                                "Default version"
                                select name="default_version" {
                                    @for version in &project_info.report_versions {
                                        @if default_version == Some(version.as_str()) {
                                            option value=(version) selected { (version) }
                                        } @else {
                                            option value=(version) { (version) }
                                        }
                                    }
                                }
                            }
                            label {
                                "GitHub workflow ID"
                                input name="workflow_id" type="text" value=(current_workflow_id);
                                small { "The GitHub Actions workflow that contains report artifacts." }
                            }
                            label {
                                @if installation_id.is_some() {
                                    @if project_info.project.enable_pr_comments {
                                        input name="enable_pr_comments" type="checkbox" role="switch" checked;
                                    } @else {
                                        input name="enable_pr_comments" type="checkbox" role="switch";
                                    }
                                    "Enable PR comments"
                                } @else {
                                    @if project_info.project.enable_pr_comments {
                                        input name="enable_pr_comments" type="checkbox" role="switch" disabled checked;
                                    } @else {
                                        input name="enable_pr_comments" type="checkbox" role="switch" disabled;
                                    }
                                    "Enable PR comments (requires GitHub App installation)"
                                }
                            }
                        }
                        button type="submit" { "Save" }
                    }
                    h4 { "Debug" }
                    @if let Some(installation_id) = installation_id {
                        p {
                            "GitHub App installation ID: "
                            kbd { (installation_id) }
                        }
                    } @else {
                        p {
                            "No GitHub App installation found. "
                            a href="https://github.com/apps/decomp-dev" target="_blank" { "Install the app" }
                        }
                    }
                    .grid {
                        form action=(refresh_path) method="post" data-loading="Refreshing..." {
                            button .outline .secondary type="submit" { "Force refresh" }
                            small { "Fetches any missing report artifacts." }
                        }
                    }
                }
            }
            (footer(start, Some(current_user)))
        }
    }
}

fn form_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where D: serde::Deserializer<'de> {
    match <&str>::deserialize(deserializer)? {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(serde::de::Error::unknown_variant(other, &["on", "off"])),
    }
}

#[derive(Deserialize)]
pub struct ProjectForm {
    pub name: String,
    pub short_name: String,
    pub platform: String,
    pub default_version: Option<String>,
    pub workflow_id: String,
    #[serde(default, deserialize_with = "form_bool")]
    pub enable_pr_comments: bool,
}

#[derive(Deserialize)]
pub struct ProjectParams {
    owner: String,
    repo: String,
}

pub async fn manage_project_save(
    Path(params): Path<ProjectParams>,
    State(state): State<AppState>,
    current_user: CurrentUser,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    let Some(project_info) = state.db.get_project_info(&params.owner, &params.repo, None).await?
    else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    if !current_user.can_manage_repo(project_info.project.id) {
        return Err(AppError::Status(StatusCode::FORBIDDEN));
    }
    let installation_id = if let Some(installations) = &state.github.installations {
        let installations = installations.lock().await;
        installations.repo_to_installation.get(&project_info.project.id).cloned()
    } else {
        None
    };
    let name = form.name.trim();
    let short_name = form.short_name.trim();
    let platform = form.platform.trim();
    let workflow_id = form.workflow_id.trim();
    let project = Project {
        id: project_info.project.id,
        owner: project_info.project.owner,
        repo: project_info.project.repo,
        name: (!name.is_empty()).then_some(name.to_string()),
        short_name: (!short_name.is_empty()).then_some(short_name.to_string()),
        default_category: project_info.project.default_category,
        default_version: form.default_version,
        platform: (!platform.is_empty()).then_some(platform.to_string()),
        workflow_id: (!workflow_id.is_empty()).then_some(workflow_id.to_string()),
        // If there's no installation ID, use the existing value
        enable_pr_comments: if installation_id.is_some() {
            form.enable_pr_comments
        } else {
            project_info.project.enable_pr_comments
        },
    };
    state.db.update_project(&project).await?;
    let redirect_url = format!("/{}/{}", params.owner, params.repo);
    Ok(Redirect::to(&redirect_url).into_response())
}

pub async fn manage_project_refresh(
    Path(params): Path<ProjectParams>,
    State(state): State<AppState>,
    current_user: CurrentUser,
) -> Result<Markup, AppError> {
    let start = Instant::now();
    let Some(project_info) = state.db.get_project_info(&params.owner, &params.repo, None).await?
    else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    if !current_user.can_manage_repo(project_info.project.id) {
        return Err(AppError::Status(StatusCode::FORBIDDEN));
    }
    let client = current_user.client()?;
    let message = match refresh_project(
        &state.github,
        &state.db,
        project_info.project.id,
        Some(&client),
        true,
    )
    .await
    {
        Ok(inserted_reports) => Message::Info(format!("Fetched {} new reports", inserted_reports)),
        Err(e) => {
            tracing::error!("Failed to refresh project: {:?}", e);
            Message::Error(format!("Failed to refresh project: {}", e))
        }
    };
    Ok(render_manage_project(start, &state, &project_info, &current_user, message).await)
}
