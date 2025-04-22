use std::cmp::Ordering;

use anyhow::Result;
use decomp_dev_core::models::{Commit, FullReportFile};
use objdiff_core::bindings::report::{
    ChangeItem, ChangeItemInfo, ChangeUnit, Changes, Report, ReportItem, ReportUnit,
};

use crate::ProcessArtifactResult;

pub fn generate_changes(previous: &Report, current: &Report) -> Result<Changes> {
    let mut changes = Changes { from: previous.measures, to: current.measures, units: vec![] };
    for prev_unit in &previous.units {
        let curr_unit = current.units.iter().find(|u| u.name == prev_unit.name);
        let sections = process_items(prev_unit, curr_unit, |u| &u.sections);
        let functions = process_items(prev_unit, curr_unit, |u| &u.functions);

        let prev_measures = prev_unit.measures;
        let curr_measures = curr_unit.and_then(|u| u.measures);
        if !functions.is_empty() || prev_measures != curr_measures {
            changes.units.push(ChangeUnit {
                name: prev_unit.name.clone(),
                from: prev_measures,
                to: curr_measures,
                sections,
                functions,
                metadata: curr_unit
                    .as_ref()
                    .and_then(|u| u.metadata.clone())
                    .or_else(|| prev_unit.metadata.clone()),
            });
        }
    }
    for curr_unit in &current.units {
        if !previous.units.iter().any(|u| u.name == curr_unit.name) {
            changes.units.push(ChangeUnit {
                name: curr_unit.name.clone(),
                from: None,
                to: curr_unit.measures,
                sections: process_new_items(&curr_unit.sections),
                functions: process_new_items(&curr_unit.functions),
                metadata: curr_unit.metadata.clone(),
            });
        }
    }
    Ok(changes)
}

fn process_items<F: Fn(&ReportUnit) -> &Vec<ReportItem>>(
    prev_unit: &ReportUnit,
    curr_unit: Option<&ReportUnit>,
    getter: F,
) -> Vec<ChangeItem> {
    let prev_items = getter(prev_unit);
    let mut items = vec![];
    if let Some(curr_unit) = curr_unit {
        let curr_items = getter(curr_unit);
        for prev_func in prev_items {
            let prev_func_info = ChangeItemInfo::from(prev_func);
            let prev_func_address = prev_func.metadata.as_ref().and_then(|m| m.virtual_address);
            let curr_func = curr_items.iter().find(|f| {
                f.name == prev_func.name
                    || prev_func_address.is_some_and(|a| {
                        f.metadata.as_ref().and_then(|m| m.virtual_address).is_some_and(|b| a == b)
                    })
            });
            if let Some(curr_func) = curr_func {
                let curr_func_info = ChangeItemInfo::from(curr_func);
                if prev_func_info != curr_func_info {
                    items.push(ChangeItem {
                        name: curr_func.name.clone(),
                        from: Some(prev_func_info),
                        to: Some(curr_func_info),
                        metadata: curr_func.metadata.clone(),
                    });
                }
            } else {
                items.push(ChangeItem {
                    name: prev_func.name.clone(),
                    from: Some(prev_func_info),
                    to: None,
                    metadata: prev_func.metadata.clone(),
                });
            }
        }
        for curr_func in curr_items {
            let curr_func_address = curr_func.metadata.as_ref().and_then(|m| m.virtual_address);
            if !prev_items.iter().any(|f| {
                f.name == curr_func.name
                    || curr_func_address.is_some_and(|a| {
                        f.metadata.as_ref().and_then(|m| m.virtual_address).is_some_and(|b| a == b)
                    })
            }) {
                items.push(ChangeItem {
                    name: curr_func.name.clone(),
                    from: None,
                    to: Some(ChangeItemInfo::from(curr_func)),
                    metadata: curr_func.metadata.clone(),
                });
            }
        }
    } else {
        for prev_func in prev_items {
            items.push(ChangeItem {
                name: prev_func.name.clone(),
                from: Some(ChangeItemInfo::from(prev_func)),
                to: None,
                metadata: prev_func.metadata.clone(),
            });
        }
    }
    items
}

fn process_new_items(items: &[ReportItem]) -> Vec<ChangeItem> {
    items
        .iter()
        .map(|item| ChangeItem {
            name: item.name.clone(),
            from: None,
            to: Some(ChangeItemInfo::from(item)),
            metadata: item.metadata.clone(),
        })
        .collect()
}

fn measure_line_matched(
    name: &str,
    from: u64,
    from_percent: f32,
    to: u64,
    to_percent: f32,
) -> String {
    let emoji = if to > from { "📈" } else { "📉" };
    let percent_diff = to_percent - from_percent;
    let percent_str = if percent_diff < 0.0 {
        format!("{percent_diff:.2}%")
    } else {
        format!("+{percent_diff:.2}%")
    };
    let bytes_diff = to as i64 - from as i64;
    let bytes_str = match bytes_diff.cmp(&0) {
        Ordering::Less => bytes_diff.to_string(),
        Ordering::Equal | Ordering::Greater => format!("+{bytes_diff}"),
    };
    format!("{emoji} **{name}**: {to_percent:.2}% ({percent_str}, {bytes_str} bytes)\n")
}

fn measure_line_bytes(name: &str, from: u64, to: u64) -> String {
    let diff = to as i64 - from as i64;
    let diff_str = match diff.cmp(&0) {
        Ordering::Less => diff.to_string(),
        Ordering::Equal | Ordering::Greater => format!("+{diff}"),
    };
    format!("**{name}**: {to} bytes ({diff_str} bytes)\n")
}

fn measure_line_simple(name: &str, from: u64, to: u64) -> String {
    let diff = to as i64 - from as i64;
    let diff_str = match diff.cmp(&0) {
        Ordering::Less => diff.to_string(),
        Ordering::Equal | Ordering::Greater => format!("+{diff}"),
    };
    format!("**{name}**: {to} ({diff_str})\n")
}

pub fn generate_comment(
    from: &FullReportFile,
    to: &ProcessArtifactResult,
    to_commit: &Commit,
    changes: Changes,
) -> String {
    let mut comment = format!(
        "### Report for {} ({} - {})\n\n",
        to.version,
        &from.commit.sha[..7],
        &to_commit.sha[..7]
    );
    let mut measure_written = false;
    let from_measures = from.report.measures;
    let to_measures = to.report.measures.unwrap_or_default();
    if from_measures.total_code != to_measures.total_code {
        comment.push_str(&measure_line_bytes(
            "Total code",
            from_measures.total_code,
            to_measures.total_code,
        ));
        measure_written = true;
    }
    if from_measures.total_functions != to_measures.total_functions {
        comment.push_str(&measure_line_simple(
            "Total functions",
            from_measures.total_functions as u64,
            to_measures.total_functions as u64,
        ));
        measure_written = true;
    }
    if from_measures.matched_code != to_measures.matched_code {
        comment.push_str(&measure_line_matched(
            "Matched code",
            from_measures.matched_code,
            from_measures.matched_code_percent,
            to_measures.matched_code,
            to_measures.matched_code_percent,
        ));
        measure_written = true;
    }
    if from_measures.complete_code != to_measures.complete_code {
        comment.push_str(&measure_line_matched(
            "Linked code",
            from_measures.complete_code,
            from_measures.complete_code_percent,
            to_measures.complete_code,
            to_measures.complete_code_percent,
        ));
        measure_written = true;
    }
    if measure_written {
        comment.push('\n');
    }
    let mut total_changes = 0;
    let mut iter = changes.units.into_iter().flat_map(|mut unit| {
        let functions = core::mem::take(&mut unit.functions);
        functions.into_iter().map(move |f| (unit.clone(), f))
    });
    for (unit, item) in iter.by_ref() {
        let (from, to) = match (item.from, item.to) {
            (Some(from), Some(to)) => (from, to),
            (None, Some(to)) => (ChangeItemInfo::default(), to),
            (Some(from), None) => (from, ChangeItemInfo::default()),
            (None, None) => continue,
        };
        let emoji = if to.fuzzy_match_percent == 100.0 {
            "✅"
        } else if to.fuzzy_match_percent > from.fuzzy_match_percent {
            "📈"
        } else {
            "📉"
        };
        let from_bytes = ((from.fuzzy_match_percent as f64 / 100.0) * from.size as f64) as u64;
        let to_bytes = ((to.fuzzy_match_percent as f64 / 100.0) * to.size as f64) as u64;
        let bytes_diff = to_bytes as i64 - from_bytes as i64;
        let bytes_str = match bytes_diff.cmp(&0) {
            Ordering::Less => bytes_diff.to_string(),
            Ordering::Equal => "0".to_string(),
            Ordering::Greater => format!("+{}", bytes_diff),
        };
        let name =
            item.metadata.as_ref().and_then(|m| m.demangled_name.as_deref()).unwrap_or(&item.name);
        comment.push_str(&format!(
            "{emoji} `{} | {}` {} bytes -> {:.2}%\n",
            unit.name, name, bytes_str, to.fuzzy_match_percent
        ));
        total_changes += 1;
        if total_changes >= 30 {
            break;
        }
    }
    let remaining = iter.count();
    if remaining > 0 {
        comment.push_str(&format!("...and {} more items\n", remaining));
    } else if total_changes == 0 {
        comment.push_str("No changes\n");
    }
    comment
}

#[allow(unused)]
fn platform_name(platform: &str) -> &str {
    match platform {
        "gc" => "GameCube",
        "wii" => "Wii",
        "n64" => "Nintendo 64",
        "switch" => "Nintendo Switch",
        "3ds" => "Nintendo 3DS",
        "nds" => "Nintendo DS",
        "gba" => "Game Boy Advance",
        "gbc" => "Game Boy Color",
        "ps" => "PlayStation",
        "ps2" => "PlayStation 2",
        _ => platform,
    }
}
