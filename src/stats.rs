use crate::annotation::{Fuse, Status};
use crate::output::OutputFormat;
use colored::Colorize;
use serde::Serialize;
use std::collections::HashMap;

/// One row in the owner breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct OwnerRow {
    pub owner: String,
    pub total: usize,
    pub detonated: usize,
    pub ticking: usize,
    pub inert: usize,
}

/// One row in the tag breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct TagRow {
    pub tag: String,
    pub total: usize,
    pub detonated: usize,
    pub ticking: usize,
    pub inert: usize,
}

/// One row in the month timeline breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct MonthRow {
    /// Expiry month in `YYYY-MM` format.
    pub month: String,
    pub total: usize,
    pub detonated: usize,
    pub ticking: usize,
    pub inert: usize,
}

/// The complete stats result.
#[derive(Debug, Serialize)]
pub struct StatsResult {
    pub total_fuses: usize,
    pub total_detonated: usize,
    pub total_ticking: usize,
    pub total_inert: usize,
    pub by_owner: Vec<OwnerRow>,
    pub by_tag: Vec<TagRow>,
    pub by_month: Vec<MonthRow>,
}

/// Compute stats from a slice of fuses.
/// Rows are sorted: detonated count descending, then total descending, then name ascending.
/// Month rows are sorted chronologically (ascending).
pub fn compute_stats(fuses: &[Fuse]) -> StatsResult {
    let mut owner_map: HashMap<String, OwnerRow> = HashMap::new();
    let mut tag_map: HashMap<String, TagRow> = HashMap::new();
    let mut month_map: HashMap<String, MonthRow> = HashMap::new();

    let mut total_fuses = 0usize;
    let mut total_detonated = 0usize;
    let mut total_ticking = 0usize;
    let mut total_inert = 0usize;

    for fuse in fuses {
        // Use as_deref to avoid cloning the Option<String> before the entry() call.
        let owner_key = fuse.owner.as_deref().unwrap_or("(unowned)");

        total_fuses += 1;

        let (is_detonated, is_ticking, is_inert) = match fuse.status {
            Status::Detonated => {
                total_detonated += 1;
                (1usize, 0usize, 0usize)
            }
            Status::Ticking => {
                total_ticking += 1;
                (0, 1, 0)
            }
            Status::Inert => {
                total_inert += 1;
                (0, 0, 1)
            }
        };

        // Update owner row
        let orow = owner_map
            .entry(owner_key.to_string())
            .or_insert_with(|| OwnerRow {
                owner: owner_key.to_string(),
                total: 0,
                detonated: 0,
                ticking: 0,
                inert: 0,
            });
        orow.total += 1;
        orow.detonated += is_detonated;
        orow.ticking += is_ticking;
        orow.inert += is_inert;

        // Update tag row
        let trow = tag_map.entry(fuse.tag.clone()).or_insert_with(|| TagRow {
            tag: fuse.tag.clone(),
            total: 0,
            detonated: 0,
            ticking: 0,
            inert: 0,
        });
        trow.total += 1;
        trow.detonated += is_detonated;
        trow.ticking += is_ticking;
        trow.inert += is_inert;

        // Update month row — group by expiry month (YYYY-MM).
        // The key is moved into entry(), so or_insert_with recomputes the format string
        // for the MonthRow.month field rather than cloning or pre-computing it.
        let mrow = month_map
            .entry(fuse.date.format("%Y-%m").to_string())
            .or_insert_with(|| MonthRow {
                month: fuse.date.format("%Y-%m").to_string(),
                total: 0,
                detonated: 0,
                ticking: 0,
                inert: 0,
            });
        mrow.total += 1;
        mrow.detonated += is_detonated;
        mrow.ticking += is_ticking;
        mrow.inert += is_inert;
    }

    let mut by_owner: Vec<OwnerRow> = owner_map.into_values().collect();
    by_owner.sort_by(|a, b| {
        b.detonated
            .cmp(&a.detonated)
            .then(b.total.cmp(&a.total))
            .then(a.owner.cmp(&b.owner))
    });

    let mut by_tag: Vec<TagRow> = tag_map.into_values().collect();
    by_tag.sort_by(|a, b| {
        b.detonated
            .cmp(&a.detonated)
            .then(b.total.cmp(&a.total))
            .then(a.tag.cmp(&b.tag))
    });

    // Month rows: sorted chronologically ascending by YYYY-MM string.
    let mut by_month: Vec<MonthRow> = month_map.into_values().collect();
    by_month.sort_by(|a, b| a.month.cmp(&b.month));

    StatsResult {
        total_fuses,
        total_detonated,
        total_ticking,
        total_inert,
        by_owner,
        by_tag,
        by_month,
    }
}

/// Truncate a name to fit within 20 chars (left-aligned).
/// If the name is longer than 18 chars, truncate to 18 and append "..".
/// Uses char-safe truncation to avoid panicking on multi-byte UTF-8 characters.
fn truncate_name(name: &str) -> String {
    if name.chars().count() > 18 {
        // Find the byte offset of the 18th char boundary so we can slice safely.
        let end = name
            .char_indices()
            .nth(18)
            .map(|(i, _)| i)
            .unwrap_or(name.len());
        format!("{}..", &name[..end])
    } else {
        name.to_string()
    }
}

/// Whether color output should be enabled (respects NO_COLOR env var).
fn color_enabled() -> bool {
    std::env::var("NO_COLOR").is_err()
}

/// Format a detonated count cell, optionally in red.
fn fmt_detonated(count: usize, use_color: bool) -> String {
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
        "OWNER", "TOTAL", "DETONATED", "TICKING", "INERT"
    );
    for row in &result.by_owner {
        let name = truncate_name(&row.owner);
        println!(
            "{:<20}{:>8}{}{:>8}{:>8}",
            name,
            row.total,
            fmt_detonated(row.detonated, use_color),
            row.ticking,
            row.inert,
        );
    }

    println!();

    // BY TAG
    println!("BY TAG");
    println!("------");
    println!(
        "{:<20}{:>8}{:>10}{:>8}{:>8}",
        "TAG", "TOTAL", "DETONATED", "TICKING", "INERT"
    );
    for row in &result.by_tag {
        let name = truncate_name(&row.tag);
        println!(
            "{:<20}{:>8}{}{:>8}{:>8}",
            name,
            row.total,
            fmt_detonated(row.detonated, use_color),
            row.ticking,
            row.inert,
        );
    }

    println!();
    println!(
        "{} fuse(s) total · {} detonated · {} ticking · {} inert",
        result.total_fuses, result.total_detonated, result.total_ticking, result.total_inert,
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
        if row.detonated > 0 {
            println!(
                "::warning ::OWNER {} has {} detonated fuse(s)",
                row.owner, row.detonated
            );
        }
    }
    for row in &result.by_tag {
        if row.detonated > 0 {
            println!(
                "::warning ::TAG {} has {} detonated fuse(s)",
                row.tag, row.detonated
            );
        }
    }
}

/// Print the month timeline breakdown in terminal format.
pub fn print_stats_month_terminal(result: &StatsResult) {
    let use_color = color_enabled();
    println!("BY MONTH");
    println!("--------");
    println!(
        "{:<20}{:>8}{:>10}{:>8}{:>8}",
        "MONTH", "TOTAL", "DETONATED", "TICKING", "INERT"
    );
    for row in &result.by_month {
        println!(
            "{:<20}{:>8}{}{:>8}{:>8}",
            row.month,
            row.total,
            fmt_detonated(row.detonated, use_color),
            row.ticking,
            row.inert,
        );
    }
    println!();
    println!(
        "{} fuse(s) total · {} detonated · {} ticking · {} inert",
        result.total_fuses, result.total_detonated, result.total_ticking, result.total_inert,
    );
}

/// Top-level dispatch.
pub fn print_stats(result: &StatsResult, format: &OutputFormat) {
    match format {
        OutputFormat::Terminal | OutputFormat::Csv => print_stats_terminal(result),
        OutputFormat::Json => print_stats_json(result),
        OutputFormat::GitHub => print_stats_github(result),
    }
}

/// Top-level dispatch for month breakdown.
pub fn print_stats_month(result: &StatsResult, format: &OutputFormat) {
    match format {
        OutputFormat::Terminal | OutputFormat::Csv => print_stats_month_terminal(result),
        OutputFormat::Json => print_stats_json(result),
        OutputFormat::GitHub => {
            for row in &result.by_month {
                if row.detonated > 0 {
                    println!(
                        "::warning ::MONTH {} has {} detonated fuse(s)",
                        row.month, row.detonated
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    fn make_fuse(tag: &str, owner: Option<&str>, status: Status) -> Fuse {
        let date = match status {
            Status::Detonated => NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
            Status::Ticking => NaiveDate::from_ymd_opt(2025, 6, 10).unwrap(),
            Status::Inert => NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
        };
        Fuse {
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
        assert_eq!(result.total_fuses, 0);
        assert_eq!(result.total_detonated, 0);
        assert_eq!(result.total_ticking, 0);
        assert_eq!(result.total_inert, 0);
        assert!(result.by_owner.is_empty());
        assert!(result.by_tag.is_empty());
        assert!(result.by_month.is_empty());
    }

    #[test]
    fn test_compute_stats_single_detonated() {
        let fuses = vec![make_fuse("TODO", Some("alice"), Status::Detonated)];
        let result = compute_stats(&fuses);
        assert_eq!(result.total_fuses, 1);
        assert_eq!(result.total_detonated, 1);
        assert_eq!(result.total_ticking, 0);
        assert_eq!(result.total_inert, 0);

        assert_eq!(result.by_owner.len(), 1);
        assert_eq!(result.by_owner[0].detonated, 1);

        assert_eq!(result.by_tag.len(), 1);
        assert_eq!(result.by_tag[0].detonated, 1);
    }

    #[test]
    fn test_compute_stats_unowned() {
        let fuses = vec![make_fuse("TODO", None, Status::Inert)];
        let result = compute_stats(&fuses);
        assert_eq!(result.by_owner.len(), 1);
        assert_eq!(result.by_owner[0].owner, "(unowned)");
    }

    #[test]
    fn test_compute_stats_owner_grouping() {
        let fuses = vec![
            make_fuse("TODO", Some("alice"), Status::Inert),
            make_fuse("FIXME", Some("alice"), Status::Detonated),
        ];
        let result = compute_stats(&fuses);
        assert_eq!(result.by_owner.len(), 1);
        assert_eq!(result.by_owner[0].owner, "alice");
        assert_eq!(result.by_owner[0].total, 2);
    }

    #[test]
    fn test_compute_stats_tag_grouping() {
        let fuses = vec![
            make_fuse("TODO", Some("alice"), Status::Inert),
            make_fuse("TODO", Some("bob"), Status::Detonated),
        ];
        let result = compute_stats(&fuses);
        assert_eq!(result.by_tag.len(), 1);
        assert_eq!(result.by_tag[0].tag, "TODO");
        assert_eq!(result.by_tag[0].total, 2);
    }

    #[test]
    fn test_compute_stats_sort_order() {
        let fuses = vec![
            make_fuse("TODO", Some("alice"), Status::Detonated),
            make_fuse("TODO", Some("alice"), Status::Detonated),
            make_fuse("TODO", Some("alice"), Status::Detonated),
            make_fuse("TODO", Some("bob"), Status::Detonated),
        ];
        let result = compute_stats(&fuses);
        assert_eq!(result.by_owner.len(), 2);
        // alice has 3 detonated, bob has 1 — alice should come first
        assert_eq!(result.by_owner[0].owner, "alice");
        assert_eq!(result.by_owner[0].detonated, 3);
        assert_eq!(result.by_owner[1].owner, "bob");
        assert_eq!(result.by_owner[1].detonated, 1);
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
        let fuses = vec![
            make_fuse("TODO", Some("alice"), Status::Detonated),
            make_fuse("FIXME", None, Status::Inert),
        ];
        let result = compute_stats(&fuses);
        print_stats_json(&result);
    }

    #[test]
    fn test_print_stats_terminal_does_not_panic() {
        let fuses = vec![
            make_fuse("TODO", Some("alice"), Status::Detonated),
            make_fuse("FIXME", None, Status::Ticking),
            make_fuse("HACK", Some("bob"), Status::Inert),
        ];
        let result = compute_stats(&fuses);
        print_stats_terminal(&result);
    }

    #[test]
    fn test_print_stats_github_does_not_panic() {
        let fuses = vec![
            make_fuse("TODO", Some("alice"), Status::Detonated),
            make_fuse("FIXME", None, Status::Inert),
        ];
        let result = compute_stats(&fuses);
        print_stats_github(&result);
    }

    fn make_fuse_on_date(tag: &str, owner: Option<&str>, status: Status, date: NaiveDate) -> Fuse {
        Fuse {
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
    fn test_by_month_grouping() {
        let fuses = vec![
            make_fuse_on_date(
                "TODO",
                None,
                Status::Detonated,
                NaiveDate::from_ymd_opt(2020, 3, 15).unwrap(),
            ),
            make_fuse_on_date(
                "FIXME",
                None,
                Status::Detonated,
                NaiveDate::from_ymd_opt(2020, 3, 28).unwrap(),
            ),
            make_fuse_on_date(
                "HACK",
                None,
                Status::Inert,
                NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
            ),
        ];
        let result = compute_stats(&fuses);
        assert_eq!(result.by_month.len(), 2);
        // 2020-03 comes before 2099-01
        assert_eq!(result.by_month[0].month, "2020-03");
        assert_eq!(result.by_month[0].total, 2);
        assert_eq!(result.by_month[0].detonated, 2);
        assert_eq!(result.by_month[1].month, "2099-01");
        assert_eq!(result.by_month[1].total, 1);
        assert_eq!(result.by_month[1].inert, 1);
    }

    #[test]
    fn test_by_month_sorted_chronologically() {
        let fuses = vec![
            make_fuse_on_date(
                "TODO",
                None,
                Status::Inert,
                NaiveDate::from_ymd_opt(2099, 6, 1).unwrap(),
            ),
            make_fuse_on_date(
                "TODO",
                None,
                Status::Detonated,
                NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
            ),
            make_fuse_on_date(
                "TODO",
                None,
                Status::Detonated,
                NaiveDate::from_ymd_opt(2020, 3, 1).unwrap(),
            ),
        ];
        let result = compute_stats(&fuses);
        let months: Vec<&str> = result.by_month.iter().map(|r| r.month.as_str()).collect();
        assert_eq!(months, vec!["2020-01", "2020-03", "2099-06"]);
    }

    #[test]
    fn test_by_month_empty() {
        let result = compute_stats(&[]);
        assert!(result.by_month.is_empty());
    }

    #[test]
    fn test_print_stats_month_terminal_does_not_panic() {
        let fuses = vec![
            make_fuse("TODO", Some("alice"), Status::Detonated),
            make_fuse("FIXME", None, Status::Ticking),
        ];
        let result = compute_stats(&fuses);
        print_stats_month_terminal(&result);
    }
}
