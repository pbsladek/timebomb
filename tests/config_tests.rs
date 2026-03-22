//! Integration tests for config loading and merging.
//!
//! These tests exercise the full config loading pipeline including reading from
//! actual `.timebomb.toml` files on disk and merging with CLI overrides.

use std::io::Write;
use std::path::Path;

use timebomb::config::{load_config, CliOverrides, Config};

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Write a `.timebomb.toml` into a temp directory and return the dir.
fn write_config(content: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join(".timebomb.toml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(f, "{}", content).unwrap();
    dir
}

fn no_overrides() -> CliOverrides {
    CliOverrides::default()
}

// ─── Default config ───────────────────────────────────────────────────────────

#[test]
fn test_default_config_tags() {
    let cfg = Config::default();
    let tags = &cfg.tags;
    assert!(tags.contains(&"TODO".to_string()));
    assert!(tags.contains(&"FIXME".to_string()));
    assert!(tags.contains(&"HACK".to_string()));
    assert!(tags.contains(&"TEMP".to_string()));
    assert!(tags.contains(&"REMOVEME".to_string()));
    assert_eq!(tags.len(), 5);
}

#[test]
fn test_default_config_warn_within_days_is_zero() {
    let cfg = Config::default();
    assert_eq!(cfg.warn_within_days, 0);
}

#[test]
fn test_default_config_fail_on_warn_is_false() {
    let cfg = Config::default();
    assert!(!cfg.fail_on_warn);
}

#[test]
fn test_default_config_extensions_non_empty() {
    let cfg = Config::default();
    assert!(!cfg.extensions.is_empty());
}

#[test]
fn test_default_config_extensions_contain_common_types() {
    let cfg = Config::default();
    let exts = &cfg.extensions;
    for expected in &[
        "rs", "go", "ts", "js", "py", "rb", "java", "sql", "yaml", "yml",
    ] {
        assert!(
            exts.contains(&expected.to_string()),
            "expected extension '{}' to be in default list",
            expected
        );
    }
}

#[test]
fn test_default_config_excludes_contain_git() {
    let cfg = Config::default();
    assert!(
        cfg.exclude_patterns.iter().any(|p| p.contains(".git")),
        "default excludes should contain .git/**"
    );
}

#[test]
fn test_default_config_excludes_contain_node_modules() {
    let cfg = Config::default();
    assert!(
        cfg.exclude_patterns
            .iter()
            .any(|p| p.contains("node_modules")),
        "default excludes should contain node_modules/**"
    );
}

#[test]
fn test_default_config_excludes_contain_vendor() {
    let cfg = Config::default();
    assert!(
        cfg.exclude_patterns.iter().any(|p| p.contains("vendor")),
        "default excludes should contain vendor/**"
    );
}

// ─── load_config: no config file present ─────────────────────────────────────

#[test]
fn test_load_config_no_file_uses_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.tags, Config::default().tags);
    assert_eq!(cfg.warn_within_days, 0);
    assert!(!cfg.fail_on_warn);
}

#[test]
fn test_load_config_no_file_no_overrides_default_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert!(cfg.extensions.contains(&"rs".to_string()));
    assert!(cfg.extensions.contains(&"py".to_string()));
}

// ─── load_config: config file present ────────────────────────────────────────

#[test]
fn test_load_config_reads_tags_from_file() {
    let dir = write_config(r#"tags = ["TODO", "FIXME"]"#);
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.tags, vec!["TODO", "FIXME"]);
}

#[test]
fn test_load_config_reads_warn_within_days() {
    let dir = write_config("warn_within_days = 21\n");
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.warn_within_days, 21);
}

#[test]
fn test_load_config_reads_exclude_patterns() {
    let dir = write_config(r#"exclude = ["build/**", "dist/**"]"#);
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert!(cfg.exclude_patterns.contains(&"build/**".to_string()));
    assert!(cfg.exclude_patterns.contains(&"dist/**".to_string()));
}

#[test]
fn test_load_config_reads_extensions() {
    let dir = write_config(r#"extensions = ["rs", "toml"]"#);
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.extensions, vec!["rs", "toml"]);
}

#[test]
fn test_load_config_partial_file_fills_rest_from_defaults() {
    // Only warn_within_days specified; tags, exclude, extensions should come from defaults
    let dir = write_config("warn_within_days = 7\n");
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.warn_within_days, 7);
    // Tags should fall back to defaults
    assert!(cfg.tags.contains(&"TODO".to_string()));
    assert!(cfg.tags.contains(&"FIXME".to_string()));
    // Extensions should fall back to defaults
    assert!(cfg.extensions.contains(&"rs".to_string()));
}

#[test]
fn test_load_config_empty_file_uses_defaults() {
    let dir = write_config("");
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.tags, Config::default().tags);
    assert_eq!(cfg.warn_within_days, 0);
}

#[test]
fn test_load_config_full_file() {
    let toml = r#"
tags = ["TODO", "FIXME", "HACK"]
warn_within_days = 14
exclude = ["vendor/**", "node_modules/**", ".git/**"]
extensions = ["rs", "go", "py"]
"#;
    let dir = write_config(toml);
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();

    assert_eq!(cfg.tags, vec!["TODO", "FIXME", "HACK"]);
    assert_eq!(cfg.warn_within_days, 14);
    assert!(cfg.exclude_patterns.contains(&"vendor/**".to_string()));
    assert_eq!(cfg.extensions, vec!["rs", "go", "py"]);
}

// ─── load_config: invalid config file ────────────────────────────────────────

#[test]
fn test_load_config_invalid_toml_returns_error() {
    let dir = write_config("this is not valid toml ][[[");
    let result = load_config(dir.path(), &no_overrides());
    assert!(result.is_err(), "invalid TOML should return an error");
}

#[test]
fn test_load_config_unknown_field_returns_error() {
    // We use deny_unknown_fields in ConfigFile
    let dir = write_config("unknown_key = true\n");
    let result = load_config(dir.path(), &no_overrides());
    assert!(
        result.is_err(),
        "unknown field in config should return an error due to deny_unknown_fields"
    );
}

#[test]
fn test_load_config_wrong_type_returns_error() {
    // warn_within_days should be an integer, not a string
    let dir = write_config(r#"warn_within_days = "fourteen""#);
    let result = load_config(dir.path(), &no_overrides());
    assert!(
        result.is_err(),
        "wrong type for warn_within_days should error"
    );
}

// ─── CLI overrides ────────────────────────────────────────────────────────────

#[test]
fn test_cli_override_warn_within_overrides_file() {
    let dir = write_config("warn_within_days = 7\n");
    let overrides = CliOverrides::new(Some("30d".to_string()), false);
    let cfg = load_config(dir.path(), &overrides).unwrap();
    assert_eq!(
        cfg.warn_within_days, 30,
        "CLI --warn-within should override config file"
    );
}

#[test]
fn test_cli_override_warn_within_overrides_default() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = CliOverrides::new(Some("14d".to_string()), false);
    let cfg = load_config(dir.path(), &overrides).unwrap();
    assert_eq!(cfg.warn_within_days, 14);
}

#[test]
fn test_cli_override_fail_on_warn_sets_flag() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = CliOverrides::new(None, true);
    let cfg = load_config(dir.path(), &overrides).unwrap();
    assert!(cfg.fail_on_warn);
}

#[test]
fn test_cli_override_fail_on_warn_false_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert!(!cfg.fail_on_warn);
}

#[test]
fn test_cli_override_invalid_duration_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = CliOverrides::new(Some("notvalid".to_string()), false);
    let result = load_config(dir.path(), &overrides);
    assert!(
        result.is_err(),
        "invalid duration string should return error"
    );
}

#[test]
fn test_cli_override_duration_without_d_suffix_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = CliOverrides::new(Some("30".to_string()), false);
    let result = load_config(dir.path(), &overrides);
    assert!(
        result.is_err(),
        "duration without 'd' suffix should return error"
    );
}

#[test]
fn test_cli_override_zero_days_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = CliOverrides::new(Some("0d".to_string()), false);
    let cfg = load_config(dir.path(), &overrides).unwrap();
    assert_eq!(cfg.warn_within_days, 0);
}

#[test]
fn test_cli_override_large_days_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = CliOverrides::new(Some("365d".to_string()), false);
    let cfg = load_config(dir.path(), &overrides).unwrap();
    assert_eq!(cfg.warn_within_days, 365);
}

// ─── GlobSet / extension helpers ─────────────────────────────────────────────

#[test]
fn test_build_exclude_globset_succeeds_with_defaults() {
    let cfg = Config::default();
    let result = cfg.build_exclude_globset();
    assert!(
        result.is_ok(),
        "building default exclude globset should succeed"
    );
}

#[test]
fn test_build_exclude_globset_invalid_pattern_returns_error() {
    let cfg = Config {
        exclude_patterns: vec!["[invalid".to_string()],
        ..Config::default()
    };
    let result = cfg.build_exclude_globset();
    assert!(result.is_err(), "invalid glob pattern should return error");
}

#[test]
fn test_is_excluded_git_dir() {
    let cfg = Config::default();
    let gs = cfg.build_exclude_globset().unwrap();
    assert!(cfg.is_excluded(Path::new(".git/config"), &gs));
    assert!(cfg.is_excluded(Path::new(".git/objects/abc123"), &gs));
}

#[test]
fn test_is_excluded_node_modules() {
    let cfg = Config::default();
    let gs = cfg.build_exclude_globset().unwrap();
    assert!(cfg.is_excluded(Path::new("node_modules/react/index.js"), &gs));
}

#[test]
fn test_is_excluded_vendor() {
    let cfg = Config::default();
    let gs = cfg.build_exclude_globset().unwrap();
    assert!(cfg.is_excluded(Path::new("vendor/github.com/some/pkg/file.go"), &gs));
}

#[test]
fn test_is_excluded_min_js() {
    let cfg = Config::default();
    let gs = cfg.build_exclude_globset().unwrap();
    assert!(cfg.is_excluded(Path::new("bundle.min.js"), &gs));
}

#[test]
fn test_is_not_excluded_src_file() {
    let cfg = Config::default();
    let gs = cfg.build_exclude_globset().unwrap();
    assert!(!cfg.is_excluded(Path::new("src/main.rs"), &gs));
    assert!(!cfg.is_excluded(Path::new("src/lib/util.go"), &gs));
}

#[test]
fn test_is_not_excluded_root_file() {
    let cfg = Config::default();
    let gs = cfg.build_exclude_globset().unwrap();
    assert!(!cfg.is_excluded(Path::new("main.rs"), &gs));
    assert!(!cfg.is_excluded(Path::new("README.md"), &gs));
}

#[test]
fn test_extension_allowed_rs() {
    let cfg = Config::default();
    assert!(cfg.extension_allowed(Path::new("src/main.rs")));
}

#[test]
fn test_extension_allowed_go() {
    let cfg = Config::default();
    assert!(cfg.extension_allowed(Path::new("pkg/server.go")));
}

#[test]
fn test_extension_allowed_py() {
    let cfg = Config::default();
    assert!(cfg.extension_allowed(Path::new("scripts/deploy.py")));
}

#[test]
fn test_extension_allowed_sql() {
    let cfg = Config::default();
    assert!(cfg.extension_allowed(Path::new("db/schema.sql")));
}

#[test]
fn test_extension_allowed_yaml() {
    let cfg = Config::default();
    assert!(cfg.extension_allowed(Path::new(".github/workflows/ci.yml")));
    assert!(cfg.extension_allowed(Path::new("config/app.yaml")));
}

#[test]
fn test_extension_not_allowed_xyz() {
    let cfg = Config::default();
    assert!(!cfg.extension_allowed(Path::new("data.xyz")));
}

#[test]
fn test_extension_not_allowed_no_extension() {
    let cfg = Config::default();
    assert!(!cfg.extension_allowed(Path::new("Makefile")));
    assert!(!cfg.extension_allowed(Path::new("Dockerfile")));
}

#[test]
fn test_extension_allowed_case_insensitive() {
    let cfg = Config::default();
    // .RS should match "rs" in the list
    assert!(cfg.extension_allowed(Path::new("MAIN.RS")));
}

#[test]
fn test_extension_allowed_empty_list_allows_all() {
    let cfg = Config {
        extensions: vec![],
        ..Config::default()
    };
    assert!(cfg.extension_allowed(Path::new("anything.xyz")));
    assert!(cfg.extension_allowed(Path::new("Makefile")));
    assert!(cfg.extension_allowed(Path::new("binary.exe")));
}

// ─── annotation_regex_pattern ────────────────────────────────────────────────

#[test]
fn test_annotation_regex_pattern_contains_all_default_tags() {
    let cfg = Config::default();
    let pattern = cfg.annotation_regex_pattern();
    for tag in &["TODO", "FIXME", "HACK", "TEMP", "REMOVEME"] {
        assert!(
            pattern.contains(tag),
            "pattern should contain tag '{}'",
            tag
        );
    }
}

#[test]
fn test_annotation_regex_pattern_custom_tags() {
    let cfg = Config {
        tags: vec!["MYTAGONE".to_string(), "MYTAGTWO".to_string()],
        ..Config::default()
    };
    let pattern = cfg.annotation_regex_pattern();
    assert!(pattern.contains("MYTAGONE"));
    assert!(pattern.contains("MYTAGTWO"));
    assert!(
        !pattern.contains("TODO"),
        "TODO should not be in custom-only pattern"
    );
}

#[test]
fn test_annotation_regex_pattern_is_valid_regex() {
    let cfg = Config::default();
    let pattern = cfg.annotation_regex_pattern();
    let result = regex::Regex::new(&pattern);
    assert!(
        result.is_ok(),
        "annotation_regex_pattern must produce a valid regex: {:?}",
        result.err()
    );
}

#[test]
fn test_annotation_regex_pattern_matches_basic_annotation() {
    let cfg = Config::default();
    let pattern = cfg.annotation_regex_pattern();
    let re = regex::Regex::new(&pattern).unwrap();
    assert!(re.is_match("// TODO[2020-01-01]: some message"));
    assert!(re.is_match("# FIXME[2099-12-31]: future task"));
    assert!(re.is_match("-- HACK[2018-06-01]: sql comment"));
}

#[test]
fn test_annotation_regex_pattern_does_not_match_plain_todo() {
    let cfg = Config::default();
    let pattern = cfg.annotation_regex_pattern();
    let re = regex::Regex::new(&pattern).unwrap();
    assert!(!re.is_match("// TODO: no date"));
    assert!(!re.is_match("// FIXME: no date bracket"));
    assert!(!re.is_match("// TODO [2020-01-01]: space before bracket"));
}

#[test]
fn test_annotation_regex_pattern_matches_with_owner() {
    let cfg = Config::default();
    let pattern = cfg.annotation_regex_pattern();
    let re = regex::Regex::new(&pattern).unwrap();
    assert!(re.is_match("// TODO[2020-01-01][alice]: owned annotation"));
    assert!(re.is_match("# FIXME[2099-01-01][bob]: another owned"));
}

// ─── CliOverrides construction ────────────────────────────────────────────────

#[test]
fn test_cli_overrides_default() {
    let overrides = CliOverrides::default();
    assert!(overrides.warn_within.is_none());
    assert!(!overrides.fail_on_warn);
}

#[test]
fn test_cli_overrides_new_with_values() {
    let overrides = CliOverrides::new(Some("30d".to_string()), true);
    assert_eq!(overrides.warn_within, Some("30d".to_string()));
    assert!(overrides.fail_on_warn);
}

#[test]
fn test_cli_overrides_new_without_warn_within() {
    let overrides = CliOverrides::new(None, false);
    assert!(overrides.warn_within.is_none());
    assert!(!overrides.fail_on_warn);
}

// ─── Config interaction with scanner ─────────────────────────────────────────

#[test]
fn test_config_from_file_integrates_with_scanner() {
    use std::io::Write as _;
    use timebomb::scanner::scan;

    // Write a config that only scans .rs files and has a 14-day warn window
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join(".timebomb.toml");
    {
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, r#"extensions = ["rs"]"#).unwrap();
        writeln!(f, "warn_within_days = 14").unwrap();
    }

    // Create a .rs file with annotations
    let src_path = dir.path().join("main.rs");
    {
        let mut f = std::fs::File::create(&src_path).unwrap();
        writeln!(f, "// TODO[2020-01-01]: expired").unwrap();
        writeln!(f, "// FIXME[2099-01-01]: future").unwrap();
    }

    // Create a .py file with annotations — should be ignored per config
    let py_path = dir.path().join("script.py");
    {
        let mut f = std::fs::File::create(&py_path).unwrap();
        writeln!(f, "# TODO[2020-01-01]: python should be ignored").unwrap();
    }

    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let today = chrono::NaiveDate::parse_from_str("2025-06-01", "%Y-%m-%d").unwrap();
    let result = scan(dir.path(), &cfg, today).unwrap();

    // Only main.rs should be scanned
    assert_eq!(
        result.scanned_files, 1,
        "only .rs file should be scanned per config extensions"
    );
    assert_eq!(result.annotations.len(), 2);
    assert!(result.has_expired());
}

#[test]
fn test_config_exclude_pattern_integrates_with_scanner() {
    use std::io::Write as _;
    use timebomb::scanner::scan;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join(".timebomb.toml");
    {
        let mut f = std::fs::File::create(&config_path).unwrap();
        // Exclude everything under "generated/"
        writeln!(f, r#"exclude = ["generated/**"]"#).unwrap();
    }

    // File that should be scanned
    let src_path = dir.path().join("main.rs");
    {
        let mut f = std::fs::File::create(&src_path).unwrap();
        writeln!(f, "// TODO[2020-01-01]: should be found").unwrap();
    }

    // File in excluded directory
    std::fs::create_dir(dir.path().join("generated")).unwrap();
    let gen_path = dir.path().join("generated").join("auto.rs");
    {
        let mut f = std::fs::File::create(&gen_path).unwrap();
        writeln!(f, "// TODO[2020-01-01]: should be excluded").unwrap();
    }

    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let today = chrono::NaiveDate::parse_from_str("2025-06-01", "%Y-%m-%d").unwrap();
    let result = scan(dir.path(), &cfg, today).unwrap();

    assert_eq!(
        result.scanned_files, 1,
        "generated/ directory should be excluded"
    );
    // The one annotation found should be from main.rs
    assert_eq!(result.annotations.len(), 1);
    assert_eq!(result.annotations[0].file, std::path::Path::new("main.rs"));
}
