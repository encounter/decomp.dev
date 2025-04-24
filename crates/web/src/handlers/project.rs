use std::{str::FromStr, sync::Arc, time::Instant};

use anyhow::{Context, anyhow};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use decomp_dev_auth::CurrentUser;
use decomp_dev_core::{
    AppError, FullUri,
    models::{Commit, Platform, Project},
    util::UrlExt,
};
use maud::{DOCTYPE, Markup, html};
use objdiff_core::bindings::report::Measures;
use serde::{Deserialize, Serialize};
use tokio::{sync::Semaphore, task::JoinSet};
use url::Url;

use crate::{
    AppState,
    handlers::common::{code_progress_sections, date, footer, header, nav_links, size, timeago},
};

#[derive(Serialize)]
struct ProjectInfoContext {
    project: Project,
    commit: Commit,
    measures: Measures,
}

#[derive(Deserialize)]
pub struct ProjectsQuery {
    sort: Option<String>,
}

#[derive(Serialize, Copy, Clone)]
struct SortOption {
    key: &'static str,
    name: &'static str,
}

const SORT_OPTIONS: &[SortOption] = &[
    SortOption { key: "updated", name: "Last updated" },
    SortOption { key: "name", name: "Name" },
    SortOption { key: "matched_code_percent", name: "Matched Code (Percent)" },
    SortOption { key: "matched_code", name: "Matched Code" },
    SortOption { key: "total_code", name: "Total Code" },
];

#[derive(Serialize, Clone)]
pub struct ProgressSection {
    pub class: String,
    pub percent: f32,
    pub tooltip: String,
}

pub async fn get_projects(
    State(state): State<AppState>,
    Query(query): Query<ProjectsQuery>,
    FullUri(uri): FullUri,
    current_user: Option<CurrentUser>,
) -> Result<Response, AppError> {
    let start = Instant::now();
    let projects = state.db.get_projects().await?;
    let mut out = projects
        .iter()
        .filter_map(|p| {
            let commit = p.commit.as_ref()?;
            Some(ProjectInfoContext {
                project: p.project.clone(),
                commit: commit.clone(),
                measures: Default::default(),
            })
        })
        .collect::<Vec<_>>();

    // Fetch latest report for each
    let sem = Arc::new(Semaphore::new(10));
    let mut join_set = JoinSet::new();
    for info in projects {
        let sem = sem.clone();
        let state = state.clone();
        join_set.spawn(async move {
            let _permit = sem.acquire().await;
            let Some(version) = info.default_version() else {
                return (info, Err(anyhow!("No report version found")));
            };
            let commit = info.commit.as_ref().unwrap();
            let report = state
                .db
                .get_report(&info.project.owner, &info.project.repo, &commit.sha, version)
                .await
                .with_context(|| {
                    format!(
                        "Failed to fetch report for {}/{} sha {} version {}",
                        info.project.owner, info.project.repo, commit.sha, version
                    )
                });
            (info, report)
        });
    }
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok((info, Ok(Some(file)))) => {
                if let Some(c) = out.iter_mut().find(|i| i.project.id == info.project.id) {
                    c.measures = *file.report.measures(info.project.default_category.as_deref());
                }
            }
            Ok((info, Ok(None))) => {
                tracing::warn!("No report found for {}", info.project.id);
            }
            Ok((info, Err(e))) => {
                tracing::error!("Failed to fetch report for {}: {:?}", info.project.id, e);
            }
            Err(e) => {
                tracing::error!("Failed to fetch report: {:?}", e);
            }
        }
    }

    let current_sort_key = query.sort.as_deref().unwrap_or("updated");
    let current_sort = SORT_OPTIONS
        .iter()
        .find(|s| s.key.eq_ignore_ascii_case(current_sort_key))
        .copied()
        .ok_or(AppError::Status(StatusCode::BAD_REQUEST))?;
    match current_sort.key {
        "name" => out.sort_by(|a, b| a.project.name().cmp(&b.project.name())),
        "updated" => out.sort_by(|a, b| b.commit.timestamp.cmp(&a.commit.timestamp)),
        "matched_code_percent" => out.sort_by(|a, b| {
            b.measures
                .matched_code_percent
                .partial_cmp(&a.measures.matched_code_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "matched_code" => out.sort_by(|a, b| {
            b.measures
                .matched_code
                .partial_cmp(&a.measures.matched_code)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "total_code" => out.sort_by(|a, b| {
            b.measures
                .total_code
                .partial_cmp(&a.measures.total_code)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        _ => return Err(AppError::Status(StatusCode::BAD_REQUEST)),
    }

    let request_url = Url::parse(&uri.to_string()).context("Failed to parse URI")?;
    let canonical_url = request_url.with_path("");

    let rendered = html! {
        (DOCTYPE)
        html {
            head lang="en" {
                meta charset="utf-8";
                title { "Projects • decomp.dev" }
                (header())
                meta name="description" content="Decompilation progress reports";
                meta property="og:title" content="Decompilation progress reports";
                meta property="og:description" content="Progress reports for matching decompilation projects";
                meta property="og:image" content=(canonical_url.with_path("/og.png"));
                meta property="og:url" content=(canonical_url);
            }
            body {
                header {
                    nav {
                        ul {
                            li {
                                a href="https://decomp.dev" { strong { "decomp.dev" } }
                            }
                            li {
                                a href="/" { "Projects" }
                            }
                            li class="md" {
                                details class="dropdown" {
                                    summary { (current_sort.name) }
                                    ul {
                                        @for option in SORT_OPTIONS {
                                            li {
                                                a href=(request_url.query_param("sort", Some(option.key))) {
                                                    (option.name)
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        (nav_links())
                    }
                    div class="title-group" {
                        h3 { "Progress Reports" }
                        blockquote {
                            "Matching decompilation projects attempt to write source code (C, C++)"
                            " that compiles to the same binary as the original."
                            " All source code is written from scratch."
                            footer {
                                a href="https://wiki.decomp.dev/" { "Learn more" }
                            }
                        }
                    }
                }
                main {
                    details class="dropdown sm" {
                        summary { (current_sort.name) }
                        ul {
                            @for option in SORT_OPTIONS {
                                li {
                                    a href=(request_url.query_param("sort", Some(option.key))) {
                                        (option.name)
                                    }
                                }
                            }
                        }
                    }
                    @for project in out {
                        (project_fragment(project, current_sort, &canonical_url))
                    }
                }
                (footer(start, current_user.as_ref()))
            }
        }
    };
    Ok(rendered.into_response())
}

fn project_fragment(
    info: ProjectInfoContext,
    current_sort: SortOption,
    canonical_url: &Url,
) -> Markup {
    let project_path =
        canonical_url.with_path(&format!("/{}/{}", info.project.owner, info.project.repo));
    let commit_url = format!(
        "https://github.com/{}/{}/commit/{}",
        info.project.owner, info.project.repo, info.commit.sha
    );
    html! {
        article class="project" {
            div class="project-header" {
                h3 class="project-title" {
                    a href=(project_path) { (info.project.name()) }
                }
                @if let Some(platform) = &info.project.platform {
                    @let platform_name = Platform::from_str(platform).map(|p| p.name()).unwrap_or(platform);
                    img class="platform-icon" src=(format!("/assets/platforms/{}.svg", platform))
                        alt=(platform_name) title=(platform_name) width="24" height="24";
                }
            }
            h6 {
                @if current_sort.key == "total_code" || current_sort.key == "matched_code" {
                    (size(info.measures.matched_code))
                    " matched code | "
                    (size(info.measures.total_code))
                    " total code"
                } @else {
                    (format!("{:.2}%", info.measures.matched_code_percent))
                    " decompiled"
                    @if info.measures.complete_code_percent > 0.0 {
                        " | "
                        (format!("{:.2}%", info.measures.complete_code_percent))
                        " fully linked"
                    }
                }
            }
            (code_progress_sections(&info.measures))
            small class="muted" {
                span title=(date(info.commit.timestamp)) { "Updated " (timeago(info.commit.timestamp)) }
                " in commit "
                a href=(commit_url) target="_blank" { (info.commit.sha[..7]) }
            }
        }
    }
}
