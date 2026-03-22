use crate::annotation::{Annotation, Status};
use crate::output::OutputFormat;
use colored::Colorize;
use serde::Serialize;
use std::collections::HashMap;

/// One row in the owner breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct OwnerRow {
    pub owner: String,
    pub total: usize,
    pub expired: usize,
    pub expiring_soon: usize,
    pub ok: usize,
}

/// One row in the tag breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct TagRow {
    pub tag: String,
    pub total: usize,
    pub expired: usize,
    pub expiring_soon: usize,
    pub ok: usize,
}

/// The complete stats result.
#[derive(Debug, Serialize)]
pub struct StatsResult {
    pub total_annotations: usize,
    pub total_expired: usize,
    pub total_expiring_soon: usize,
    pub total_ok: usize,
    pub by_owner: Vec<OwnerRow>,
    pub by_tag: Vec<TagRow>,
}

/// Compute stats from a slice of annotations.
/// Rows are sorted: expired count descending, then total descending, then name ascending.
pub fn compute_stats(annotations: &[Annotation]) -> StatsResult {
    let mut owner_map: HashMap<String, OwnerRow> = HashMap::new();
    let mut tag_map: HashMap<String, TagRow> = HashMap::new();

    let mut total_annotations = 0usize;
    let mut total_expired = 0usize;
    let mut total_expiring_soon = 0usize;
    let mut total_ok = 0usize;

    for ann in annotations {
        let owner_key = ann.owner.clone().unwrap_or_else(|| "(unowned)".to_string());

        total_annotations += 1;

        let (is_expired, is_soon, is_ok) = match ann.status {
            Status::Expired => {
                total_expired += 1;
                (1usize, 0usize, 0usize)
            }
            Status::ExpiringSoon => {
                total_expiring_soon += 1;
                (0, 1, 0)
            }
            Status::Ok => {
                total_ok += 1;
                (0, 0, 1)
            }
        };

        // Update owner row
        let orow = owner_map
            .entry(owner_key.clone())
            .or_insert_with(|| OwnerRow {
                owner: owner_key,
                total: 0,
                expired: 0,
                expiring_soon: 0,
                ok: 0,
            });
        orow.total += 1;
        orow.expired += is_expired;
        orow.expiring_soon += is_soon;
        orow.ok += is_ok;

        // Update tag row
        let trow = tag_map.entry(ann.tag.clone()).or_insert_with(|| TagRow {
            tag: ann.tag.clone(),
            total: 0,
            expired: 0,
            expiring_soon: 0,
            ok: 0,
        });
        trow.total += 1;
        trow.expired += is_expired;
        trow.expiring_soon += is_soon;
        trow.ok += is_ok;
    }

    let mut by_owner: Vec<OwnerRow> = owner_map.into_values().collect();
    by_owner.sort_by(|a, b| {
        b.expired
            .cmp(&a.expired)
            .then(b.total.cmp(&a.total))
            .then(a.owner.cmp(&b.owner))
    });

    let mut by_tag: Vec<TagRow> = tag_map.into_values().collect();
    by_tag.sort_by(|a, b| {
        b.expired
            .cmp(&a.expired)
            .then(b.total.cmp(&a.total))
            .then(a.tag.cmp(&b.tag))
    });

    StatsResult {
        total_annotations,
        total_expired,
        total_expiring_soon,
        total_ok,
        by_owner,
        by_tag,
    }
}

/// Truncate a name to fit within 20 chars (left-aligned).
/// If the name is longer than 18 chars, truncate to 18 and append "..".
fn truncate_name(name: &str) -> String {
    if name.len() > 18 {
        format!("{}..", &name[..18])
    } else {
        name.to_string()
    }
}

/// Whether color output should be enabled (respects NO_COLOR env var).
fn color_enabled() -> bool {
    std::env::var("NO_COLOR").is_err()
}

/// Format an expired count cell, optionally in red.
fn fmt_expired(count: usize, use_color: bool) -> String {
    let s = format!("{:>8}", count);
    if use_color && count > 0 {
        s.red().to_string()
    } else {
        s
    }
}

/// Print stats in terminal (human-readable table) format.
pub fn print_stats_terminal(result: &StatsResult) {
    let use_color = color_enabled();

    // BY OWNER
    println!("BY OWNER");
    println!("--------");
    println!(
        "{:<20}{:>8}{:>10}{:>8}{:>8}",
        "OWNER", "TOTAL", "EXPIRED", "SOON", "OK"
    );
    for row in &result.by_owner {
        let name = truncate_name(&row.owner);
        println!(
            "{:<20}{:>8}{}{:>8}{:>8}",
            name,
            row.total,
            fmt_expired(row.expired, use_color),
            row.expiring_soon,
            row.ok,
        );
    }

    println!();

    // BY TAG
    println!("BY TAG");
    println!("------");
    println!(
        "{:<20}{:>8}{:>10}{:>8}{:>8}",
        "TAG", "TOTAL", "EXPIRED", "SOON", "OK"
    );
    for row in &result.by_tag {
        let name = truncate_name(&row.tag);
        println!(
            "{:<20}{:>8}{}{:>8}{:>8}",
            name,
            row.total,
            fmt_expired(row.expired, use_color),
            row.expiring_soon,
            row.ok,
        );
    }

    println!();
    println!(
        "{} annotation(s) total · {} expired · {} expiring soon · {} ok",
        result.total_annotations, result.total_expired, result.total_expiring_soon, result.total_ok,
    );
}

/// Print stats as JSON.
pub fn print_stats_json(result: &StatsResult) {
    let json = serde_json::to_string_pretty(result).expect("Failed to serialize stats to JSON");
    println!("{}", json);
}

/// Print stats in GitHub Actions format.
pub fn print_stats_github(result: &StatsResult) {
    for row in &result.by_owner {
        if row.expired > 0 {
            println!(
                "::warning ::OWNER {} has {} expired annotation(s)",
                row.owner, row.expired
            );
        }
    }
    for row in &result.by_tag {
        if row.expired > 0 {
            println!(
                "::warning ::TAG {} has {} expired annotation(s)",
                row.tag, row.expired
            );
        }
    }
}

/// Top-level dispatch.
pub fn print_stats(result: &StatsResult, format: &OutputFormat) {
    match format {
        OutputFormat::Terminal => print_stats_terminal(result),
        OutputFormat::Json => print_stats_json(result),
        OutputFormat::GitHub => print_stats_github(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    fn make_annotation(tag: &str, owner: Option<&str>, status: Status) -> Annotation {
        let date = match status {
            Status::Expired => NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
            Status::ExpiringSoon => NaiveDate::from_ymd_opt(2025, 6, 10).unwrap(),
            Status::Ok => NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
        };
        Annotation {
            file: PathBuf::from("src/foo.rs"),
            line: 1,
            tag: tag.to_string(),
            date,
            owner: owner.map(|s| s.to_string()),
            message: "test message".to_string(),
            status,
            blamed_owner: None,
        }
    }

    #[test]
    fn test_compute_stats_empty() {
        let result = compute_stats(&[]);
        assert_eq!(result.total_annotations, 0);
        assert_eq!(result.total_expired, 0);
        assert_eq!(result.total_expiring_soon, 0);
        assert_eq!(result.total_ok, 0);
        assert!(result.by_owner.is_empty());
        assert!(result.by_tag.is_empty());
    }

    #[test]
    fn test_compute_stats_single_expired() {
        let anns = vec![make_annotation("TODO", Some("alice"), Status::Expired)];
        let result = compute_stats(&anns);
        assert_eq!(result.total_annotations, 1);
        assert_eq!(result.total_expired, 1);
        assert_eq!(result.total_expiring_soon, 0);
        assert_eq!(result.total_ok, 0);

        assert_eq!(result.by_owner.len(), 1);
        assert_eq!(result.by_owner[0].expired, 1);

        assert_eq!(result.by_tag.len(), 1);
        assert_eq!(result.by_tag[0].expired, 1);
    }

    #[test]
    fn test_compute_stats_unowned() {
        let anns = vec![make_annotation("TODO", None, Status::Ok)];
        let result = compute_stats(&anns);
        assert_eq!(result.by_owner.len(), 1);
        assert_eq!(result.by_owner[0].owner, "(unowned)");
    }

    #[test]
    fn test_compute_stats_owner_grouping() {
        let anns = vec![
            make_annotation("TODO", Some("alice"), Status::Ok),
            make_annotation("FIXME", Some("alice"), Status::Expired),
        ];
        let result = compute_stats(&anns);
        assert_eq!(result.by_owner.len(), 1);
        assert_eq!(result.by_owner[0].owner, "alice");
        assert_eq!(result.by_owner[0].total, 2);
    }

    #[test]
    fn test_compute_stats_tag_grouping() {
        let anns = vec![
            make_annotation("TODO", Some("alice"), Status::Ok),
            make_annotation("TODO", Some("bob"), Status::Expired),
        ];
        let result = compute_stats(&anns);
        assert_eq!(result.by_tag.len(), 1);
        assert_eq!(result.by_tag[0].tag, "TODO");
        assert_eq!(result.by_tag[0].total, 2);
    }

    #[test]
    fn test_compute_stats_sort_order() {
        let anns = vec![
            make_annotation("TODO", Some("alice"), Status::Expired),
            make_annotation("TODO", Some("alice"), Status::Expired),
            make_annotation("TODO", Some("alice"), Status::Expired),
            make_annotation("TODO", Some("bob"), Status::Expired),
        ];
        let result = compute_stats(&anns);
        assert_eq!(result.by_owner.len(), 2);
        // alice has 3 expired, bob has 1 — alice should come first
        assert_eq!(result.by_owner[0].owner, "alice");
        assert_eq!(result.by_owner[0].expired, 3);
        assert_eq!(result.by_owner[1].owner, "bob");
        assert_eq!(result.by_owner[1].expired, 1);
    }

    #[test]
    fn test_owner_row_name_truncation() {
        // A name of exactly 19 chars is longer than 18, so it should be truncated
        let long_name = "a".repeat(19); // 19 chars > 18
        let truncated = truncate_name(&long_name);
        assert_eq!(truncated.len(), 20); // 18 chars + ".."
        assert!(truncated.ends_with(".."));

        // A name of exactly 18 chars should NOT be truncated
        let exact_name = "b".repeat(18);
        let not_truncated = truncate_name(&exact_name);
        assert_eq!(not_truncated, exact_name);

        // A name shorter than 18 chars should not be truncated
        let short_name = "hello";
        let not_truncated_short = truncate_name(short_name);
        assert_eq!(not_truncated_short, short_name);
    }

    #[test]
    fn test_print_stats_json_does_not_panic() {
        let anns = vec![
            make_annotation("TODO", Some("alice"), Status::Expired),
            make_annotation("FIXME", None, Status::Ok),
        ];
        let result = compute_stats(&anns);
        print_stats_json(&result);
    }

    #[test]
    fn test_print_stats_terminal_does_not_panic() {
        let anns = vec![
            make_annotation("TODO", Some("alice"), Status::Expired),
            make_annotation("FIXME", None, Status::ExpiringSoon),
            make_annotation("HACK", Some("bob"), Status::Ok),
        ];
        let result = compute_stats(&anns);
        print_stats_terminal(&result);
    }

    #[test]
    fn test_print_stats_github_does_not_panic() {
        let anns = vec![
            make_annotation("TODO", Some("alice"), Status::Expired),
            make_annotation("FIXME", None, Status::Ok),
        ];
        let result = compute_stats(&anns);
        print_stats_github(&result);
    }
}
