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
) -> timebomb::error::Result<Vec<timebomb::annotation::Annotation>> {
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
/// Past dates (2020-01-01) will be Expired.
/// Near-future dates (2025-06-08, 2025-06-10) will be ExpiringSoon when warn window >= 9.
/// Far-future dates (2099-01-01) will always be Ok.
fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str("2025-06-01", "%Y-%m-%d").unwrap()
}

fn default_config() -> Config {
    Config::default()
}

fn config_with_warn(days: u32) -> Config {
    Config {
        warn_within_days: days,
        ..Config::default()
    }
}

// ─── Fixture: sample.rs ───────────────────────────────────────────────────────

#[test]
fn test_sample_rs_expired_count() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let expired: Vec<_> = anns.iter().filter(|a| a.is_expired()).collect();

    // sample.rs has 6 expired annotations (5 unowned + 1 owned by alice)
    assert_eq!(
        expired.len(),
        6,
        "expected 6 expired annotations in sample.rs, got {}: {:?}",
        expired.len(),
        expired
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

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();

    // With warn_within=0, the 2025-06-08 and 2025-06-10 items are ExpiringSoon (0 days <= 0 is false for those)
    // Actually with warn_within=0: days_remaining for 2025-06-08 is 7 > 0 so Ok;
    // days_remaining for 2025-06-10 is 9 > 0 so Ok.
    // Future (2099-*): 4 annotations
    let ok: Vec<_> = anns.iter().filter(|a| a.status == Status::Ok).collect();
    assert!(
        ok.len() >= 4,
        "expected at least 4 ok annotations in sample.rs, got {}",
        ok.len()
    );
}

#[test]
fn test_sample_rs_plain_todos_not_matched() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();

    // There must be no annotation with an empty message or one that came from a plain TODO comment
    // Plain "// TODO: this is a plain TODO..." must not produce any match
    for ann in &anns {
        assert!(
            !ann.message.starts_with("this is a plain"),
            "plain TODO should not be matched: {:?}",
            ann
        );
    }
}

#[test]
fn test_sample_rs_space_before_bracket_not_matched() {
    // "// TODO [2020-01-01]: space between tag and bracket" must NOT match
    let src = "// TODO [2020-01-01]: space before bracket should not match\n";
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let anns = scan_content(src, Path::new("test.rs"), &regex, &cfg, today()).unwrap();
    assert!(
        anns.is_empty(),
        "space between tag and bracket must not produce a match"
    );
}

#[test]
fn test_sample_rs_note_tag_not_matched() {
    // NOTE is not in the default tag list
    let src = "// NOTE[2020-01-01]: this should not be matched\n";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("test.rs"), &cfg, today()).unwrap();
    assert!(
        anns.is_empty(),
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

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let alice_ann = anns
        .iter()
        .find(|a| a.owner.as_deref() == Some("alice"))
        .expect("should find annotation owned by alice");

    assert_eq!(alice_ann.tag, "TODO");
    assert!(alice_ann.is_expired());
}

#[test]
fn test_sample_rs_bob_owner_detected() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let bob_ann = anns
        .iter()
        .find(|a| a.owner.as_deref() == Some("bob"))
        .expect("should find annotation owned by bob");

    assert_eq!(bob_ann.tag, "TODO");
    assert_eq!(bob_ann.status, Status::Ok);
}

#[test]
fn test_sample_rs_expiring_soon_with_wide_window() {
    // With warn_within=30d, the 2025-06-08 and 2025-06-10 items should be ExpiringSoon
    let path = fixture_path("sample.rs");
    let cfg = config_with_warn(30);
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let soon: Vec<_> = anns.iter().filter(|a| a.is_expiring_soon()).collect();
    assert!(
        soon.len() >= 2,
        "expected at least 2 expiring-soon annotations with 30d window, got {}",
        soon.len()
    );
}

#[test]
fn test_sample_rs_all_tags_present() {
    let path = fixture_path("sample.rs");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.rs");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let tags: std::collections::HashSet<&str> = anns.iter().map(|a| a.tag.as_str()).collect();

    for expected_tag in &["TODO", "FIXME", "HACK", "TEMP", "REMOVEME"] {
        assert!(
            tags.contains(expected_tag),
            "expected tag {} to appear in sample.rs annotations",
            expected_tag
        );
    }
}

// ─── Fixture: sample.py ───────────────────────────────────────────────────────

#[test]
fn test_sample_py_expired_count() {
    let path = fixture_path("sample.py");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.py");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let expired: Vec<_> = anns.iter().filter(|a| a.is_expired()).collect();

    // sample.py has 6 expired annotations (5 unowned + 1 owned by carol)
    assert_eq!(
        expired.len(),
        6,
        "expected 6 expired annotations in sample.py, got {}",
        expired.len()
    );
}

#[test]
fn test_sample_py_hash_prefix_tags_detected() {
    // Python uses # comments — make sure # prefix doesn't confuse the scanner
    let src = "# TODO[2020-01-01]: python style comment\n";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("test.py"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 1);
    assert_eq!(anns[0].tag, "TODO");
    assert_eq!(anns[0].message, "python style comment");
    assert!(anns[0].is_expired());
}

#[test]
fn test_sample_py_owner_carol_detected() {
    let path = fixture_path("sample.py");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.py");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let carol_ann = anns
        .iter()
        .find(|a| a.owner.as_deref() == Some("carol"))
        .expect("should find annotation owned by carol");

    assert!(carol_ann.is_expired());
}

#[test]
fn test_sample_py_future_annotations_are_ok() {
    let path = fixture_path("sample.py");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.py");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let future: Vec<_> = anns
        .iter()
        .filter(|a| a.date.year() == 2099 || a.date.year() == 2088)
        .collect();

    for ann in &future {
        assert_eq!(
            ann.status,
            Status::Ok,
            "far-future annotation should be Ok: {:?}",
            ann
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

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();

    // None of the annotations should have empty messages or messages starting
    // with "plain todo"
    for ann in &anns {
        assert!(
            !ann.message.to_lowercase().starts_with("plain todo"),
            "plain TODO should not be matched: {:?}",
            ann
        );
    }
}

// ─── Fixture: sample.sql ─────────────────────────────────────────────────────

#[test]
fn test_sample_sql_expired_count() {
    let path = fixture_path("sample.sql");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.sql");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let expired: Vec<_> = anns.iter().filter(|a| a.is_expired()).collect();

    // sample.sql has 6 expired annotations (5 unowned + 1 owned by eve)
    assert_eq!(
        expired.len(),
        6,
        "expected 6 expired annotations in sample.sql, got {}",
        expired.len()
    );
}

#[test]
fn test_sample_sql_double_dash_prefix_detected() {
    // SQL uses -- comments
    let src = "-- TODO[2020-01-01]: drop this column\n";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("schema.sql"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 1);
    assert_eq!(anns[0].tag, "TODO");
    assert_eq!(anns[0].message, "drop this column");
}

#[test]
fn test_sample_sql_owner_eve_detected() {
    let path = fixture_path("sample.sql");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.sql");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let eve_ann = anns
        .iter()
        .find(|a| a.owner.as_deref() == Some("eve"))
        .expect("should find annotation owned by eve");

    assert!(eve_ann.is_expired());
}

#[test]
fn test_sample_sql_owner_frank_detected() {
    let path = fixture_path("sample.sql");
    let cfg = default_config();
    let regex = build_regex(&cfg).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let rel = Path::new("tests/fixtures/sample.sql");

    let anns = scan_content(&content, rel, &regex, &cfg, today()).unwrap();
    let frank_ann = anns
        .iter()
        .find(|a| a.owner.as_deref() == Some("frank"))
        .expect("should find annotation owned by frank");

    assert_eq!(frank_ann.status, Status::Ok);
}

// ─── Full directory scan ──────────────────────────────────────────────────────

#[test]
fn test_scan_fixtures_dir_finds_all_files() {
    let dir = fixtures_dir();
    let cfg = default_config();

    let result = scan(&dir, &cfg, today()).unwrap();

    // We have 3 fixture files (sample.rs, sample.py, sample.sql)
    assert_eq!(
        result.scanned_files, 3,
        "expected 3 scanned files in fixtures dir, got {}",
        result.scanned_files
    );
}

#[test]
fn test_scan_fixtures_dir_has_expired() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();
    assert!(
        result.has_expired(),
        "fixtures directory must contain expired annotations"
    );
}

#[test]
fn test_scan_fixtures_dir_total_expired_count() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    // 6 expired per file × 3 files = 18 total expired
    let expired_count = result.expired().len();
    assert_eq!(
        expired_count, 18,
        "expected 18 total expired annotations across all fixture files, got {}",
        expired_count
    );
}

#[test]
fn test_scan_fixtures_dir_sorted_by_date() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    let dates: Vec<_> = result.annotations.iter().map(|a| a.date).collect();
    let mut sorted = dates.clone();
    sorted.sort();
    assert_eq!(
        dates, sorted,
        "scan results must be sorted by date ascending"
    );
}

#[test]
fn test_scan_fixtures_dir_expiring_soon_with_wide_window() {
    let dir = fixtures_dir();
    let cfg = config_with_warn(30);
    let result = scan(&dir, &cfg, today()).unwrap();

    // Each fixture file has 2 expiring-soon items × 3 files = 6
    let soon_count = result.expiring_soon().len();
    assert_eq!(
        soon_count, 6,
        "expected 6 expiring-soon annotations with 30d window across fixture files, got {}",
        soon_count
    );
}

#[test]
fn test_scan_fixtures_dir_has_expiring_soon_with_wide_window() {
    let dir = fixtures_dir();
    let cfg = config_with_warn(30);
    let result = scan(&dir, &cfg, today()).unwrap();
    assert!(
        result.has_expiring_soon(),
        "should detect expiring-soon annotations with 30d window"
    );
}

// ─── Inline annotation pattern tests ─────────────────────────────────────────

#[test]
fn test_all_five_default_tags_matched() {
    let src = "\
// TODO[2020-01-01]: todo expired
// FIXME[2020-01-01]: fixme expired
// HACK[2020-01-01]: hack expired
// TEMP[2020-01-01]: temp expired
// REMOVEME[2020-01-01]: removeme expired
";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("tags.rs"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 5);
    let tags: Vec<&str> = anns.iter().map(|a| a.tag.as_str()).collect();
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
    let anns = scan_str(src, Path::new("case.rs"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 3);
    // All tags should be normalised to upper case
    for ann in &anns {
        assert_eq!(
            ann.tag,
            ann.tag.to_uppercase(),
            "tag should be uppercased: {}",
            ann.tag
        );
    }
}

#[test]
fn test_annotation_on_same_line_as_code() {
    let src = r#"let x = some_func(); // TODO[2020-01-01]: refactor this inline"#;
    let cfg = default_config();
    let anns = scan_str(src, Path::new("inline.rs"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 1);
    assert_eq!(anns[0].message, "refactor this inline");
}

#[test]
fn test_multiple_annotations_on_separate_lines() {
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
    let anns = scan_str(src, Path::new("multi.rs"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 3);
    // Verify line numbers (1-based)
    let lines: Vec<usize> = anns.iter().map(|a| a.line).collect();
    assert!(lines.contains(&2), "line 2 should have TODO");
    assert!(lines.contains(&4), "line 4 should have FIXME");
    assert!(lines.contains(&6), "line 6 should have HACK");
}

#[test]
fn test_owner_with_spaces_trimmed() {
    let src = "// TODO[2020-01-01][ alice ]: trimmed owner\n";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("owner.rs"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 1);
    // The owner is captured as the content between brackets; the scanner trims it
    assert_eq!(anns[0].owner.as_deref(), Some("alice"));
}

#[test]
fn test_invalid_date_produces_no_annotation() {
    // Month 13 is not valid — should warn to stderr and skip
    let src = "// TODO[2026-13-01]: invalid month\n";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("bad.rs"), &cfg, today()).unwrap();
    assert!(
        anns.is_empty(),
        "invalid date should produce no annotation, got {:?}",
        anns
    );
}

#[test]
fn test_invalid_day_produces_no_annotation() {
    let src = "// FIXME[2026-02-30]: invalid day\n";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("bad.rs"), &cfg, today()).unwrap();
    assert!(
        anns.is_empty(),
        "invalid day should produce no annotation, got {:?}",
        anns
    );
}

#[test]
fn test_message_with_colons_and_special_chars() {
    let src = "// TODO[2020-01-01]: fix https://example.com/path?q=1&r=2 handling\n";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("url.rs"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 1);
    assert!(anns[0].message.contains("https://"));
}

#[test]
fn test_file_path_stored_as_relative() {
    let src = "// TODO[2020-01-01]: check path\n";
    let cfg = default_config();
    let rel = Path::new("some/deep/path/file.rs");
    let anns = scan_str(src, rel, &cfg, today()).unwrap();
    assert_eq!(anns[0].file, rel);
}

// ─── Binary file detection ────────────────────────────────────────────────────

/// Verify that the full `scan()` pipeline skips binary files (null bytes) and
/// scans text files — covers the inline binary detection in Phase 2.
#[test]
fn test_scan_skips_binary_files() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();

    // A text file with an annotation — should be scanned.
    let mut text = std::fs::File::create(dir.path().join("ok.rs")).unwrap();
    writeln!(text, "// TODO[2020-01-01]: expired annotation").unwrap();

    // A binary file (contains null byte) — should be skipped.
    let mut bin = std::fs::File::create(dir.path().join("blob.bin")).unwrap();
    bin.write_all(b"ELF\x00binary\x00data").unwrap();

    let cfg = Config {
        extensions: vec!["rs".to_string(), "bin".to_string()],
        ..Config::default()
    };
    let result = scan(dir.path(), &cfg, today()).unwrap();

    assert_eq!(
        result.scanned_files, 1,
        "only the text file should be scanned"
    );
    assert_eq!(
        result.skipped_files, 1,
        "the binary file should be counted as skipped"
    );
    assert_eq!(result.annotations.len(), 1);
}

// ─── Custom tag configuration ─────────────────────────────────────────────────

#[test]
fn test_custom_tag_only_matches_custom() {
    let src = "\
// TODO[2020-01-01]: standard tag — should NOT match
// CUSTOM[2020-01-01]: custom tag — should match
// FIXME[2020-01-01]: another standard — should NOT match
";
    let cfg = Config {
        tags: vec!["CUSTOM".to_string()],
        ..Config::default()
    };
    let anns = scan_str(src, Path::new("custom.rs"), &cfg, today()).unwrap();
    assert_eq!(anns.len(), 1, "only CUSTOM tag should match");
    assert_eq!(anns[0].tag, "CUSTOM");
}

#[test]
fn test_empty_file_produces_no_annotations() {
    let cfg = default_config();
    let anns = scan_str("", Path::new("empty.rs"), &cfg, today()).unwrap();
    assert!(anns.is_empty());
}

#[test]
fn test_file_with_only_plain_todos_produces_no_annotations() {
    let src = "\
// TODO: this
// FIXME: that
// HACK: something
// TODO fix me
// TODO - do this later
";
    let cfg = default_config();
    let anns = scan_str(src, Path::new("plain.rs"), &cfg, today()).unwrap();
    assert!(
        anns.is_empty(),
        "plain TODOs without date brackets must not produce annotations"
    );
}

// ─── scan_result helpers ──────────────────────────────────────────────────────

#[test]
fn test_scan_result_expired_helper() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    let expired = result.expired();
    assert!(!expired.is_empty());
    for ann in &expired {
        assert_eq!(ann.status, Status::Expired);
    }
}

#[test]
fn test_scan_result_ok_helper() {
    let dir = fixtures_dir();
    let cfg = default_config();
    let result = scan(&dir, &cfg, today()).unwrap();

    let ok = result.ok();
    assert!(!ok.is_empty());
    for ann in ok {
        assert_eq!(ann.status, Status::Ok);
    }
}

#[test]
fn test_scan_result_total_equals_sum_of_parts() {
    let dir = fixtures_dir();
    let cfg = config_with_warn(30);
    let result = scan(&dir, &cfg, today()).unwrap();

    let total = result.total();
    let sum = result.expired().len() + result.expiring_soon().len() + result.ok().len();
    assert_eq!(
        total, sum,
        "total() must equal expired + expiring_soon + ok"
    );
}
