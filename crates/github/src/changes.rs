use std::{cmp::Ordering, collections::BTreeMap};

use anyhow::Result;
use decomp_dev_core::{models::Commit, util::format_percent};
use objdiff_core::bindings::report::{
    ChangeItem, ChangeItemInfo, ChangeUnit, Changes, Report, ReportItem, ReportUnit,
};

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
    let to_percent_str = format_percent(to_percent);
    let percent_diff = to_percent - from_percent;
    let percent_diff_str = if percent_diff < 0.0 {
        format!("{percent_diff:.2}%")
    } else {
        format!("+{percent_diff:.2}%")
    };
    let bytes_diff = to as i64 - from as i64;
    let bytes_str = match bytes_diff.cmp(&0) {
        Ordering::Less => bytes_diff.to_string(),
        Ordering::Equal | Ordering::Greater => format!("+{bytes_diff}"),
    };
    format!("{emoji} **{name}**: {to_percent_str} ({percent_diff_str}, {bytes_str} bytes)\n")
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

const MAX_CHANGE_LINES: usize = 30;

// Note: The order the tables are printed in is determined by the order of the variants in this enum.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug)]
enum ChangeKind {
    NewMatch,
    BrokenMatch,
    Improvement,
    Regression,
}

impl ChangeKind {
    fn emoji(self) -> &'static str {
        match self {
            ChangeKind::NewMatch => "✅",
            ChangeKind::BrokenMatch => "💔",
            ChangeKind::Improvement => "📈",
            ChangeKind::Regression => "📉",
        }
    }

    fn singular_description(self) -> &'static str {
        match self {
            ChangeKind::NewMatch => "new match",
            ChangeKind::BrokenMatch => "broken match",
            ChangeKind::Improvement => "improvement in unmatched functions",
            ChangeKind::Regression => "regression in unmatched functions",
        }
    }

    fn plural_description(self) -> &'static str {
        match self {
            ChangeKind::NewMatch => "new matches",
            ChangeKind::BrokenMatch => "broken matches",
            ChangeKind::Improvement => "improvements in unmatched functions",
            ChangeKind::Regression => "regressions in unmatched functions",
        }
    }
}

struct ChangeLine {
    kind: ChangeKind,
    unit_name: String,
    item_name: String,
    from_fuzzy_match_percent: f32,
    to_fuzzy_match_percent: f32,
    bytes_diff: i64,
}

fn output_line(line: &ChangeLine, out: &mut String) {
    let bytes_str = match line.bytes_diff.cmp(&0) {
        Ordering::Less => line.bytes_diff.to_string(),
        Ordering::Equal => "0".to_string(),
        Ordering::Greater => format!("+{}", line.bytes_diff),
    };

    out.push_str(&format!(
        "| `{}` | `{}` | {} | {} | {} |\n",
        line.unit_name,
        line.item_name,
        bytes_str,
        format_percent(line.from_fuzzy_match_percent),
        format_percent(line.to_fuzzy_match_percent),
    ));
}

fn generate_changes_list(changes: Vec<ChangeLine>, out: &mut String) {
    let mut changes_by_kind = BTreeMap::new();
    for change in changes {
        changes_by_kind.entry(change.kind.clone()).or_insert(vec![]).push(change);
    }
    for (change_kind, mut changes) in changes_by_kind {
        let total_changes = changes.len();
        let description = if total_changes == 0 {
            out.push_str(&format!("No {}.\n", change_kind.plural_description()));
            continue;
        } else if total_changes == 1 {
            change_kind.singular_description()
        } else {
            change_kind.plural_description()
        };

        if change_kind == ChangeKind::BrokenMatch {
            out.push_str("<details open>\n");
        } else {
            out.push_str("<details>\n");
        }
        out.push_str(&format!(
            "<summary>{} {total_changes} {description}</summary>\n",
            change_kind.emoji()
        ));
        out.push('\n'); // Must include a blank line before a table
        out.push_str("| Unit | Function | Bytes | Before | After |\n");
        out.push_str("| - | - | - | - | - |\n");

        // Sort to show the biggest changes first.
        match change_kind {
            ChangeKind::NewMatch | ChangeKind::Improvement => {
                changes.sort_by_key(|item| -item.bytes_diff)
            }
            ChangeKind::BrokenMatch | ChangeKind::Regression => {
                changes.sort_by_key(|item| item.bytes_diff)
            }
        }

        let mut shown_changes = 0;
        for line in changes.iter().take(MAX_CHANGE_LINES) {
            output_line(line, out);
            shown_changes += 1;
        }

        out.push('\n'); // Must include a blank line after a table

        let remaining = total_changes - shown_changes;
        if remaining > 0 {
            out.push_str(&format!("...and {remaining} more {description}\n"));
        }
        out.push_str("</details>\n");
        out.push('\n');
    }
}

pub fn generate_missing_report_comment(
    version: &str,
    from_commit: Option<&Commit>,
    to_commit: Option<&Commit>,
) -> String {
    format!(
        "### Report for {} ({} - {})\n\n[!] Report not found. Did the build succeed?\n\n",
        version,
        from_commit.map_or("<none>", |c| &c.sha[..7]),
        to_commit.map_or("<none>", |c| &c.sha[..7])
    )
}

pub fn generate_combined_comment(version_comments: Vec<String>) -> String {
    version_comments.join("---\n\n")
}

pub fn generate_comment(
    from: &Report,
    to: &Report,
    version: Option<&str>,
    from_commit: Option<&Commit>,
    to_commit: Option<&Commit>,
    changes: Changes,
) -> String {
    let mut comment = format!(
        "### Report for {} ({} - {})\n\n",
        version.unwrap_or("unknown"),
        from_commit.map_or("<none>", |c| &c.sha[..7]),
        to_commit.map_or("<none>", |c| &c.sha[..7])
    );
    let mut measure_written = false;
    let from_measures = from.measures.unwrap_or_default();
    let to_measures = to.measures.unwrap_or_default();
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
    let mut iter = changes.units.into_iter().flat_map(|mut unit| {
        let functions = core::mem::take(&mut unit.functions);
        functions.into_iter().map(move |f| (unit.clone(), f))
    });

    let mut changes = vec![];

    for (unit, item) in iter.by_ref() {
        let (from, to) = match (item.from, item.to) {
            (Some(from), Some(to)) => (from, to),
            (None, Some(to)) => (ChangeItemInfo::default(), to),
            (Some(from), None) => (from, ChangeItemInfo::default()),
            (None, None) => continue,
        };
        let kind = if to.fuzzy_match_percent == 100.0 {
            ChangeKind::NewMatch
        } else if from.fuzzy_match_percent == 100.0 {
            ChangeKind::BrokenMatch
        } else if to.fuzzy_match_percent > from.fuzzy_match_percent {
            ChangeKind::Improvement
        } else if from.fuzzy_match_percent > 0.0 {
            ChangeKind::Regression
        } else {
            continue; // No change
        };
        let from_bytes = ((from.fuzzy_match_percent as f64 / 100.0) * from.size as f64) as u64;
        let to_bytes = ((to.fuzzy_match_percent as f64 / 100.0) * to.size as f64) as u64;
        let bytes_diff = to_bytes as i64 - from_bytes as i64;
        let name =
            item.metadata.as_ref().and_then(|m| m.demangled_name.as_deref()).unwrap_or(&item.name);

        let change = ChangeLine {
            kind,
            unit_name: unit.name.to_owned(),
            item_name: name.to_owned(),
            bytes_diff,
            from_fuzzy_match_percent: from.fuzzy_match_percent,
            to_fuzzy_match_percent: to.fuzzy_match_percent,
        };

        changes.push(change);
    }
    if !changes.is_empty() {
        generate_changes_list(changes, &mut comment);
    } else {
        comment.push_str("No changes\n");
    }
    comment
}

#[cfg(test)]
mod tests {
    use decomp_dev_core::models::Commit;
    use time::UtcDateTime;

    use super::*;

    #[test]
    fn test_generate_missing_report_comment() {
        let commit = Commit {
            sha: "abc1234567890".to_string(),
            message: Some("Test commit".to_string()),
            timestamp: UtcDateTime::UNIX_EPOCH,
        };
        let comment = generate_missing_report_comment("GALE01", Some(&commit), Some(&commit));
        assert_eq!(
            comment,
            "### Report for GALE01 (abc1234 - abc1234)\n\n[!] Report not found. Did the build succeed?\n\n"
        );
    }

    #[test]
    fn test_commit_sha_truncation() {
        let long_commit = Commit {
            sha: "abcdef1234567890abcdef1234567890abcdef12".to_string(),
            message: Some("Long commit SHA".to_string()),
            timestamp: UtcDateTime::UNIX_EPOCH,
        };
        let comment =
            generate_missing_report_comment("GALE01", Some(&long_commit), Some(&long_commit));
        // Should truncate SHA to 7 characters
        assert!(comment.contains("(abcdef1 - abcdef1)"));
        assert!(!comment.contains("abcdef1234567890"));
    }
}
