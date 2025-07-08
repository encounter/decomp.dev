#![allow(unused)]
use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use decomp_dev_core::models::Commit;
use itertools::Itertools;
use objdiff_core::bindings::report::{Measures, Report, ReportCategory};
use time::UtcDateTime;
use tracing::log;

use crate::AppState;

type FrogressCategory<T> = HashMap<String, T>;
type FrogressVersion<T> = HashMap<String, T>;

#[derive(Debug, serde::Deserialize)]
struct FrogressEntry {
    pub timestamp: u64,
    pub git_hash: String,
    pub description: String,
    pub measures: FrogressMeasures,
}

type FrogressMeasures = HashMap<String, u64>;

type FrogressAllData = HashMap<String, FrogressVersion<FrogressCategory<Vec<FrogressEntry>>>>;

pub async fn migrate_data(state: &mut AppState) -> Result<()> {
    let mappings = state.db.get_frogress_mappings().await?;
    let version_chunks = mappings.iter().chunk_by(|m| {
        (
            m.frogress_slug.as_str(),
            m.frogress_version.as_str(),
            m.project_id,
            m.project_version.as_str(),
        )
    });
    for ((slug, version, project_id, project_version), version_group) in &version_chunks {
        let project = state
            .db
            .get_project_info_by_id(project_id, None)
            .await?
            .ok_or_else(|| anyhow!("No project ID {}", project_id))?;
        let data =
            reqwest::get(&format!("https://progress.decomp.club/data/{slug}/{version}?mode=all"))
                .await?;
        let data: FrogressAllData = data.json().await?;
        let mut data_iter = data.iter();
        let (data_slug, version_data) =
            data_iter.next().ok_or_else(|| anyhow::anyhow!("No data for {}/{}", slug, version))?;
        if data_slug != slug {
            bail!("Slug mismatch for {}/{}: expected {}, got {}", slug, version, slug, data_slug);
        }
        if data_iter.next().is_some() {
            bail!("Unexpected data for {}/{}", slug, version);
        }
        let mut version_data_iter = version_data.iter();
        let (data_version, category_data) = version_data_iter
            .next()
            .ok_or_else(|| anyhow::anyhow!("No version data for {}/{}", slug, version))?;
        if data_version != version {
            bail!(
                "Version mismatch for {}/{}: expected {}, got {}",
                slug,
                version,
                version,
                data_version
            );
        }
        if version_data_iter.next().is_some() {
            bail!("Unexpected version data for {}/{}", slug, version);
        }
        let mut reports = HashMap::<String, Report>::new(); // keyed by git sha
        let mut commits = HashMap::<String, Commit>::new();
        let category_chunks = version_group.chunk_by(|m| m.frogress_category.as_str());
        for (category, mappings) in &category_chunks {
            let mappings = mappings.collect::<Vec<_>>();
            let entries = category_data
                .get(category)
                .ok_or_else(|| anyhow::anyhow!("No data for {}/{}/{}", slug, version, category))?;
            tracing::info!(
                "Migrating {}/{}/{}: {} entries",
                slug,
                version,
                category,
                entries.len()
            );
            for entry in entries {
                if state.db.report_exists(project_id, &entry.git_hash).await? {
                    tracing::info!(
                        "Skipping {}/{}/{}: {} ({})",
                        slug,
                        version,
                        category,
                        entry.git_hash,
                        entry.timestamp
                    );
                    continue;
                }
                let timestamp = UtcDateTime::from_unix_timestamp(entry.timestamp as i64)
                    .with_context(|| {
                        format!(
                            "Invalid timestamp {} for {}/{}/{}",
                            entry.timestamp, slug, version, category
                        )
                    })?;
                let commit = commits.entry(entry.git_hash.clone()).or_insert_with(|| Commit {
                    sha: entry.git_hash.clone(),
                    message: (!entry.description.is_empty()).then(|| entry.description.clone()),
                    timestamp,
                });
                if timestamp < commit.timestamp {
                    // Use the earliest timestamp
                    commit.timestamp = timestamp;
                }
                let report = reports.entry(entry.git_hash.clone()).or_default();
                for mapping in &mappings {
                    let Some(value) = entry.measures.get(&mapping.frogress_measure).cloned() else {
                        tracing::warn!(
                            "Missing measure {} for {}/{}/{}",
                            mapping.frogress_measure,
                            slug,
                            version,
                            category
                        );
                        continue;
                    };
                    let Some(total) =
                        entry.measures.get(&format!("{}/total", mapping.frogress_measure)).cloned()
                    else {
                        tracing::warn!(
                            "Missing total measure {}/total for {}/{}/{}",
                            mapping.frogress_measure,
                            slug,
                            version,
                            category
                        );
                        continue;
                    };
                    let percent =
                        if total == 0 { 100.0 } else { (value as f32 / total as f32) * 100.0 };
                    let measures = if mapping.project_category == "all" {
                        report.measures.get_or_insert_with(Default::default)
                    } else {
                        let mut category = report
                            .categories
                            .iter_mut()
                            .find(|c| c.name == mapping.project_category);
                        if category.is_none() {
                            report.categories.push(ReportCategory {
                                id: mapping.project_category.clone(),
                                name: mapping.project_category_name.clone(),
                                measures: None,
                            });
                            category = report.categories.last_mut();
                        }
                        let category = category.unwrap();
                        category.measures.get_or_insert_with(Default::default)
                    };
                    match mapping.project_measure.as_str() {
                        "matched_code" => {
                            measures.total_code = total;
                            measures.matched_code = value;
                            measures.matched_code_percent = percent;
                        }
                        "matched_data" => {
                            measures.total_data = total;
                            measures.matched_data = value;
                            measures.matched_data_percent = percent;
                        }
                        "matched_functions" => {
                            measures.total_functions = total as u32;
                            measures.matched_functions = value as u32;
                            measures.matched_functions_percent = percent;
                        }
                        "complete_code" => {
                            measures.total_code = total;
                            measures.complete_code = value;
                            measures.complete_code_percent = percent;
                        }
                        "complete_data" => {
                            measures.total_data = total;
                            measures.complete_data = value;
                            measures.complete_data_percent = percent;
                        }
                        "complete_units" => {
                            measures.total_units = total as u32;
                            measures.complete_units = value as u32;
                        }
                        measure => {
                            tracing::error!("Unknown measure {}", measure);
                        }
                    }
                }
                if report.measures.is_none() {
                    if report.categories.is_empty() {
                        log::warn!("No measures for {slug}/{version}/{category}");
                        reports.remove(&entry.git_hash);
                    } else {
                        let mut all_measures = Measures::default();
                        for category in &report.categories {
                            let measures = category.measures.as_ref().unwrap();
                            all_measures += *measures;
                        }
                        all_measures.calc_matched_percent();
                        report.measures = Some(all_measures);
                    }
                }
            }
        }
        tracing::info!("Generated {} reports", reports.len());
        for (git_hash, report) in reports {
            let commit = commits.get(&git_hash).unwrap();
            state.db.insert_report(&project.project, commit, project_version, report).await?;
        }
    }
    Ok(())
}
