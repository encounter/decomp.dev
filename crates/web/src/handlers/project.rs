use std::{str::FromStr, sync::Arc};

use anyhow::{Context, anyhow};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use decomp_dev_auth::CurrentUser;
use decomp_dev_core::{
    AppError, FullUri,
    models::{
        ALL_PLATFORMS, CachedReportFile, Commit, Platform, ProjectInfo, ProjectVisibility,
        ReportInner, project_visibility,
    },
    util::{UrlExt, format_percent, size},
};
use itertools::Itertools;
use maud::{DOCTYPE, Markup, html};
use objdiff_core::bindings::report::Measures;
use serde::{Deserialize, Serialize};
use time::{UtcDateTime, format_description::well_known::Rfc3339};
use tokio::{sync::Semaphore, task::JoinSet};
use url::Url;

use crate::{
    AppState,
    handlers::{
        common::{Load, ProgressSections, TemplateContext, date, nav_links, timeago},
        parse_accept,
        report::TemplateMeasures,
    },
};

struct ProjectInfoContext {
    info: ProjectInfo,
    measures: Measures,
    report: Option<CachedReportFile>,
    code_progress: ProgressSections,
}

#[derive(Deserialize)]
pub struct ProjectsQuery {
    sort: Option<String>,
    platform: Option<String>,
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

#[derive(serde::Serialize)]
pub struct ProjectsResponse {
    pub projects: Vec<ProjectResponse>,
}

#[derive(serde::Serialize)]
pub struct ProjectResponse {
    pub id: u64,
    pub owner: String,
    pub repo: String,
    pub repo_url: String,
    pub name: Option<String>,
    pub short_name: Option<String>,
    pub platform: Option<String>,
    pub default_version: Option<String>,
    pub default_category: Option<String>,
    pub commit: Option<CommitResponse>,
    pub measures: TemplateMeasures,
    pub report_versions: Vec<String>,
    pub report_categories: Vec<CategoryResponse>,
}

impl ProjectResponse {
    pub fn new<U>(info: &ProjectInfo, measures: &Measures, report: &ReportInner<U>) -> Self {
        ProjectResponse {
            id: info.project.id,
            owner: info.project.owner.clone(),
            repo: info.project.repo.clone(),
            repo_url: info.project.repo_url(),
            name: info.project.name.clone(),
            short_name: info.project.short_name.clone(),
            platform: info.project.platform.clone(),
            default_version: info.default_version().map(|v| v.to_string()),
            default_category: info.project.default_category.clone(),
            commit: info.commit.clone().map(CommitResponse::from),
            measures: measures.into(),
            report_versions: info.report_versions.clone(),
            report_categories: report
                .categories
                .iter()
                .map(|c| CategoryResponse {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    measures: c.measures.as_ref().map(TemplateMeasures::from).unwrap_or_default(),
                })
                .collect(),
        }
    }
}

#[derive(serde::Serialize)]
pub struct CategoryResponse {
    pub id: String,
    pub name: String,
    pub measures: TemplateMeasures,
}

#[derive(serde::Serialize)]
pub struct CommitResponse {
    pub sha: String,
    pub message: Option<String>,
    pub timestamp: String,
}

impl From<Commit> for CommitResponse {
    fn from(value: Commit) -> Self {
        Self {
            sha: value.sha,
            message: value.message,
            timestamp: value.timestamp.format(&Rfc3339).unwrap_or_else(|_| "[invalid]".to_string()),
        }
    }
}

fn extract_extension(uri: &Uri) -> Option<String> {
    let path = uri.path();
    if let Some(pos) = path.rfind('.') {
        let ext = &path[pos + 1..];
        if !ext.is_empty() {
            return Some(ext.to_string());
        }
    }
    None
}

pub async fn get_projects(
    ctx: TemplateContext,
    State(state): State<AppState>,
    Query(query): Query<ProjectsQuery>,
    FullUri(uri): FullUri,
    current_user: Option<CurrentUser>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let ext = extract_extension(&uri);
    let acceptable = parse_accept(&headers, ext.as_deref());
    if acceptable.is_empty() {
        return Err(AppError::Status(StatusCode::NOT_ACCEPTABLE));
    }

    let platforms = query
        .platform
        .as_deref()
        .into_iter()
        .flat_map(|s| s.split(','))
        .filter_map(|s| Platform::from_str(s).ok())
        .sorted()
        .dedup()
        .collect::<Vec<_>>();
    let show_all = platforms.is_empty() || platforms == ALL_PLATFORMS;

    let projects = state.db.get_projects().await?;

    let mut out = projects
        .iter()
        .map(|p| ProjectInfoContext {
            info: p.clone(),
            measures: Default::default(),
            report: None,
            code_progress: Default::default(),
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
                if let Some(c) = out.iter_mut().find(|i| i.info.project.id == info.project.id) {
                    c.measures = *file.report.measures(info.project.default_category.as_deref());
                    c.report = Some(file.clone());
                    c.code_progress = ctx.code_progress_sections(&c.measures);
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

    // Hide projects that are disabled or don't meet visibility criteria
    out.retain(|c| {
        project_visibility(&c.info.project, Some(&c.measures)) == ProjectVisibility::Visible
    });

    let available_platforms = out
        .iter()
        .filter_map(|p| p.info.project.platform.as_deref())
        .filter_map(|s| Platform::from_str(s).ok())
        .sorted()
        .dedup_with_count()
        .collect::<Vec<_>>();

    if !show_all {
        out.retain(|c| {
            c.info
                .project
                .platform
                .as_deref()
                .and_then(|p| Platform::from_str(p).ok())
                .is_some_and(|p| platforms.contains(&p))
        });
    }

    let current_sort_key = query.sort.as_deref().unwrap_or("updated");
    let current_sort = SORT_OPTIONS
        .iter()
        .find(|s| s.key.eq_ignore_ascii_case(current_sort_key))
        .copied()
        .ok_or(AppError::Status(StatusCode::BAD_REQUEST))?;
    match current_sort.key {
        "name" => out.sort_by(|a, b| {
            lexicmp::natural_lexical_cmp(&a.info.project.name(), &b.info.project.name())
        }),
        "updated" => out.sort_by(|a, b| {
            let a_ts =
                a.report.as_ref().map(|r| r.commit.timestamp).unwrap_or(UtcDateTime::UNIX_EPOCH);
            let b_ts =
                b.report.as_ref().map(|r| r.commit.timestamp).unwrap_or(UtcDateTime::UNIX_EPOCH);
            b_ts.cmp(&a_ts)
        }),
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

    for mime in acceptable {
        if (mime.type_() == mime::STAR && mime.subtype() == mime::STAR)
            || (mime.type_() == mime::TEXT && mime.subtype() == mime::HTML)
        {
            return render_project(
                ctx,
                out,
                uri,
                current_sort,
                current_user.as_ref(),
                &available_platforms,
                &platforms,
                show_all,
            )
            .await;
        } else if mime.type_() == mime::APPLICATION && mime.subtype() == mime::JSON {
            let projects = out
                .into_iter()
                .filter_map(|info| {
                    Some(ProjectResponse::new(
                        &info.info,
                        &info.measures,
                        &info.report.as_ref()?.report,
                    ))
                })
                .collect::<Vec<_>>();
            return Ok(Json(ProjectsResponse { projects }).into_response());
        }
    }
    Err(AppError::Status(StatusCode::NOT_ACCEPTABLE))
}

async fn render_project(
    mut ctx: TemplateContext,
    mut out: Vec<ProjectInfoContext>,
    uri: Uri,
    current_sort: SortOption,
    current_user: Option<&CurrentUser>,
    available_platforms: &[(usize, Platform)],
    platforms: &[Platform],
    show_all: bool,
) -> Result<Response, AppError> {
    let mut combined_styles = ProgressSections { nonce: ctx.nonce.clone(), ..Default::default() };
    for info in &mut out {
        combined_styles.width_classes.append(&mut info.code_progress.width_classes);
    }

    let request_url = Url::parse(&uri.to_string()).context("Failed to parse URI")?;
    let canonical_url = request_url.with_path("/projects");
    let is_primary_view = show_all && current_sort.key == "updated";

    let rendered = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "Projects • decomp.dev" }
                (ctx.header().await)
                (ctx.chunks("main", Load::Deferred).await)
                (ctx.chunks("projects", Load::Deferred).await)
                link rel="canonical" href=(canonical_url);
                meta name="description" content="Decompilation progress reports";
                meta property="og:title" content="Decompilation progress reports";
                meta property="og:description" content="Progress reports for matching decompilation projects";
                meta property="og:image" content=(canonical_url.with_path("/og.png"));
                meta property="og:url" content=(canonical_url);
                @if !is_primary_view {
                    // Prevent search engines from indexing anything but the primary view
                    meta name="robots" content="noindex";
                }
            }
            body {
                header {
                    nav {
                        ul {
                            li {
                                a href="/" { strong { "decomp.dev" } }
                            }
                            li {
                                a href="/projects" { "Projects" }
                            }
                        }
                        (nav_links())
                    }
                    .title-group {
                        h3 { "Progress Reports" }
                        blockquote {
                            "Matching decompilation projects attempt to write source code (C, C++)"
                            " that compiles to the same binary as the original."
                            " All source code is written from scratch."
                            footer {
                                a href="https://decomp.wiki/" { "Learn more" }
                            }
                        }
                    }
                }
                main {
                    .grid.platform-grid {
                        details.dropdown.platform-dropdown {
                            summary {
                                @if show_all {
                                    "All Platforms"
                                } @else if platforms.len() == 1 {
                                    (platforms[0].name())
                                } @else {
                                    (platforms.len()) " Platforms"
                                }
                            }
                            ul {
                                li {
                                    a href=(request_url.query_param("platform", None)) {
                                        "All Platforms"
                                    }
                                }
                                @for (n, platform) in available_platforms {
                                    li.platform-item {
                                        label {
                                            input type="checkbox" name="platform" value=(platform.to_str())
                                                checked[show_all || platforms.contains(platform)];
                                            span.platform-icon.(format!("icon-{}", platform.to_str())) {}
                                            (platform.name())
                                            span.count-badge { (n) }
                                        }
                                        button.secondary { "Only" }
                                    }
                                }
                            }
                        }
                        details.dropdown {
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
                    (combined_styles)
                    @for project in out {
                        (project_fragment(project, current_sort, &canonical_url))
                    }
                }
                (ctx.footer(current_user))
            }
        }
    };
    Ok((ctx, rendered).into_response())
}

fn project_fragment(
    ctx: ProjectInfoContext,
    current_sort: SortOption,
    canonical_url: &Url,
) -> Markup {
    let project = &ctx.info.project;
    let Some(commit) = ctx.report.as_ref().map(|r| r.commit.clone()) else {
        return Markup::default();
    };
    let mut project_path = canonical_url.with_path(&format!("/{}/{}", project.owner, project.repo));
    project_path.set_query(None);
    let commit_url =
        format!("https://github.com/{}/{}/commit/{}", project.owner, project.repo, commit.sha);
    let header_image_id = project.header_image_id.map(hex::encode);
    const HEADER_QUERY: &str = "?w=1024&h=256";
    html! {
        article.project data-platform=[project.platform.as_deref()] {
            a.project-link href=(project_path) aria-label="View project" {}
            @if let Some(header_image_id) = header_image_id {
                .project-image-container {
                    picture {
                        source srcset=(format!("/images/{header_image_id}.avif{HEADER_QUERY}")) type="image/avif";
                        source srcset=(format!("/images/{header_image_id}.webp{HEADER_QUERY}")) type="image/webp";
                        img.project-image src=(format!("/images/{header_image_id}.jpg{HEADER_QUERY}")) alt=(project.name()) loading="lazy";
                    }
                }
            }
            .project-header {
                h3.project-title { (project.name()) }
                @if let Some(platform) = &project.platform {
                    @let platform_name = Platform::from_str(platform).map(|p| p.name()).unwrap_or(platform);
                    span.platform-icon.(format!("icon-{platform}")) title=(platform_name) {}
                }
            }
            h6 {
                @if current_sort.key == "total_code" || current_sort.key == "matched_code" {
                    (size(ctx.measures.matched_code))
                    " matched code | "
                    (size(ctx.measures.total_code))
                    " total code"
                } @else {
                    (format_percent(ctx.measures.matched_code_percent))
                    " decompiled"
                    @if ctx.measures.complete_code_percent > 0.0 {
                        " | "
                        (format_percent(ctx.measures.complete_code_percent))
                        " fully linked"
                    }
                }
            }
            (ctx.code_progress)
            small.muted {
                span title=(date(commit.timestamp)) { "Updated " (timeago(commit.timestamp)) }
                " in commit "
                a href=(commit_url) target="_blank" { (commit.sha[..7]) }
            }
        }
    }
}
