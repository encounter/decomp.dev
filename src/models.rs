use std::{borrow::Cow, sync::Arc};

use objdiff_core::bindings::report::{Measures, Report, ReportCategory, ReportUnit};
use serde::Serialize;
use time::UtcDateTime;

use crate::db::UnitKey;

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
}

impl Project {
    pub fn name(&self) -> Cow<str> {
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

impl From<&octocrab::models::workflows::HeadCommit> for Commit {
    fn from(commit: &octocrab::models::workflows::HeadCommit) -> Self {
        Self {
            sha: commit.id.clone(),
            timestamp: UtcDateTime::from_unix_timestamp(
                commit.timestamp.to_utc().timestamp_millis(),
            )
            .unwrap_or_else(|_| UtcDateTime::now()),
            message: (!commit.message.is_empty()).then(|| commit.message.clone()),
        }
    }
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
