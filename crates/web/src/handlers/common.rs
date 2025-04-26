use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use axum::http::StatusCode;
use decomp_dev_auth::CurrentUser;
use decomp_dev_core::AppError;
use maud::{Markup, PreEscaped, html};
use objdiff_core::bindings::report::Measures;
use time::{UtcDateTime, macros::format_description};

pub fn timeago(value: UtcDateTime) -> String {
    let Ok(duration) = Duration::try_from(UtcDateTime::now() - value) else {
        return "[out of range]".to_string();
    };
    timeago::Formatter::new().convert(duration)
}

pub fn date(value: UtcDateTime) -> String {
    value.format(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second] [offset_hour sign:mandatory]:[offset_minute]"
    )).unwrap_or_else(|_| "[invalid]".to_string())
}

pub fn header() -> Markup {
    html! {
        meta name="viewport" content="width=device-width, initial-scale=1.0";
        meta name="color-scheme" content="dark light";
        meta name="darkreader-lock";
        script { (PreEscaped(r#"let t;try{t=localStorage.getItem("theme")}catch(_){}if(t)document.documentElement.setAttribute("data-theme",t);"#)) }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebpackManifest {
    // pub all_files: Vec<String>,
    pub entries: HashMap<String, WebpackManifestEntry>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebpackManifestEntry {
    pub initial: WebpackManifestEntryPaths,
    // pub r#async: WebpackManifestEntryPaths,
    // pub html: Vec<String>,
    // pub assets: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebpackManifestEntryPaths {
    pub js: Vec<String>,
    pub css: Vec<String>,
}

pub async fn manifest_paths(entry: &str) -> Result<WebpackManifestEntryPaths> {
    let manifest_str = tokio::fs::read_to_string("dist/manifest.json").await?;
    let manifest: WebpackManifest = serde_json::from_str(&manifest_str)?;
    let entry = manifest
        .entries
        .get(entry)
        .ok_or_else(|| anyhow!("Entry {} not found in manifest", entry))?;
    Ok(entry.initial.clone())
}

pub async fn chunks(entry: &str, defer: bool) -> Markup {
    let paths = manifest_paths(entry).await.unwrap_or_else(|e| {
        tracing::error!("Failed to load chunks for {entry}: {e}");
        Default::default()
    });
    let mut out = String::new();
    for path in paths.css {
        out.push_str(&html! { link rel="stylesheet" href=(path); }.0);
    }
    if defer {
        for path in paths.js {
            out.push_str(&html! { script src=(path) defer {} }.0);
        }
    } else {
        for path in paths.js {
            out.push_str(&html! { script src=(path) {} }.0);
        }
    }
    PreEscaped(out)
}

pub fn footer(start: Instant, current_user: Option<&CurrentUser>) -> Markup {
    let elapsed = start.elapsed();
    html! {
        footer {
            span class="section" {
                small class="muted" { "Generated in " (elapsed.as_millis()) "ms" }
                " | "
                small class="muted" {
                    a href="https://github.com/encounter/decomp.dev" { "Code" }
                    " by "
                    a href="https://github.com/encounter" { "@encounter" }
                }
            }
            @if let Some(user) = current_user {
                span class="section" {
                    small class="muted" {
                        "Logged in as "
                        a href=(user.data.url) { "@" (user.data.login) }
                    }
                    " | "
                    form action="/logout" method="post" style="display: inline" {
                        input type="submit" class="button outline secondary" value="Logout";
                    }
                }
            } @else {
                span class="section" {
                    small class="muted" {
                        a href="/login" { "Login" }
                    }
                }
            }
        }
    }
}

pub fn nav_links() -> Markup {
    html! {
        ul {
            li {
                a href="https://ghidra.decomp.dev" { "Ghidra" }
            }
            li {
                a href="https://wiki.decomp.dev" { "Wiki" }
            }
            li {
                a #theme-toggle .icon-theme-light-dark href="#" title="Toggle theme" {}
            }
        }
    }
}

/// Format a size in bytes to a human-readable string.
/// Uses SI (kilo = 1000) units, formatted to two decimal places.
pub fn size(value: u64) -> String {
    let units = ["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    let mut value = value as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{:.2} {}", value, units[unit])
}

pub fn code_progress_sections(measures: &Measures) -> Markup {
    if measures.total_code == 0 {
        return Markup::default();
    }
    let mut out = Markup::default();
    let mut add_section = |class: &str, percent: f32, tooltip: String| {
        let rendered = html! {
            div class=(class) style=(format!("width: {}%", percent))
                data-tooltip=(tooltip) {}
        };
        out.0.push_str(&rendered.0);
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
    html! {
        div class="progress-root code" {
            (out)
        }
    }
}

pub fn data_progress_sections(measures: &Measures) -> Markup {
    if measures.total_data == 0 {
        return Markup::default();
    }
    let mut out = Markup::default();
    let mut add_section = |class: &str, percent: f32, tooltip: String| {
        let rendered = html! {
            div class=(class) style=(format!("width: {}%", percent))
                data-tooltip=(tooltip) {}
        };
        out.0.push_str(&rendered.0);
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
    html! {
        div class="progress-root data" {
            (out)
        }
    }
}

pub async fn get_robots() -> Result<String, AppError> {
    static ROBOTS_CACHE: LazyLock<std::sync::RwLock<Option<String>>> =
        LazyLock::new(|| std::sync::RwLock::new(None));
    {
        let cache =
            ROBOTS_CACHE.read().map_err(|_| AppError::Status(StatusCode::INTERNAL_SERVER_ERROR))?;
        if let Some(robots) = &*cache {
            return Ok(robots.clone());
        }
    }
    let response = reqwest::get(
        "https://raw.githubusercontent.com/ai-robots-txt/ai.robots.txt/refs/heads/main/robots.txt",
    )
    .await?;
    if response.status() != StatusCode::OK {
        return Err(AppError::Status(StatusCode::BAD_GATEWAY));
    }
    let text = response.text().await?;
    {
        let mut cache = ROBOTS_CACHE
            .write()
            .map_err(|_| AppError::Status(StatusCode::INTERNAL_SERVER_ERROR))?;
        *cache = Some(text.clone());
    }
    Ok(text)
}
