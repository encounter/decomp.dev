use std::{borrow::Cow, iter, time::Instant};

use anyhow::{Context, Result};
use axum::{
    Form, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Redirect, Response},
};
use decomp_dev_auth::CurrentUser;
use decomp_dev_core::{
    AppError, FullUri,
    models::{FullReportFile, ProjectInfo},
    util::UrlExt,
};
use decomp_dev_images::{
    badge,
    treemap::{layout_units, unit_color},
};
use image::ImageFormat;
use maud::{DOCTYPE, Markup, PreEscaped, html};
use mime::Mime;
use objdiff_core::bindings::report::{Measures, ReportCategory, ReportUnit};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use url::Url;

use super::{parse_accept, treemap};
use crate::{
    AppState,
    handlers::common::{
        code_progress_sections, data_progress_sections, footer, header, nav_links, size,
    },
    proto::{PROTOBUF, Protobuf},
};

#[derive(Deserialize)]
pub struct ReportParams {
    owner: String,
    repo: String,
    version: Option<String>,
    commit: Option<String>,
}

const DEFAULT_IMAGE_WIDTH: u32 = 950;
const DEFAULT_IMAGE_HEIGHT: u32 = 475;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportQuery {
    mode: Option<String>,
    category: Option<String>,
    w: Option<u32>,
    h: Option<u32>,
    #[serde(flatten)]
    shield: badge::ShieldParams,
    unit: Option<String>,
}

impl ReportQuery {
    pub fn size(&self) -> (u32, u32) {
        (self.w.unwrap_or(DEFAULT_IMAGE_WIDTH), self.h.unwrap_or(DEFAULT_IMAGE_HEIGHT))
    }
}

#[derive(Serialize)]
pub struct ReportTemplateUnit<'a> {
    pub name: &'a str,
    pub total_code: u64,
    pub fuzzy_match_percent: f32,
    pub color: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Serialize, Clone)]
struct ReportCategoryItem<'a> {
    id: &'a str,
    name: &'a str,
    path: String,
}

#[derive(Serialize, Clone)]
struct ReportTemplateVersion<'a> {
    id: &'a str,
    path: String,
}

/// Duplicate of Measures to avoid omitting empty fields
#[derive(Serialize, Default, Clone)]
pub struct TemplateMeasures {
    pub fuzzy_match_percent: f32,
    pub total_code: u64,
    pub matched_code: u64,
    pub matched_code_percent: f32,
    pub total_data: u64,
    pub matched_data: u64,
    pub matched_data_percent: f32,
    pub total_functions: u32,
    pub matched_functions: u32,
    pub matched_functions_percent: f32,
    pub complete_code: u64,
    pub complete_code_percent: f32,
    pub complete_data: u64,
    pub complete_data_percent: f32,
    pub total_units: u32,
    pub complete_units: u32,
}

impl From<&Measures> for TemplateMeasures {
    fn from(
        &Measures {
            fuzzy_match_percent,
            total_code,
            matched_code,
            matched_code_percent,
            total_data,
            matched_data,
            matched_data_percent,
            total_functions,
            matched_functions,
            matched_functions_percent,
            complete_code,
            complete_code_percent,
            complete_data,
            complete_data_percent,
            total_units,
            complete_units,
        }: &Measures,
    ) -> Self {
        Self {
            fuzzy_match_percent,
            total_code,
            matched_code,
            matched_code_percent,
            total_data,
            matched_data,
            matched_data_percent,
            total_functions,
            matched_functions,
            matched_functions_percent,
            complete_code,
            complete_code_percent,
            complete_data,
            complete_data_percent,
            total_units,
            complete_units,
        }
    }
}

fn is_valid_extension(ext: &str) -> bool {
    // FIXME: hack for versions that have .nn where nn is a number
    ext.chars().any(|c| c.is_ascii_alphabetic())
}

fn extract_extension(params: ReportParams) -> (ReportParams, Option<String>) {
    if let Some(commit) = params.commit.as_deref() {
        if let Some((commit, ext)) = commit.rsplit_once('.') {
            if is_valid_extension(ext) {
                return (
                    ReportParams { commit: Some(commit.to_string()), ..params },
                    Some(ext.to_string()),
                );
            }
        }
    } else if let Some(version) = params.version.as_deref() {
        if let Some((version, ext)) = version.rsplit_once('.') {
            if is_valid_extension(ext) {
                return (
                    ReportParams { version: Some(version.to_string()), ..params },
                    Some(ext.to_string()),
                );
            }
        }
    } else if let Some((repo, ext)) = params.repo.rsplit_once('.') {
        if is_valid_extension(ext) {
            return (ReportParams { repo: repo.to_string(), ..params }, Some(ext.to_string()));
        }
    }
    (params, None)
}

pub async fn get_report(
    Path(params): Path<ReportParams>,
    Query(query): Query<ReportQuery>,
    headers: HeaderMap,
    FullUri(uri): FullUri,
    State(state): State<AppState>,
    current_user: Option<CurrentUser>,
) -> Result<Response, AppError> {
    let start = Instant::now();
    let (params, ext) = extract_extension(params);
    let acceptable = parse_accept(&headers, ext.as_deref());
    if acceptable.is_empty() {
        return Err(AppError::Status(StatusCode::NOT_ACCEPTABLE));
    }

    let mut commit = params.commit.as_deref();
    if matches!(commit, Some(c) if c.eq_ignore_ascii_case("latest")) {
        commit = None;
    }
    let Some(project_info) = state.db.get_project_info(&params.owner, &params.repo, commit).await?
    else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    let Some(commit) = project_info.commit.as_ref() else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    let version = if let Some(version) = &params.version {
        if version.eq_ignore_ascii_case("default") {
            project_info.default_version().ok_or(AppError::Status(StatusCode::NOT_FOUND))?
        } else {
            version.as_str()
        }
    } else if let Some(default_version) = project_info.default_version() {
        default_version
    } else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    let Some(report) =
        state.db.get_report(&params.owner, &params.repo, &commit.sha, version).await?
    else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    let report = state.db.upgrade_report(&report).await?;

    let scope = apply_scope(&report, &project_info, &query)?;
    match query.mode.as_deref().unwrap_or("report").to_ascii_lowercase().as_str() {
        "shield" => mode_shield(&scope, query, &acceptable),
        "report" => mode_report(&scope, &state, uri, query, start, &acceptable, current_user).await,
        "measures" => mode_measures(&scope, &acceptable),
        "history" => mode_history(&scope, &state, query, &acceptable).await,
        _ => Err(AppError::Status(StatusCode::BAD_REQUEST)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn mode_report(
    scope: &Scope<'_>,
    state: &AppState,
    uri: Uri,
    query: ReportQuery,
    start: Instant,
    acceptable: &[Mime],
    current_user: Option<CurrentUser>,
) -> Result<Response, AppError> {
    for mime in acceptable {
        if (mime.type_() == mime::STAR && mime.subtype() == mime::STAR)
            || (mime.type_() == mime::TEXT && mime.subtype() == mime::HTML)
        {
            let rendered = render_template(scope, state, uri, current_user, start).await?;
            return Ok(rendered.into_response());
        } else if mime.type_() == mime::APPLICATION && mime.subtype() == mime::JSON {
            let flattened = scope.report.report.flatten();
            return Ok(Json(flattened).into_response());
        } else if mime.type_() == mime::APPLICATION && mime.subtype() == PROTOBUF {
            let flattened = scope.report.report.flatten();
            return Ok(Protobuf(&flattened).into_response());
        } else if mime.type_() == mime::IMAGE && mime.subtype() == mime::SVG {
            let (w, h) = query.size();
            let svg = treemap::render_svg(&scope.units, w, h);
            return Ok(([(header::CONTENT_TYPE, mime::IMAGE_SVG.as_ref())], svg).into_response());
        } else if mime.type_() == mime::IMAGE {
            let format = if mime.subtype() == mime::STAR {
                // Default to PNG
                ImageFormat::Png
            } else {
                ImageFormat::from_mime_type(mime.essence_str())
                    .ok_or_else(|| AppError::Status(StatusCode::NOT_ACCEPTABLE))?
            };
            let (w, h) = query.size();
            let data = treemap::render_image(&scope.units, w, h, format)?;
            return Ok(([(header::CONTENT_TYPE, format.to_mime_type())], data).into_response());
        }
    }
    Err(AppError::Status(StatusCode::NOT_ACCEPTABLE))
}

fn mode_shield(
    &Scope { project_info, measures, label, .. }: &Scope<'_>,
    query: ReportQuery,
    acceptable: &[Mime],
) -> Result<Response, AppError> {
    let label = label.unwrap_or_else(|| project_info.project.short_name());
    for mime in acceptable {
        if (mime.type_() == mime::STAR && mime.subtype() == mime::STAR)
            || (mime.type_() == mime::IMAGE && mime.subtype() == mime::SVG)
            || (mime.type_() == mime::TEXT && mime.subtype() == mime::HTML)
        {
            let data = badge::render_svg(measures, label, &query.shield)?;
            return Ok(([(header::CONTENT_TYPE, mime::IMAGE_SVG.as_ref())], data).into_response());
        } else if mime.type_() == mime::APPLICATION && mime.subtype() == mime::JSON {
            let data = badge::render(measures, label, &query.shield)?;
            return Ok(Json(data).into_response());
        } else if mime.type_() == mime::IMAGE {
            let format = if mime.subtype() == mime::STAR {
                // Default to PNG
                ImageFormat::Png
            } else {
                ImageFormat::from_mime_type(mime.essence_str())
                    .ok_or_else(|| AppError::Status(StatusCode::NOT_ACCEPTABLE))?
            };
            let data = badge::render_image(measures, label, &query.shield, format)?;
            return Ok(([(header::CONTENT_TYPE, format.to_mime_type())], data).into_response());
        }
    }
    Err(AppError::Status(StatusCode::NOT_ACCEPTABLE))
}

fn mode_measures(
    &Scope { measures, .. }: &Scope<'_>,
    acceptable: &[Mime],
) -> Result<Response, AppError> {
    for mime in acceptable {
        if (mime.type_() == mime::STAR && mime.subtype() == mime::STAR)
            || (mime.type_() == mime::APPLICATION && mime.subtype() == mime::JSON)
        {
            return Ok(Json(measures).into_response());
        } else if mime.type_() == mime::APPLICATION && mime.subtype() == PROTOBUF {
            return Ok(Protobuf(measures).into_response());
        }
    }
    Err(AppError::Status(StatusCode::NOT_ACCEPTABLE))
}

#[derive(Serialize)]
struct ReportHistoryEntry {
    timestamp: String,
    commit_sha: String,
    measures: TemplateMeasures,
}

async fn mode_history(
    scope: &Scope<'_>,
    state: &AppState,
    query: ReportQuery,
    acceptable: &[Mime],
) -> Result<Response, AppError> {
    let report_measures =
        state.db.fetch_all_reports(&scope.project_info.project, &scope.report.version).await?;
    let mut result = Vec::with_capacity(report_measures.len());
    for report in report_measures {
        let mut measures =
            Some(*report.report.measures(scope.project_info.project.default_category.as_deref()));
        if let Some(unit_name) = query.unit.as_ref() {
            let full_report = state.db.upgrade_report(&report).await?;
            measures = full_report
                .report
                .units
                .iter()
                .find(|u| &u.name == unit_name)
                .and_then(|c| c.measures.as_ref())
                .copied();
        } else if let Some(category_id) = query.category.as_ref() {
            measures = report
                .report
                .categories
                .iter()
                .find(|c| &c.id == category_id)
                .and_then(|c| c.measures.as_ref())
                .copied();
        }
        let Some(measures) = &measures else {
            continue;
        };
        result.push(ReportHistoryEntry {
            timestamp: report
                .commit
                .timestamp
                .format(&Rfc3339)
                .unwrap_or_else(|_| "[invalid]".to_string()),
            commit_sha: report.commit.sha,
            measures: TemplateMeasures::from(measures),
        });
    }
    for mime in acceptable {
        if (mime.type_() == mime::STAR && mime.subtype() == mime::STAR)
            || (mime.type_() == mime::APPLICATION && mime.subtype() == mime::JSON)
        {
            return Ok(Json(result).into_response());
        }
    }
    Err(AppError::Status(StatusCode::NOT_ACCEPTABLE))
}

const EMPTY_MEASURES: Measures = Measures {
    fuzzy_match_percent: 0.0,
    total_code: 0,
    matched_code: 0,
    matched_code_percent: 0.0,
    total_data: 0,
    matched_data: 0,
    matched_data_percent: 0.0,
    total_functions: 0,
    matched_functions: 0,
    matched_functions_percent: 0.0,
    complete_code: 0,
    complete_code_percent: 0.0,
    complete_data: 0,
    complete_data_percent: 0.0,
    total_units: 0,
    complete_units: 0,
};

struct Scope<'a> {
    report: &'a FullReportFile,
    project_info: &'a ProjectInfo,
    measures: &'a Measures,
    current_category: Option<&'a ReportCategory>,
    current_unit: Option<&'a ReportUnit>,
    units: Vec<ReportTemplateUnit<'a>>,
    label: Option<&'a str>,
}

fn apply_scope<'a>(
    report: &'a FullReportFile,
    project_info: &'a ProjectInfo,
    query: &ReportQuery,
) -> Result<Scope<'a>> {
    let mut measures = &report.report.measures;
    let mut current_category = None;
    let mut category_id_filter = None;
    if let Some(category) = query
        .category
        .as_deref()
        .or(project_info.project.default_category.as_deref())
        .and_then(|id| report.report.categories.iter().find(|c| c.id == *id))
    {
        measures = category.measures.as_ref().unwrap_or(&EMPTY_MEASURES);
        current_category = Some(category);
        category_id_filter = Some(category.id.clone());
    }
    let mut current_unit = None;
    if let Some(unit) = query
        .unit
        .as_ref()
        .and_then(|unit_name| report.report.units.iter().find(|u| u.name == *unit_name))
    {
        measures = unit.measures.as_ref().unwrap_or(&EMPTY_MEASURES);
        current_unit = Some(unit.as_ref());
    }
    let (w, h) = query.size();
    let mut units = if let Some(unit) = current_unit {
        unit.functions
            .iter()
            .filter_map(|f| {
                if f.size == 0 {
                    return None;
                }
                Some(ReportTemplateUnit {
                    name: f
                        .metadata
                        .as_ref()
                        .and_then(|m| m.demangled_name.as_deref())
                        .unwrap_or(&f.name),
                    total_code: f.size,
                    fuzzy_match_percent: f.fuzzy_match_percent,
                    color: unit_color(f.fuzzy_match_percent),
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                })
            })
            .collect::<Vec<_>>()
    } else {
        report
            .report
            .units
            .iter()
            .filter_map(|unit| {
                if let Some(category_id) = &category_id_filter {
                    if !unit
                        .metadata
                        .as_ref()
                        .is_some_and(|m| m.progress_categories.iter().any(|c| c == category_id))
                    {
                        return None;
                    }
                }
                let measures = unit.measures.as_ref()?;
                if measures.total_code == 0 {
                    return None;
                }
                Some(ReportTemplateUnit {
                    name: &unit.name,
                    total_code: measures.total_code,
                    fuzzy_match_percent: measures.fuzzy_match_percent,
                    color: unit_color(measures.fuzzy_match_percent),
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                })
            })
            .collect::<Vec<_>>()
    };
    layout_units(
        &mut units,
        w as f32 / h as f32,
        |i| i.total_code as f32,
        |i, r| {
            i.x = r.x;
            i.y = r.y;
            i.w = r.w;
            i.h = r.h;
        },
    );
    let label = current_unit
        .as_ref()
        .map(|u| u.name.rsplit_once('/').map_or(u.name.as_str(), |(_, name)| name))
        .or_else(|| current_category.as_ref().map(|c| c.name.as_str()));
    Ok(Scope { report, project_info, measures, current_category, current_unit, units, label })
}

async fn render_template(
    scope: &Scope<'_>,
    state: &AppState,
    uri: Uri,
    current_user: Option<CurrentUser>,
    start: Instant,
) -> Result<Markup> {
    let Scope { report, project_info, measures, current_category, current_unit, units, label } =
        scope;

    let mut commit_message = report.commit.message.clone();
    if commit_message.is_none() {
        let commit = match state
            .github
            .get_commit(&project_info.project.owner, &project_info.project.repo, &report.commit.sha)
            .await
        {
            Ok(commit) => commit,
            Err(e) => {
                tracing::warn!(
                    "Failed to get commit {}/{}@{}: {}",
                    project_info.project.owner,
                    project_info.project.repo,
                    report.commit.sha,
                    e
                );
                None
            }
        };
        if let Some(commit) = commit {
            state
                .db
                .update_report_message(project_info.project.id, &report.commit.sha, &commit.message)
                .await?;
            commit_message = Some(commit.message);
        }
    }

    let request_url = Url::parse(&uri.to_string()).context("Failed to parse URI")?;
    let project_base_path =
        format!("/{}/{}", project_info.project.owner, project_info.project.repo);
    let canonical_url = request_url.with_path(&format!(
        "/{}/{}/{}/{}",
        project_info.project.owner, project_info.project.repo, report.version, report.commit.sha
    ));
    let image_url = canonical_url.with_path(&format!("{}.png", canonical_url.path()));

    let versions = project_info
        .report_versions
        .iter()
        .map(|version| {
            let version_url = request_url.with_path(&format!(
                "/{}/{}/{}/{}",
                project_info.project.owner, project_info.project.repo, version, report.commit.sha
            ));
            ReportTemplateVersion { id: version, path: version_url.path_and_query().to_string() }
        })
        .collect::<Vec<_>>();

    let all_url = canonical_url.query_param("category", None);
    let all_category =
        ReportCategoryItem { id: "all", name: "All", path: all_url.path_and_query().to_string() };
    let current_category = current_category
        .map(|c| {
            let path =
                canonical_url.query_param("category", Some(&c.id)).path_and_query().to_string();
            ReportCategoryItem { id: &c.id, name: &c.name, path }
        })
        .unwrap_or_else(|| all_category.clone());
    let categories = iter::once(all_category)
        .chain(report.report.categories.iter().map(|c| {
            let path =
                canonical_url.query_param("category", Some(&c.id)).path_and_query().to_string();
            ReportCategoryItem { id: &c.id, name: &c.name, path }
        }))
        .collect::<Vec<_>>();

    let prev_commit_path = project_info.prev_commit.as_deref().map(|commit| {
        let url = request_url.with_path(&format!(
            "/{}/{}/{}/{}",
            project_info.project.owner, project_info.project.repo, report.version, commit
        ));
        url.path_and_query().to_string()
    });
    let next_commit_path = project_info.next_commit.as_deref().map(|commit| {
        let url = request_url.with_path(&format!(
            "/{}/{}/{}/{}",
            project_info.project.owner, project_info.project.repo, report.version, commit
        ));
        url.path_and_query().to_string()
    });
    let latest_commit_path = project_info.next_commit.as_deref().map(|_| {
        let url = request_url.with_path(&format!(
            "/{}/{}/{}",
            project_info.project.owner, project_info.project.repo, report.version
        ));
        url.path_and_query().to_string()
    });

    let units_path = canonical_url.query_param("unit", None).path_and_query().to_string();
    let commit_message = commit_message.as_deref().and_then(|message| message.lines().next());
    let commit_url = format!("{}/commit/{}", project_info.project.repo_url(), report.commit.sha);
    let source_file_url = current_unit
        .and_then(|u| u.metadata.as_ref())
        .and_then(|m| m.source_path.as_deref())
        .map(|path| {
            format!("{}/blob/{}/{}", project_info.project.repo_url(), report.commit.sha, path)
        });
    let project_name = if let Some(label) = label {
        Cow::Owned(format!("{} ({})", project_info.project.name(), label))
    } else {
        project_info.project.name()
    };
    let project_short_name = if let Some(label) = label {
        Cow::Owned(format!("{} ({})", project_info.project.short_name(), label))
    } else {
        Cow::Borrowed(project_info.project.short_name())
    };

    Ok(html! {
        (DOCTYPE)
        html {
            head lang="en" {
                meta charset="utf-8";
                title { (project_short_name) " • Progress Report" }
                (header())
                meta name="description" content=(format!("Decompilation progress report for {project_name}"));
                meta property="og:title" content=(format!("{project_short_name} is {:.2}% decompiled", measures.matched_code_percent));
                meta property="og:description" content=(format!("Decompilation progress report for {project_name}"));
                meta property="og:image" content=(image_url);
                meta property="og:url" content=(canonical_url);
                script src="/js/treemap.min.js" {}
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
                            li {
                                a href=(project_base_path) { (project_short_name) }
                            }
                            li class="md" {
                                details class="dropdown" {
                                    summary { (report.version) }
                                    ul {
                                        @for version in &versions {
                                            li {
                                                a href=(version.path) { (version.id) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        (nav_links())
                    }
                }
                main {
                    h3 { (format!("{project_short_name} is {:.2}% decompiled", measures.matched_code_percent)) }
                    @if current_unit.is_none() && measures.complete_code_percent > 0.0 {
                        h4 class="muted" { (format!("{:.2}% fully linked", measures.complete_code_percent)) }
                    }
                    @if let Some(source_file_url) = source_file_url {
                        h4 class="muted" {
                            a href=(source_file_url) target="_blank" { "View source file" }
                        }
                    }
                    details class="dropdown sm" {
                        summary { (report.version) }
                        ul {
                            @for version in &versions {
                                li {
                                    a href=(version.path) { (version.id) }
                                }
                            }
                        }
                    }
                    @if measures.total_code > 0 {
                        h6 class="report-header" {
                            "Code "
                            small class="muted" { "(" (size(measures.total_code)) ")" }
                        }
                        (code_progress_sections(&measures))
                    }
                    @if measures.total_data > 0 {
                        h6 class="report-header" {
                            "Data "
                            small class="muted" { "(" (size(measures.total_data)) ")" }
                        }
                        (data_progress_sections(&measures))
                    }
                    h6 class="report-header" { "Commit" }
                    div {
                        @if let Some(message) = commit_message {
                            pre {
                                a href=(commit_url) target="_blank" {
                                    (report.commit.sha[..7])
                                }
                                " | "
                                (message)
                            }
                        }
                        div role="group" {
                            @if let Some(prev_commit_path) = prev_commit_path {
                                a role="button" class="outline secondary" href=(prev_commit_path) {
                                    "Previous"
                                }
                            } @else {
                                button disabled class="outline secondary" {
                                    "Previous"
                                }
                            }
                            @if let Some(next_commit_path) = next_commit_path {
                                a role="button" class="outline secondary" href=(next_commit_path) {
                                    "Next"
                                }
                            } @else {
                                button disabled class="outline secondary" {
                                    "Next"
                                }
                            }
                            @if let Some(latest_commit_path) = latest_commit_path {
                                a role="button" class="primary" href=(latest_commit_path) {
                                    "Latest"
                                }
                            } @else {
                                button disabled class="primary" {
                                    "Latest"
                                }
                            }
                        }
                    }
                    @if current_unit.is_some() {
                        h6 class="report-header" { "Functions" }
                        div role="group" {
                            a role="button" href=(units_path) { "Back to units" }
                        }
                    } @else {
                        h6 class="report-header" { "Units" }
                        @if categories.len() > 1 {
                            details class="dropdown" {
                                summary { (current_category.name) }
                                ul {
                                    @for category in &categories {
                                        li {
                                            a href=(category.path) { (category.name) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    script {
                        (PreEscaped(r#"document.write('<canvas id="treemap" width="100%"></canvas>');drawTreemap("treemap","#))
                        (current_unit.is_none())
                        ","
                        (PreEscaped(serde_json::to_string(&units)?))
                        ");"
                    }
                    noscript {
                        img #treemap src=(image_url) alt="Progress graph";
                    }
                    @if current_user.as_ref().is_some_and(|u| u.permissions_for_repo(project_info.project.id).admin) {
                        (manage_form(project_info))
                    }
                }
            }
            (footer(start, current_user.as_ref()))
        }
    })
}

fn manage_form(project_info: &ProjectInfo) -> Markup {
    let project_base_path =
        format!("/{}/{}", project_info.project.owner, project_info.project.repo);
    let default_version = project_info.default_version();
    html! {
        h6 class="report-header" { "Manage" }
        form action=(project_base_path) method="post" {
            fieldset {
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
                    @if project_info.project.enable_pr_comments {
                        input name="enable_pr_comments" type="checkbox" role="switch" checked;
                    } @else {
                        input name="enable_pr_comments" type="checkbox" role="switch";
                    }
                    "Enable PR comments"
                }
            }
            button type="submit" { "Save" }
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
    #[serde(default, deserialize_with = "form_bool")]
    pub enable_pr_comments: bool,
    pub default_version: Option<String>,
}

pub async fn save_project(
    Path(params): Path<ReportParams>,
    State(state): State<AppState>,
    current_user: CurrentUser,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    let Some(project_info) = state.db.get_project_info(&params.owner, &params.repo, None).await?
    else {
        return Err(AppError::Status(StatusCode::NOT_FOUND));
    };
    if !current_user.permissions_for_repo(project_info.project.id).admin {
        return Err(AppError::Status(StatusCode::FORBIDDEN));
    }
    state
        .db
        .update_project_settings(
            project_info.project.id,
            form.enable_pr_comments,
            form.default_version,
        )
        .await?;
    let redirect_url = format!("/{}/{}", params.owner, params.repo);
    Ok(Redirect::to(&redirect_url).into_response())
}
