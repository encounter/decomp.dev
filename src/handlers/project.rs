use std::{sync::Arc, time::Instant};

use anyhow::{anyhow, Context};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use objdiff_core::bindings::report::Measures;
use serde::{Deserialize, Serialize};
use tokio::{sync::Semaphore, task::JoinSet};
use url::Url;

use super::{AppError, FullUri};
use crate::{
    handlers::report::TemplateMeasures,
    templates::{render, size},
    util::UrlExt,
    AppState,
};

#[derive(Serialize)]
struct ProjectsTemplateContext<'a> {
    projects: Vec<ProjectInfoContext>,
    sort_options: &'static [SortOption],
    current_sort: SortOption,
    canonical_url: &'a str,
    image_url: &'a str,
}

#[derive(Serialize)]
struct ProjectInfoContext {
    id: u64,
    path: String,
    owner: String,
    repo: String,
    name: String,
    short_name: String,
    commit: String,
    timestamp: DateTime<Utc>,
    measures: TemplateMeasures,
    platform: Option<String>,
    code_progress: Vec<ProgressSection>,
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
) -> Result<Response, AppError> {
    let start = Instant::now();
    let projects = state.db.get_projects().await?;
    let mut out = projects
        .iter()
        .filter_map(|p| {
            let commit = p.commit.as_ref()?;
            Some(ProjectInfoContext {
                id: p.project.id,
                path: format!("/{}/{}", p.project.owner, p.project.repo),
                owner: p.project.owner.clone(),
                repo: p.project.repo.clone(),
                name: p.project.name().into_owned(),
                short_name: p.project.short_name().to_owned(),
                commit: commit.sha.clone(),
                timestamp: commit.timestamp,
                measures: Default::default(),
                platform: p.project.platform.clone(),
                code_progress: vec![],
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
                if let Some(c) = out.iter_mut().find(|i| i.id == info.project.id) {
                    let measures = file.report.measures(info.project.default_category.as_deref());
                    c.measures = TemplateMeasures::from(measures);
                    c.code_progress = code_progress_sections(measures);
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
        "name" => out.sort_by(|a, b| a.name.cmp(&b.name)),
        "updated" => out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp)),
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
    let image_url = canonical_url.with_path("/og.png");

    let mut rendered = render(&state.templates, "projects.html", ProjectsTemplateContext {
        projects: out,
        sort_options: SORT_OPTIONS,
        current_sort,
        canonical_url: canonical_url.as_str(),
        image_url: image_url.as_str(),
    })?;
    let elapsed = start.elapsed();
    rendered = rendered.replace("[[time]]", &format!("{}ms", elapsed.as_millis()));
    Ok(Html(rendered).into_response())
}

pub fn code_progress_sections(measures: &Measures) -> Vec<ProgressSection> {
    let mut out = vec![];
    if measures.total_code == 0 {
        return out;
    }
    let mut add_section = |class: &str, percent: f32, tooltip: String| {
        out.push(ProgressSection { class: class.to_owned(), percent, tooltip });
    };
    let mut current_percent = 0.0;
    if measures.complete_code_percent > 0.0 {
        add_section(
            "progress-section",
            measures.complete_code_percent - current_percent,
            format!(
                "{:.2}% fully linked ({})",
                measures.complete_code_percent,
                size(measures.complete_code)
            ),
        );
        current_percent = measures.complete_code_percent;
    }
    if measures.matched_code_percent > 0.0 && measures.matched_code_percent > current_percent {
        add_section(
            if current_percent > 0.0 { "progress-section striped" } else { "progress-section" },
            measures.matched_code_percent - current_percent,
            format!(
                "{:.2}% perfect match ({})",
                measures.matched_code_percent,
                size(measures.matched_code)
            ),
        );
        current_percent = measures.matched_code_percent;
    }
    if measures.fuzzy_match_percent > 0.0 && measures.fuzzy_match_percent > current_percent {
        add_section(
            "progress-section striped fuzzy",
            measures.fuzzy_match_percent - current_percent,
            format!("{:.2}% fuzzy match", measures.fuzzy_match_percent),
        );
    }
    out
}

pub fn data_progress_sections(measures: &Measures) -> Vec<ProgressSection> {
    let mut out = vec![];
    if measures.total_data == 0 {
        return out;
    }
    let mut add_section = |class: &str, percent: f32, tooltip: String| {
        out.push(ProgressSection { class: class.to_owned(), percent, tooltip });
    };
    let mut current_percent = 0.0;
    if measures.complete_data_percent > 0.0 {
        add_section(
            "progress-section",
            measures.complete_data_percent - current_percent,
            format!(
                "{:.2}% fully linked ({})",
                measures.complete_data_percent,
                size(measures.complete_data)
            ),
        );
        current_percent = measures.complete_data_percent;
    }
    if measures.matched_data_percent > 0.0 && measures.matched_data_percent > current_percent {
        add_section(
            if current_percent > 0.0 { "progress-section striped" } else { "progress-section" },
            measures.matched_data_percent - current_percent,
            format!(
                "{:.2}% perfect match ({})",
                measures.matched_data_percent,
                size(measures.matched_data)
            ),
        );
    }
    out
}
