use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet, hash_map::Entry},
    convert::Infallible,
    sync::LazyLock,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponseParts, ResponseParts},
};
use decomp_dev_auth::CurrentUser;
use decomp_dev_core::{
    AppError,
    util::{format_percent, size},
};
use maud::{Markup, PreEscaped, Render, html};
use objdiff_core::bindings::report::Measures;
use regex::Regex;
use time::{UtcDateTime, macros::format_description};
use url::Url;

use crate::handlers::csp::{ExtraDomains, Nonce};

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

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebpackManifest {
    pub all_files: Vec<String>,
    pub entries: HashMap<String, WebpackManifestEntry>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebpackManifestEntry {
    pub initial: WebpackManifestEntryPaths,
    // pub r#async: WebpackManifestEntryPaths,
    // pub html: Vec<String>,
    pub assets: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebpackManifestEntryPaths {
    pub js: Vec<String>,
    pub css: Vec<String>,
}

pub struct TemplateContext {
    pub start: Instant,
    pub manifest: Option<WebpackManifest>,
    pub added_resources: HashMap<String, Load>,
    pub nonce: Option<String>,
}

impl<S> FromRequestParts<S> for TemplateContext
where S: Send + Sync
{
    type Rejection = Infallible;

    async fn from_request_parts(req: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let start = Instant::now();
        let nonce =
            match <Option<Nonce> as FromRequestParts<S>>::from_request_parts(req, _state).await {
                Ok(Some(nonce)) => Some(nonce.0),
                Ok(None) => None,
                Err(never) => match never {},
            };
        Ok(Self { start, manifest: None, added_resources: Default::default(), nonce })
    }
}

impl IntoResponseParts for TemplateContext {
    type Error = Infallible;

    fn into_response_parts(
        self,
        mut res: ResponseParts,
    ) -> std::result::Result<ResponseParts, Self::Error> {
        let mut origins = HashSet::new();
        if let Some(manifest) = &self.manifest {
            for entry in &manifest.all_files {
                if let Ok(url) = Url::parse(entry) {
                    origins.insert(url[..url::Position::BeforePath].to_string());
                }
            }
        }
        if !origins.is_empty() {
            res.extensions_mut().insert(ExtraDomains(origins.into_iter().collect()));
        }
        Ok(res)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Load {
    Blocking,
    Deferred,
    Preload,
}

impl TemplateContext {
    pub async fn header(&mut self) -> Markup {
        html! {
            meta name="viewport" content="width=device-width, initial-scale=1.0";
            meta name="color-scheme" content="dark light";
            meta name="darkreader-lock";
            meta name="apple-mobile-web-app-title" content="decomp.dev";
            meta name="theme-color" content="#181c25" media="(prefers-color-scheme: dark)";
            meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)";
            link rel="icon" type="image/png" href="/favicon.png" sizes="96x96";
            link rel="icon" type="image/svg+xml" href="/favicon.svg";
            link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180";
            link rel="manifest" href="/site.webmanifest";
            (self.chunks("entry", Load::Blocking).await)
        }
    }

    async fn manifest_paths(&mut self, entry: &str) -> Result<WebpackManifestEntry> {
        let manifest = match self.manifest.as_ref() {
            Some(manifest) => manifest,
            None => {
                let manifest_str = tokio::fs::read_to_string("dist/manifest.json").await?;
                let manifest: WebpackManifest = serde_json::from_str(&manifest_str)?;
                self.manifest.insert(manifest)
            }
        };
        let entry = manifest
            .entries
            .get(entry)
            .ok_or_else(|| anyhow!("Entry {} not found in manifest", entry))?;
        // Asset paths are relative, resolve the full paths
        let assets = entry
            .assets
            .iter()
            .filter_map(|p| manifest.all_files.iter().find(|a| a.ends_with(p)).cloned())
            .collect();
        Ok(WebpackManifestEntry { initial: entry.initial.clone(), assets })
    }

    pub async fn chunks(&mut self, name: &str, load: Load) -> Markup {
        let entry = self.manifest_paths(name).await.unwrap_or_else(|e| {
            tracing::error!("Failed to load chunks for {name}: {e}");
            Default::default()
        });
        let mut out = String::new();
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        enum ResourceKind {
            Script,
            Style,
            Font,
        }
        let mut push = async |kind: ResourceKind, path: &str, load: Load| {
            match load {
                Load::Preload => {
                    if self.added_resources.contains_key(path) {
                        return;
                    }
                }
                Load::Deferred => match self.added_resources.entry(path.to_string()) {
                    Entry::Occupied(mut e) => match *e.get() {
                        Load::Blocking | Load::Deferred => return,
                        Load::Preload => {
                            e.insert(load);
                        }
                    },
                    Entry::Vacant(e) => {
                        e.insert(load);
                    }
                },
                Load::Blocking => match self.added_resources.entry(path.to_string()) {
                    Entry::Occupied(mut e) => match *e.get() {
                        Load::Blocking => return,
                        Load::Deferred => panic!("Resource {path} is already loaded as Deferred"),
                        Load::Preload => {
                            e.insert(load);
                        }
                    },
                    Entry::Vacant(e) => {
                        e.insert(load);
                    }
                },
            }
            // Fonts must be preloaded with crossorigin
            let crossorigin = path.starts_with("http://")
                || path.starts_with("https://")
                || kind == ResourceKind::Font;
            let nonce = match kind {
                ResourceKind::Script | ResourceKind::Style => self.nonce.as_deref(),
                _ => None,
            };
            let can_inline = path.starts_with('/')
                && matches!(kind, ResourceKind::Script | ResourceKind::Style)
                && tokio::fs::metadata(format!("dist{path}"))
                    .await
                    .is_ok_and(|m| m.is_file() && m.len() <= 2048);
            let rendered = match load {
                Load::Preload if !can_inline => html! {
                    link rel="preload" href=(path) as=(match kind {
                        ResourceKind::Script => "script",
                        ResourceKind::Style => "style",
                        ResourceKind::Font => "font",
                    }) crossorigin[crossorigin] nonce=[nonce];
                },
                Load::Preload => Markup::default(),
                Load::Blocking | Load::Deferred if can_inline => {
                    let content =
                        tokio::fs::read_to_string(format!("dist{path}")).await.unwrap_or_default();
                    let content = escape_script(&content);
                    match kind {
                        ResourceKind::Script => html! {
                            script type=[(load == Load::Deferred).then_some("module")] nonce=[nonce] { (content) }
                        },
                        ResourceKind::Style => html! {
                            style nonce=[nonce] { (content) }
                        },
                        ResourceKind::Font => Markup::default(),
                    }
                }
                Load::Blocking | Load::Deferred => match kind {
                    ResourceKind::Script => html! {
                        script src=(path) defer[load == Load::Deferred] crossorigin[crossorigin] nonce=[nonce] {};
                    },
                    ResourceKind::Style => html! {
                        link rel="stylesheet" href=(path) crossorigin[crossorigin] nonce=[nonce];
                    },
                    ResourceKind::Font => Markup::default(),
                },
            };
            out.push_str(&rendered.0);
        };
        for path in &entry.initial.js {
            push(ResourceKind::Script, path, load).await;
        }
        for path in &entry.initial.css {
            push(ResourceKind::Style, path, load).await;
        }
        for path in &entry.assets {
            if path.ends_with(".woff2") {
                push(ResourceKind::Font, path, Load::Preload).await;
            }
        }
        PreEscaped(out)
    }

    pub fn footer(&self, current_user: Option<&CurrentUser>) -> Markup {
        let elapsed = self.start.elapsed();
        html! {
            footer {
                span.section {
                    small.muted { "Generated in " (elapsed.as_millis()) "ms" }
                    " | "
                    small.muted {
                        a href="https://github.com/encounter/decomp.dev" { "Code" }
                        " by "
                        a href="https://github.com/encounter" { "@encounter" }
                    }
                }
                @if let Some(user) = current_user {
                    span.section {
                        small.muted {
                            "Logged in as "
                            a href=(user.data.url) { "@" (user.data.login) }
                        }
                        " | "
                        form action="/logout" method="post" {
                            input.button.outline.secondary type="submit" value="Logout";
                        }
                    }
                } @else {
                    span.section {
                        small.muted {
                            a href="/login" { "Login" }
                        }
                    }
                }
            }
            // Analytics
            script src="https://umami.decomp.dev/script.js" defer crossorigin
                nonce=[self.nonce.as_deref()]
                data-website-id="80d6109f-52cc-42a9-98c9-9820dc8c4435" {};
        }
    }

    pub fn code_progress_sections(&self, measures: &Measures) -> ProgressSections {
        let mut out = ProgressSections::new("code", self.nonce.clone());
        if measures.total_code == 0 {
            return out;
        }
        let mut current_percent = 0.0;
        if measures.complete_code_percent > 0.0 {
            out.push(
                "progress-section",
                measures.complete_code_percent - current_percent,
                format!(
                    "{} fully linked ({})",
                    format_percent(measures.complete_code_percent),
                    size(measures.complete_code)
                ),
            );
            current_percent = measures.complete_code_percent;
        }
        if measures.matched_code_percent > 0.0 && measures.matched_code_percent > current_percent {
            out.push(
                if current_percent > 0.0 { "progress-section striped" } else { "progress-section" },
                measures.matched_code_percent - current_percent,
                format!(
                    "{} perfect match ({})",
                    format_percent(measures.matched_code_percent),
                    size(measures.matched_code)
                ),
            );
            current_percent = measures.matched_code_percent;
        }
        if measures.fuzzy_match_percent > 0.0 && measures.fuzzy_match_percent > current_percent {
            out.push(
                "progress-section striped fuzzy",
                measures.fuzzy_match_percent - current_percent,
                format!("{} fuzzy match", format_percent(measures.fuzzy_match_percent)),
            );
        }
        out
    }

    pub fn data_progress_sections(&self, measures: &Measures) -> ProgressSections {
        let mut out = ProgressSections::new("data", self.nonce.clone());
        if measures.total_data == 0 {
            return out;
        }
        let mut current_percent = 0.0;
        if measures.complete_data_percent > 0.0 {
            out.push(
                "progress-section",
                measures.complete_data_percent - current_percent,
                format!(
                    "{} fully linked ({})",
                    format_percent(measures.complete_data_percent),
                    size(measures.complete_data)
                ),
            );
            current_percent = measures.complete_data_percent;
        }
        if measures.matched_data_percent > 0.0 && measures.matched_data_percent > current_percent {
            out.push(
                if current_percent > 0.0 { "progress-section striped" } else { "progress-section" },
                measures.matched_data_percent - current_percent,
                format!(
                    "{} perfect match ({})",
                    format_percent(measures.matched_data_percent),
                    size(measures.matched_data)
                ),
            );
        }
        out
    }
}

#[derive(Default)]
pub struct ProgressSections {
    pub width_classes: BTreeMap<String, String>,
    pub rendered: Markup,
    pub kind: &'static str,
    pub nonce: Option<String>,
}

impl ProgressSections {
    pub fn new(kind: &'static str, nonce: Option<String>) -> Self {
        Self { width_classes: BTreeMap::new(), rendered: Markup::default(), kind, nonce }
    }

    pub fn push(&mut self, class: &str, percent: f32, tooltip: String) {
        let width_class = format!("width-{percent:.2}").replace('.', "p");
        let rendered = html! { .(class).(width_class) data-tooltip=(tooltip) {} };
        self.width_classes.insert(width_class, format!("width:{percent}%"));
        self.rendered.0.push_str(&rendered.0);
    }
}

impl Render for ProgressSections {
    fn render(&self) -> Markup {
        let mut out = Markup::default();
        if !self.width_classes.is_empty() {
            let rendered = html! {
                style nonce=[self.nonce.as_deref()] {
                    @for (class, width) in &self.width_classes {
                        "." (class) "{" (width) "}"
                    }
                }
            };
            out.0.push_str(&rendered.0);
        }
        if self.kind.is_empty() {
            return out;
        }
        html! {
            (out)
            .progress-root.(self.kind) { (self.rendered) }
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
                a href="https://decomp.wiki" { "Wiki" }
            }
            li {
                a #theme-toggle .icon-theme-light-dark href="#" title="Toggle theme" {}
            }
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

pub fn escape_script(text: &str) -> PreEscaped<Cow<'_, str>> {
    static REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new("<(?i:!--|/?script)").unwrap());
    PreEscaped(REGEX.replace_all(text, |caps: &regex::Captures| format!("\\x3C{}", &caps[0][1..])))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_escape_script() {
        let input = r#"<!--<SCRIPT>alert('test');</ScRiPt>-->"#;
        let expected = "\\x3C!--\\x3CSCRIPT>alert('test');\\x3C/ScRiPt>-->";
        assert_eq!(escape_script(input).0.as_ref(), expected);
    }
}
