use std::cmp::Ordering;

use anyhow::Result;
use decomp_dev_core::models::Commit;
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

const MAX_CHANGE_LINES: i32 = 30;

#[derive(PartialEq, Eq)]
enum ChangeKind {
    NewMatch,
    BrokenMatch,
    Improvement,
    Regression,
}

struct ChangeLine {
    kind: ChangeKind,
    unit_name: String,
    item_name: String,
    to_fuzzy_match_percent: f32,
    bytes_diff: i64,
}

fn output_line(line: &ChangeLine, out: &mut String) {
    let emoji = match line.kind {
        ChangeKind::NewMatch => "✅",
        ChangeKind::BrokenMatch => "💔",
        ChangeKind::Improvement => "📈",
        ChangeKind::Regression => "📉",
    };

    let bytes_str = match line.bytes_diff.cmp(&0) {
        Ordering::Less => line.bytes_diff.to_string(),
        Ordering::Equal => "0".to_string(),
        Ordering::Greater => format!("+{}", line.bytes_diff),
    };
    out.push_str(&format!(
        "{emoji} `{} | {}` {} bytes -> {:.2}%\n",
        line.unit_name, line.item_name, bytes_str, line.to_fuzzy_match_percent
    ));
}

fn truncate_num_displayed(shown_improvements: &mut usize, shown_regressions: &mut usize) {
    loop {
        let excess = (*shown_improvements + *shown_regressions) as i32 - MAX_CHANGE_LINES;
        if excess <= 0 {
            return;
        }

        let excess = excess as usize;

        if shown_improvements == shown_regressions {
            *shown_improvements -= excess / 2;
            *shown_regressions -= excess / 2;
            if excess % 2 != 0 {
                *shown_regressions -= 1;
            }
            return;
        }

        if shown_improvements > shown_regressions {
            *shown_improvements -= excess.min(*shown_improvements - *shown_regressions);
        } else {
            *shown_regressions -= excess.min(*shown_regressions - *shown_improvements);
        }
    }
}

fn generate_changes_list(changes: Vec<ChangeLine>, out: &mut String) {
    let (mut improvements, mut regressions): (Vec<_>, Vec<_>) =
        changes.into_iter().partition(|item| {
            item.kind == ChangeKind::NewMatch || item.kind == ChangeKind::Improvement
        });
    // first show new matches, then other improvements, each sorted by amount improved
    improvements.sort_by_key(|item| (item.kind != ChangeKind::NewMatch, -item.bytes_diff));
    // first show broken matches, then regressions, each sorted by amount regressed
    regressions.sort_by_key(|item| (item.kind != ChangeKind::BrokenMatch, item.bytes_diff));

    let mut shown_improvements = improvements.len();
    let mut shown_regressions = regressions.len();

    truncate_num_displayed(&mut shown_improvements, &mut shown_regressions);

    if !improvements.is_empty() {
        let num_newly_matched =
            improvements.iter().filter(|item| item.kind == ChangeKind::NewMatch).count();

        if num_newly_matched == improvements.len() {
            out.push_str(&format!("{} newly matched\n", num_newly_matched));
        } else if num_newly_matched == 0 {
            out.push_str(&format!("{} improvements\n", improvements.len()));
        } else {
            out.push_str(&format!(
                "{} improvements ({} newly matched)\n",
                improvements.len(),
                num_newly_matched
            ));
        }

        for line in improvements.iter().take(shown_improvements) {
            output_line(line, out);
        }

        if shown_improvements < improvements.len() {
            out.push_str(&format!(
                "...and {} more improvements\n\n",
                improvements.len() - shown_improvements
            ));
        }
    }

    if !regressions.is_empty() {
        let num_broken_matches =
            regressions.iter().filter(|item| item.kind == ChangeKind::BrokenMatch).count();

        if num_broken_matches == regressions.len() {
            out.push_str(&format!("{} no longer matching\n", num_broken_matches));
        } else if num_broken_matches == 0 {
            out.push_str(&format!("{} regressions\n", regressions.len()));
        } else {
            out.push_str(&format!(
                "{} regressions ({} no longer matching)\n",
                regressions.len(),
                num_broken_matches
            ));
        }

        for line in regressions.iter().take(shown_regressions) {
            output_line(line, out);
        }

        out.push_str(&format!(
            "...and {} more regressions\n\n",
            regressions.len() - shown_regressions
        ));
    }
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
        } else {
            ChangeKind::Regression
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
