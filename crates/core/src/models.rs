use std::{borrow::Cow, str::FromStr, sync::Arc};

use objdiff_core::bindings::report::{Measures, Report, ReportCategory, ReportUnit};
use serde::Serialize;
use time::UtcDateTime;

// BLAKE3 hash of the image data
pub type ImageId = [u8; 32];

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct Project {
    pub id: u64,
    pub owner: String,
    pub repo: String,
    pub name: Option<String>,
    pub short_name: Option<String>,
    pub default_category: Option<String>,
    pub default_version: Option<String>,
    pub platform: Option<String>,
    pub workflow_id: Option<String>,
    pub enable_pr_comments: bool,
    pub header_image_id: Option<ImageId>,
    pub enabled: bool,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: 0,
            owner: String::new(),
            repo: String::new(),
            name: None,
            short_name: None,
            default_category: None,
            default_version: None,
            platform: None,
            workflow_id: None,
            enable_pr_comments: true,
            header_image_id: None,
            enabled: true,
        }
    }
}

impl Project {
    pub fn name(&self) -> Cow<'_, str> {
        if let Some(name) = self.name.as_ref() {
            Cow::Borrowed(name)
        } else {
            Cow::Owned(format!("{}/{}", self.owner, self.repo))
        }
    }

    pub fn short_name(&self) -> &str {
        self.short_name.as_deref().or(self.name.as_deref()).unwrap_or(&self.repo)
    }

    pub fn repo_url(&self) -> String { format!("https://github.com/{}/{}", self.owner, self.repo) }

    pub fn default_category(&self) -> &str { self.default_category.as_deref().unwrap_or("all") }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ProjectInfo {
    pub project: Project,
    pub commit: Option<Commit>,
    pub report_versions: Vec<String>,
    pub prev_commit: Option<String>,
    pub next_commit: Option<String>,
}

impl ProjectInfo {
    pub fn default_version(&self) -> Option<&str> {
        self.project
            .default_version
            .as_ref()
            // Verify that the default version is in the list of report versions
            .and_then(|v| self.report_versions.contains(v).then_some(v.as_str()))
            // Otherwise, return the first version in the list
            .or_else(|| self.report_versions.first().map(String::as_str))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct Commit {
    pub sha: String,
    pub message: Option<String>,
    pub timestamp: UtcDateTime,
}

#[derive(Debug, Clone)]
pub struct ReportFile<R> {
    pub commit: Commit,
    pub version: String,
    pub report: R,
}

pub type CachedReportFile = ReportFile<Arc<CachedReport>>;
pub type FullReportFile = ReportFile<FullReport>;

#[derive(Debug, Clone)]
pub struct ReportInner<U> {
    pub version: u32,
    pub measures: Measures,
    pub units: Vec<U>,
    pub categories: Vec<ReportCategory>,
}

// BLAKE3 hash of the unit data
pub type UnitKey = [u8; 32];

pub type CachedReport = ReportInner<UnitKey>;
pub type FullReport = ReportInner<Arc<ReportUnit>>;

impl<U> ReportInner<U> {
    /// Fetch the measures for a specific category (None for the "all" category)
    pub fn measures(&self, category: Option<&str>) -> &Measures {
        if let Some(category) = category {
            self.categories
                .iter()
                .find(|c| c.id == category)
                .and_then(|c| c.measures.as_ref())
                .unwrap_or(&self.measures)
        } else {
            &self.measures
        }
    }
}

impl FullReport {
    /// Flatten the report into the standard format
    pub fn flatten(&self) -> Report {
        let mut units = Vec::with_capacity(self.units.len());
        for unit in &self.units {
            units.push((**unit).clone());
        }
        Report {
            measures: Some(self.measures),
            units,
            version: self.version,
            categories: self.categories.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FrogressMapping {
    pub frogress_slug: String,
    pub frogress_version: String,
    pub frogress_category: String,
    pub frogress_measure: String,
    pub project_id: u64,
    pub project_version: String,
    pub project_category: String,
    pub project_category_name: String,
    pub project_measure: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Platform {
    GBA,
    GBC,
    N64,
    DS,
    PS,
    PS2,
    Switch,
    GC,
    Wii,
}

pub const ALL_PLATFORMS: &[Platform] = &[
    Platform::GBA,
    Platform::GBC,
    Platform::N64,
    Platform::DS,
    Platform::PS,
    Platform::PS2,
    Platform::Switch,
    Platform::GC,
    Platform::Wii,
];

impl Platform {
    pub fn to_str(self) -> &'static str {
        match self {
            Self::GBA => "gba",
            Self::GBC => "gbc",
            Self::N64 => "n64",
            Self::DS => "nds",
            Self::PS => "ps",
            Self::PS2 => "ps2",
            Self::Switch => "switch",
            Self::GC => "gc",
            Self::Wii => "wii",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Platform::GBA => "Game Boy Advance",
            Platform::GBC => "Game Boy Color",
            Platform::N64 => "Nintendo 64",
            Platform::DS => "Nintendo DS",
            Platform::PS => "PlayStation",
            Platform::PS2 => "PlayStation 2",
            Platform::Switch => "Nintendo Switch",
            Platform::GC => "GameCube",
            Platform::Wii => "Wii",
        }
    }
}

impl FromStr for Platform {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "gba" => Ok(Self::GBA),
            "gbc" => Ok(Self::GBC),
            "n64" => Ok(Self::N64),
            "nds" => Ok(Self::DS),
            "ps" => Ok(Self::PS),
            "ps2" => Ok(Self::PS2),
            "switch" => Ok(Self::Switch),
            "gc" => Ok(Self::GC),
            "wii" => Ok(Self::Wii),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectVisibility {
    Visible,
    Hidden,
    Disabled,
}

pub fn project_visibility(project: &Project, measures: Option<&Measures>) -> ProjectVisibility {
    // Hide projects with less than 0.5% matched code or if the project is disabled
    if !project.enabled {
        ProjectVisibility::Disabled
    } else if measures.is_none_or(|m| m.matched_code_percent < 0.5) {
        ProjectVisibility::Hidden
    } else {
        ProjectVisibility::Visible
    }
}
