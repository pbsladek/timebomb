//! Integration tests for the scanner module using fixture files.
//!
//! Fixtures contain hardcoded past (2020-01-01) and future (2099-01-01) dates
//! so these tests never depend on the current wall-clock date.

use chrono::Datelike;
use std::path::{Path, PathBuf};

// Bring in the library modules by path since this is an integration test
// that lives outside src/. We use `timebomb` as the crate name.
use timebomb::annotation::Status;
use timebomb::config::Config;
use timebomb::scanner::{build_regex, scan, scan_content};

/// Convenience helper: scan an inline string with a freshly built regex.
/// Mirrors the old `scan_str` function that is now `pub(crate)` (test-only).
fn scan_str(
    src: &str,
    path: &Path,
    cfg: &Config,
    today: chrono::NaiveDate,
) -> timebomb::error::Result<Vec<timebomb::annotation::Fuse>> {
    let regex = build_regex(cfg)?;
    scan_content(src, path, &regex, cfg, today)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn fixture_path(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

/// Fixed "today" used across all tests: 2025-06-01.
/// Past dates (2020-01-01) will be Detonated.
/// Near-future dates (2025-06-08, 2025-06-10) will be Ticking when fuse window >= 9.
/// Far-future dates (2099-01-01) will always be Inert.
fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str("2025-06-01", "%Y-%m-%d").unwrap()
}

fn default_config() -> Config {
    Config::default()
}

fn config_with_fuse(days: u32) -> Config {
    Config {
        fuse_days: days,
        ..Config::default()
    }
}

// ─── Fixture: sample.rs ───────────────────────────────────────────────────────

#[test]
fn test_sample_rs_detonated_count() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let detonated: Vec<_> = fuses.iter().filter(|a| a.is_detonated()).collect();

    // sample.rs has 6 detonated fuses (5 unowned + 1 owned by alice)
    assert_eq!(
        detonated.len(),
        6,
        "expected 6 detonated fuses in sample.rs, got {}: {:?}",
        detonated.len(),
        detonated
            .iter()
            .map(|a| (&a.tag, a.date_str()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_sample_rs_future_count() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();

    // With fuse_days=0, the 2025-06-08 and 2025-06-10 items are Ticking (0 days <= 0 is false for those)
    // Actually with fuse_days=0: days_remaining for 2025-06-08 is 7 > 0 so Inert;
    // days_remaining for 2025-06-10 is 9 > 0 so Inert.
    // Future (2099-*): 4 fuses
    let inert: Vec<_> = fuses.iter().filter(|a| a.status == Status::Inert).collect();
    assert!(
        inert.len() >= 4,
        "expected at least 4 inert fuses in sample.rs, got {}",
        inert.len()
    );
}

#[test]
fn test_sample_rs_plain_todos_not_matched() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();

    // There must be no fuse with an empty message or one that came from a plain TODO comment
    // Plain "// TODO: this is a plain TODO..." must not produce any match
    for fuse in &fuses {
        assert!(
            !fuse.message.starts_with("this is a plain"),
            "plain TODO should not be matched: {:?}",
            fuse
        );
    }
}

#[test]
fn test_sample_rs_space_before_bracket_not_matched() {
    // "// TODO [2020-01-01]: space between tag and bracket" must NOT match
    let src = "// TODO [2020-01-01]: space before bracket should not match\n";
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let fuses = scan_content(src, Path::new("test.rs"), &regex, &cfg, today()).unwrap();
    assert!(
        fuses.is_empty(),
        "space between tag and bracket must not produce a match"
    );
}

#[test]
fn test_sample_rs_note_tag_not_matched() {
    // NOTE is not in the default tag list
    let src = "// NOTE[2020-01-01]: this should not be matched\n";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("test.rs"), &cfg, today()).unwrap();
    assert!(
        fuses.is_empty(),
        "NOTE tag must not match with default config"
    );
}

#[test]
fn test_sample_rs_alice_owner_detected() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let alice_fuse = fuses
        .iter()
        .find(|a| a.owner.as_deref() == Some("alice"))
        .expect("should find fuse owned by alice");

    assert_eq!(alice_fuse.tag, "TODO");
    assert!(alice_fuse.is_detonated());
}

#[test]
fn test_sample_rs_bob_owner_detected() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let bob_fuse = fuses
        .iter()
        .find(|a| a.owner.as_deref() == Some("bob"))
        .expect("should find fuse owned by bob");

    assert_eq!(bob_fuse.tag, "TODO");
    assert_eq!(bob_fuse.status, Status::Inert);
}

#[test]
fn test_sample_rs_ticking_with_wide_window() {
    // With fuse_days=30d, the 2025-06-08 and 2025-06-10 items should be Ticking
    let path = fixture_path("sample.rs");
    let cfg = config_with_fuse(30);
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let ticking: Vec<_> = fuses.iter().filter(|a| a.is_ticking()).collect();
    assert!(
        ticking.len() >= 2,
        "expected at least 2 ticking fuses with 30d window, got {}",
        ticking.len()
    );
}

#[test]
fn test_sample_rs_all_tags_present() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let tags: std::collections::HashSet<&str> = fuses.iter().map(|a| a.tag.as_str()).collect();

    for expected_tag in &["TODO", "FIXME", "HACK", "TEMP", "REMOVEME"] {
        assert!(
            tags.contains(expected_tag),
            "expected tag {} to appear in sample.rs fuses",
            expected_tag
        );
    }
}

// ─── Fixture: sample.py ───────────────────────────────────────────────────────

#[test]
fn test_sample_py_detonated_count() {
    let path = fixture_path("sample.py");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.py");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let detonated: Vec<_> = fuses.iter().filter(|a| a.is_detonated()).collect();

    // sample.py has 6 detonated fuses (5 unowned + 1 owned by carol)
    assert_eq!(
        detonated.len(),
        6,
        "expected 6 detonated fuses in sample.py, got {}",
        detonated.len()
    );
}

#[test]
fn test_sample_py_hash_prefix_tags_detected() {
    // Python uses # comments — make sure # prefix doesn't confuse the scanner
    let src = "# TODO[2020-01-01]: python style comment\n";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("test.py"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 1);
    assert_eq!(fuses[0].tag, "TODO");
    assert_eq!(fuses[0].message, "python style comment");
    assert!(fuses[0].is_detonated());
}

#[test]
fn test_sample_py_owner_carol_detected() {
    let path = fixture_path("sample.py");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.py");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let carol_fuse = fuses
        .iter()
        .find(|a| a.owner.as_deref() == Some("carol"))
        .expect("should find fuse owned by carol");

    assert!(carol_fuse.is_detonated());
}

#[test]
fn test_sample_py_future_fuses_are_inert() {
    let path = fixture_path("sample.py");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.py");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let future: Vec<_> = fuses
        .iter()
        .filter(|a| a.date.year() == 2099 || a.date.year() == 2088)
        .collect();

    for fuse in &future {
        assert_eq!(
            fuse.status,
            Status::Inert,
            "far-future fuse should be Inert: {:?}",
            fuse
        );
    }
}

#[test]
fn test_sample_py_plain_todos_ignored() {
    let path = fixture_path("sample.py");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.py");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();

    // None of the fuses should have empty messages or messages starting with "plain todo"
    for fuse in &fuses {
        assert!(
            !fuse.message.to_lowercase().starts_with("plain todo"),
            "plain TODO should not be matched: {:?}",
            fuse
        );
    }
}

// ─── Fixture: sample.sql ─────────────────────────────────────────────────────

#[test]
fn test_sample_sql_detonated_count() {
    let path = fixture_path("sample.sql");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.sql");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let detonated: Vec<_> = fuses.iter().filter(|a| a.is_detonated()).collect();

    // sample.sql has 6 detonated fuses (5 unowned + 1 owned by eve)
    assert_eq!(
        detonated.len(),
        6,
        "expected 6 detonated fuses in sample.sql, got {}",
        detonated.len()
    );
}

#[test]
fn test_sample_sql_double_dash_prefix_detected() {
    // SQL uses -- comments
    let src = "-- TODO[2020-01-01]: drop this column\n";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("schema.sql"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 1);
    assert_eq!(fuses[0].tag, "TODO");
    assert_eq!(fuses[0].message, "drop this column");
}

#[test]
fn test_sample_sql_owner_eve_detected() {
    let path = fixture_path("sample.sql");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.sql");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let eve_fuse = fuses
        .iter()
        .find(|a| a.owner.as_deref() == Some("eve"))
        .expect("should find fuse owned by eve");

    assert!(eve_fuse.is_detonated());
}

#[test]
fn test_sample_sql_owner_frank_detected() {
    let path = fixture_path("sample.sql");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.sql");

    let fuses = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let frank_fuse = fuses
        .iter()
        .find(|a| a.owner.as_deref() == Some("frank"))
        .expect("should find fuse owned by frank");

    assert_eq!(frank_fuse.status, Status::Inert);
}

// ─── Full directory scan ──────────────────────────────────────────────────────

#[test]
fn test_scan_fixtures_dir_finds_all_files() {
    let dir = fixtures_dir();
    let cfg = default_config();

    let result = scan(&dir, &cfg, today()).unwrap();

    // 25 fixture files: rs, py, sql, ts, rb, go, js, hs, fs, cs, java, php, clj, lisp, rkt,
    //                    ex, erl, c, cpp, d, swift, ml, lua, dart, kt
    assert_eq!(
        result.swept_files, 25,
        "expected 25 swept files in fixtures dir, got {}",
        result.swept_files
    );
}

#[test]
fn test_scan_fixtures_dir_has_detonated() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();
    assert!(
        result.has_detonated(),
        "fixtures directory must contain detonated fuses"
    );
}

#[test]
fn test_scan_fixtures_dir_total_detonated_count() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    let detonated_count = result.detonated().len();
    assert_eq!(
        detonated_count, 100,
        "expected 100 total detonated fuses across all fixture files, got {}",
        detonated_count
    );
}

#[test]
fn test_scan_fixtures_dir_sorted_by_date() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    let dates: Vec<_> = result.fuses.iter().map(|a| a.date).collect();
    let mut sorted = dates.clone();
    sorted.sort();
    assert_eq!(
        dates, sorted,
        "scan results must be sorted by date ascending"
    );
}

#[test]
fn test_scan_fixtures_dir_ticking_with_wide_window() {
    let dir = fixtures_dir();
    let cfg = config_with_fuse(30);
    let result = scan(&dir, &cfg, today()).unwrap();

    // rs/py/go/js/ts: 2 each; rb/hs/fs/cs/java/php/clj/lisp/rkt/ex/erl/c/cpp/d/kt: 1 each; sql: 0
    let ticking_count = result.ticking().len();
    assert_eq!(
        ticking_count, 29,
        "expected 29 ticking fuses with 30d window across fixture files, got {}",
        ticking_count
    );
}

#[test]
fn test_scan_fixtures_dir_has_ticking_with_wide_window() {
    let dir = fixtures_dir();
    let cfg = config_with_fuse(30);
    let result = scan(&dir, &cfg, today()).unwrap();
    assert!(
        result.is_ticking(),
        "should detect ticking fuses with 30d window"
    );
}

// ─── Inline fuse pattern tests ────────────────────────────────────────────────

#[test]
fn test_all_five_default_tags_matched() {
    let src = "\
// TODO[2020-01-01]: todo detonated
// FIXME[2020-01-01]: fixme detonated
// HACK[2020-01-01]: hack detonated
// TEMP[2020-01-01]: temp detonated
// REMOVEME[2020-01-01]: removeme detonated
";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("tags.rs"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 5);
    let tags: Vec<&str> = fuses.iter().map(|a| a.tag.as_str()).collect();
    assert!(tags.contains(&"TODO"));
    assert!(tags.contains(&"FIXME"));
    assert!(tags.contains(&"HACK"));
    assert!(tags.contains(&"TEMP"));
    assert!(tags.contains(&"REMOVEME"));
}

#[test]
fn test_case_insensitive_tags() {
    let src = "\
// todo[2020-01-01]: lowercase todo
// Fixme[2020-01-01]: mixed case fixme
// HACK[2020-01-01]: uppercase hack
";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("case.rs"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 3);
    // All tags should be normalised to upper case
    for fuse in &fuses {
        assert_eq!(
            fuse.tag,
            fuse.tag.to_uppercase(),
            "tag should be uppercased: {}",
            fuse.tag
        );
    }
}

#[test]
fn test_fuse_on_same_line_as_code() {
    let src = r#"let x = some_func(); // TODO[2020-01-01]: refactor this inline"#;
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("inline.rs"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 1);
    assert_eq!(fuses[0].message, "refactor this inline");
}

#[test]
fn test_multiple_fuses_on_separate_lines() {
    let src = "\
fn foo() {
    // TODO[2020-01-01]: first
    let x = 1;
    // FIXME[2099-12-31]: second
    let y = 2;
    // HACK[2020-06-15]: third
}
";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("multi.rs"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 3);
    // Verify line numbers (1-based)
    let lines: Vec<usize> = fuses.iter().map(|a| a.line).collect();
    assert!(lines.contains(&2), "line 2 should have TODO");
    assert!(lines.contains(&4), "line 4 should have FIXME");
    assert!(lines.contains(&6), "line 6 should have HACK");
}

#[test]
fn test_owner_with_spaces_trimmed() {
    let src = "// TODO[2020-01-01][ alice ]: trimmed owner\n";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("owner.rs"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 1);
    // The owner is captured as the content between brackets; the scanner trims it
    assert_eq!(fuses[0].owner.as_deref(), Some("alice"));
}

#[test]
fn test_invalid_date_produces_no_fuse() {
    // Month 13 is not valid — should warn to stderr and skip
    let src = "// TODO[2026-13-01]: invalid month\n";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("bad.rs"), &cfg, today()).unwrap();
    assert!(
        fuses.is_empty(),
        "invalid date should produce no fuse, got {:?}",
        fuses
    );
}

#[test]
fn test_invalid_day_produces_no_fuse() {
    let src = "// FIXME[2026-02-30]: invalid day\n";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("bad.rs"), &cfg, today()).unwrap();
    assert!(
        fuses.is_empty(),
        "invalid day should produce no fuse, got {:?}",
        fuses
    );
}

#[test]
fn test_message_with_colons_and_special_chars() {
    let src = "// TODO[2020-01-01]: fix https://example.com/path?q=1&r=2 handling\n";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("url.rs"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 1);
    assert!(fuses[0].message.contains("https://"));
}

#[test]
fn test_file_path_stored_as_relative() {
    let src = "// TODO[2020-01-01]: check path\n";
    let cfg = default_config();
    let rel = Path::new("some/deep/path/file.rs");
    let fuses = scan_str(src, rel, &cfg, today()).unwrap();
    assert_eq!(fuses[0].file, rel);
}

// ─── Binary file detection ────────────────────────────────────────────────────

/// Verify that the full `scan()` pipeline skips binary files (null bytes) and
/// scans text files — covers the inline binary detection in Phase 2.
#[test]
fn test_scan_skips_binary_files() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();

    // A text file with a fuse — should be scanned.
    let mut text = std::fs::File::create(dir.path().join("ok.rs")).unwrap();
    writeln!(text, "// TODO[2020-01-01]: detonated fuse").unwrap();

    // A binary file (contains null byte) — should be skipped.
    let mut bin = std::fs::File::create(dir.path().join("blob.bin")).unwrap();
    bin.write_all(b"ELF\x00binary\x00data").unwrap();

    let cfg = Config {
        extensions: vec!["rs".to_string(), "bin".to_string()],
        ..Config::default()
    };
    let result = scan(dir.path(), &cfg, today()).unwrap();

    assert_eq!(result.swept_files, 1, "only the text file should be swept");
    assert_eq!(
        result.skipped_files, 1,
        "the binary file should be counted as skipped"
    );
    assert_eq!(result.fuses.len(), 1);
}

// ─── Custom trigger configuration ─────────────────────────────────────────────

#[test]
fn test_custom_trigger_only_matches_custom() {
    let src = "\
// TODO[2020-01-01]: standard tag — should NOT match
// CUSTOM[2020-01-01]: custom tag — should match
// FIXME[2020-01-01]: another standard — should NOT match
";
    let cfg = Config {
        triggers: vec!["CUSTOM".to_string()],
        ..Config::default()
    };
    let fuses = scan_str(src, Path::new("custom.rs"), &cfg, today()).unwrap();
    assert_eq!(fuses.len(), 1, "only CUSTOM trigger should match");
    assert_eq!(fuses[0].tag, "CUSTOM");
}

#[test]
fn test_empty_file_produces_no_fuses() {
    let cfg = default_config();
    let fuses = scan_str("", Path::new("empty.rs"), &cfg, today()).unwrap();
    assert!(fuses.is_empty());
}

#[test]
fn test_file_with_only_plain_todos_produces_no_fuses() {
    let src = "\
// TODO: this
// FIXME: that
// HACK: something
// TODO fix me
// TODO - do this later
";
    let cfg = default_config();
    let fuses = scan_str(src, Path::new("plain.rs"), &cfg, today()).unwrap();
    assert!(
        fuses.is_empty(),
        "plain TODOs without date brackets must not produce fuses"
    );
}

// ─── scan_result helpers ──────────────────────────────────────────────────────

#[test]
fn test_scan_result_detonated_helper() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    let detonated = result.detonated();
    assert!(!detonated.is_empty());
    for fuse in &detonated {
        assert_eq!(fuse.status, Status::Detonated);
    }
}

#[test]
fn test_scan_result_inert_helper() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    let inert = result.inert();
    assert!(!inert.is_empty());
    for fuse in inert {
        assert_eq!(fuse.status, Status::Inert);
    }
}

#[test]
fn test_scan_result_total_equals_sum_of_parts() {
    let dir = fixtures_dir();
    let cfg = config_with_fuse(30);
    let result = scan(&dir, &cfg, today()).unwrap();

    let total = result.total();
    let sum = result.detonated().len() + result.ticking().len() + result.inert().len();
    assert_eq!(total, sum, "total() must equal detonated + ticking + inert");
}
