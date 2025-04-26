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
                a #theme-toggle href="#" title="Toggle theme" {
                    (PreEscaped(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="1.33em" height="1.33em">
    <path d="M7.5,2C5.71,3.15 4.5,5.18 4.5,7.5C4.5,9.82 5.71,11.85 7.53,13C4.46,13 2,10.54 2,7.5A5.5,5.5 0 0,1 7.5,2M19.07,3.5L20.5,4.93L4.93,20.5L3.5,19.07L19.07,3.5M12.89,5.93L11.41,5L9.97,6L10.39,4.3L9,3.24L10.75,3.12L11.33,1.47L12,3.1L13.73,3.13L12.38,4.26L12.89,5.93M9.59,9.54L8.43,8.81L7.31,9.59L7.65,8.27L6.56,7.44L7.92,7.35L8.37,6.06L8.88,7.33L10.24,7.36L9.19,8.23L9.59,9.54M19,13.5A5.5,5.5 0 0,1 13.5,19C12.28,19 11.15,18.6 10.24,17.93L17.93,10.24C18.6,11.15 19,12.28 19,13.5M14.6,20.08L17.37,18.93L17.13,22.28L14.6,20.08M18.93,17.38L20.08,14.61L22.28,17.15L18.93,17.38M20.08,12.42L18.94,9.64L22.28,9.88L20.08,12.42M9.63,18.93L12.4,20.08L9.87,22.27L9.63,18.93Z" />
</svg>"#))
                }
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
