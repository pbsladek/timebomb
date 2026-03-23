//! Integration tests for the `timebomb bunker` feature and ratchet enforcement.

use std::io::Write as _;
use std::path::Path;

use chrono::NaiveDate;
use timebomb::baseline::{
    check_ratchet, load_baseline, run_baseline_save, run_baseline_show, Baseline,
};
use timebomb::cli::Cli;
use timebomb::config::{load_config, CliOverrides};

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn fixed_today() -> NaiveDate {
    NaiveDate::parse_from_str("2025-06-01", "%Y-%m-%d").unwrap()
}

fn no_overrides() -> CliOverrides {
    CliOverrides::default()
}

/// Write a `.timebomb.toml` into a temp directory and return the dir.
fn write_config(content: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join(".timebomb.toml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    write!(f, "{}", content).unwrap();
    dir
}

// ─── Config field tests ───────────────────────────────────────────────────────

#[test]
fn test_config_max_detonated_parsed() {
    let dir = write_config("max_detonated = 5\n");
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.max_detonated, Some(5));
}

#[test]
fn test_config_max_ticking_parsed() {
    let dir = write_config("max_ticking = 10\n");
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.max_ticking, Some(10));
}

#[test]
fn test_config_max_fields_default_none() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.max_detonated, None);
    assert_eq!(cfg.max_ticking, None);
}

#[test]
fn test_config_both_max_fields_parsed() {
    let dir = write_config("max_detonated = 3\nmax_ticking = 7\n");
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    assert_eq!(cfg.max_detonated, Some(3));
    assert_eq!(cfg.max_ticking, Some(7));
}

// ─── baseline save tests ──────────────────────────────────────────────────────

#[test]
fn test_baseline_save_creates_file() {
    let dir = tempfile::tempdir().unwrap();

    // Write a file with one detonated fuse
    let mut f = std::fs::File::create(dir.path().join("main.rs")).unwrap();
    writeln!(f, "// TODO[2020-01-01]: expired").unwrap();

    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let baseline_path = dir.path().join(".timebomb-baseline.json");
    let generated_at = "2025-06-01T12:00:00Z";

    let code = run_baseline_save(
        dir.path(),
        &cfg,
        fixed_today(),
        &baseline_path,
        generated_at,
    )
    .unwrap();
    assert_eq!(code, 0);

    // File must exist and be parseable
    assert!(baseline_path.exists());
    let loaded = load_baseline(&baseline_path).unwrap().unwrap();
    assert_eq!(loaded.detonated, 1);
    assert_eq!(loaded.ticking, 0);
    assert_eq!(loaded.generated_at, generated_at);
}

#[test]
fn test_baseline_save_empty_dir_writes_zeros() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let baseline_path = dir.path().join(".timebomb-baseline.json");

    run_baseline_save(dir.path(), &cfg, fixed_today(), &baseline_path, "ts").unwrap();

    let loaded = load_baseline(&baseline_path).unwrap().unwrap();
    assert_eq!(loaded.detonated, 0);
    assert_eq!(loaded.ticking, 0);
}

// ─── baseline ratchet enforcement via `sweep` ─────────────────────────────────

#[test]
fn test_sweep_exits_one_when_ratchet_violated() {
    use clap::Parser;

    let dir = tempfile::tempdir().unwrap();

    // Save a baseline with 0 detonated
    {
        let baseline = Baseline {
            generated_at: "2025-01-01T00:00:00Z".to_string(),
            detonated: 0,
            ticking: 0,
        };
        timebomb::baseline::save_baseline(&baseline, &dir.path().join(".timebomb-baseline.json"))
            .unwrap();
    }

    // Write a file with 1 detonated fuse — violates baseline
    {
        let mut f = std::fs::File::create(dir.path().join("main.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: expired").unwrap();
    }

    let cli = Cli::parse_from(["timebomb", "sweep", dir.path().to_str().unwrap()]);
    // run is private; invoke via the binary entry point through the public API instead
    // by constructing the args and calling sweep directly via library code
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let result = timebomb::scanner::scan(dir.path(), &cfg, fixed_today()).unwrap();
    let baseline = load_baseline(&dir.path().join(".timebomb-baseline.json"))
        .unwrap()
        .unwrap();

    let violations = check_ratchet(
        result.detonated().len(),
        result.ticking().len(),
        Some(&baseline),
        cfg.max_detonated,
        cfg.max_ticking,
    );
    assert!(!violations.is_empty(), "ratchet should be violated");
    // Also confirm cli parses without error
    let _ = cli;
}

#[test]
fn test_sweep_exits_zero_within_baseline() {
    let dir = tempfile::tempdir().unwrap();

    // Save a baseline with 2 detonated
    {
        let baseline = Baseline {
            generated_at: "2025-01-01T00:00:00Z".to_string(),
            detonated: 2,
            ticking: 0,
        };
        timebomb::baseline::save_baseline(&baseline, &dir.path().join(".timebomb-baseline.json"))
            .unwrap();
    }

    // Write a file with exactly 2 detonated fuses — same as baseline
    {
        let mut f = std::fs::File::create(dir.path().join("main.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: expired one").unwrap();
        writeln!(f, "// FIXME[2019-01-01]: expired two").unwrap();
    }

    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let result = timebomb::scanner::scan(dir.path(), &cfg, fixed_today()).unwrap();
    let baseline = load_baseline(&dir.path().join(".timebomb-baseline.json"))
        .unwrap()
        .unwrap();

    let violations = check_ratchet(
        result.detonated().len(),
        result.ticking().len(),
        Some(&baseline),
        cfg.max_detonated,
        cfg.max_ticking,
    );
    assert!(
        violations.is_empty(),
        "no ratchet violation when count matches baseline: {:?}",
        violations
    );
}

// ─── baseline show tests ──────────────────────────────────────────────────────

#[test]
fn test_baseline_show_no_file_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let baseline_path = dir.path().join(".timebomb-baseline.json");

    // No baseline file — should still succeed with exit code 0
    let code = run_baseline_show(dir.path(), &cfg, fixed_today(), &baseline_path).unwrap();
    assert_eq!(code, 0);
}

#[test]
fn test_baseline_show_with_file_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let baseline_path = dir.path().join(".timebomb-baseline.json");

    // Save a baseline first
    run_baseline_save(dir.path(), &cfg, fixed_today(), &baseline_path, "ts").unwrap();

    let code = run_baseline_show(dir.path(), &cfg, fixed_today(), &baseline_path).unwrap();
    assert_eq!(code, 0);
}

// ─── CLI parsing tests ────────────────────────────────────────────────────────

#[test]
fn test_cli_bunker_save_defaults() {
    use clap::Parser;
    use timebomb::cli::{BaselineCommand, Command};

    let cli = Cli::parse_from(["timebomb", "bunker", "save"]);
    match cli.command {
        Command::Bunker(args) => match args.command {
            BaselineCommand::Save(a) => {
                assert_eq!(a.path, ".");
                assert_eq!(a.baseline_file, ".timebomb-baseline.json");
                assert!(a.config.is_none());
                assert!(a.fuse.is_none());
            }
            _ => panic!("expected Save"),
        },
        _ => panic!("expected Bunker"),
    }
}

#[test]
fn test_cli_bunker_show_defaults() {
    use clap::Parser;
    use timebomb::cli::{BaselineCommand, Command};

    let cli = Cli::parse_from(["timebomb", "bunker", "show"]);
    match cli.command {
        Command::Bunker(args) => match args.command {
            BaselineCommand::Show(a) => {
                assert_eq!(a.path, ".");
                assert_eq!(a.baseline_file, ".timebomb-baseline.json");
                assert!(a.config.is_none());
                assert!(a.fuse.is_none());
            }
            _ => panic!("expected Show"),
        },
        _ => panic!("expected Bunker"),
    }
}

#[test]
fn test_cli_bunker_save_custom_file() {
    use clap::Parser;
    use timebomb::cli::{BaselineCommand, Command};

    let cli = Cli::parse_from([
        "timebomb",
        "bunker",
        "save",
        "--baseline-file",
        "custom.json",
    ]);
    match cli.command {
        Command::Bunker(args) => match args.command {
            BaselineCommand::Save(a) => {
                assert_eq!(a.baseline_file, "custom.json");
            }
            _ => panic!("expected Save"),
        },
        _ => panic!("expected Bunker"),
    }
}

#[test]
fn test_cli_bunker_show_custom_file() {
    use clap::Parser;
    use timebomb::cli::{BaselineCommand, Command};

    let cli = Cli::parse_from([
        "timebomb",
        "bunker",
        "show",
        "--baseline-file",
        "custom.json",
    ]);
    match cli.command {
        Command::Bunker(args) => match args.command {
            BaselineCommand::Show(a) => {
                assert_eq!(a.baseline_file, "custom.json");
            }
            _ => panic!("expected Show"),
        },
        _ => panic!("expected Bunker"),
    }
}

#[test]
fn test_cli_bunker_save_with_path_and_fuse() {
    use clap::Parser;
    use timebomb::cli::{BaselineCommand, Command};

    let cli = Cli::parse_from([
        "timebomb",
        "bunker",
        "save",
        "./src",
        "--fuse",
        "14d",
    ]);
    match cli.command {
        Command::Bunker(args) => match args.command {
            BaselineCommand::Save(a) => {
                assert_eq!(a.path, "./src");
                assert_eq!(a.fuse, Some("14d".to_string()));
            }
            _ => panic!("expected Save"),
        },
        _ => panic!("expected Bunker"),
    }
}

// ─── max_detonated config ratchet tests ─────────────────────────────────────────

#[test]
fn test_ratchet_max_detonated_from_config_causes_violation() {
    let dir = write_config("max_detonated = 0\n");

    // Write a file with 1 detonated fuse — exceeds max_detonated=0
    {
        let mut f = std::fs::File::create(dir.path().join("main.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: expired").unwrap();
    }

    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let result = timebomb::scanner::scan(dir.path(), &cfg, fixed_today()).unwrap();

    let violations = check_ratchet(
        result.detonated().len(),
        result.ticking().len(),
        None,
        cfg.max_detonated,
        cfg.max_ticking,
    );
    assert!(
        !violations.is_empty(),
        "should violate max_detonated=0 with 1 detonated"
    );
}

#[test]
fn test_ratchet_max_detonated_from_config_no_violation_at_limit() {
    let dir = write_config("max_detonated = 1\n");

    // Write a file with 1 detonated fuse — exactly at limit
    {
        let mut f = std::fs::File::create(dir.path().join("main.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: expired").unwrap();
    }

    let cfg = load_config(dir.path(), &no_overrides()).unwrap();
    let result = timebomb::scanner::scan(dir.path(), &cfg, fixed_today()).unwrap();

    let violations = check_ratchet(
        result.detonated().len(),
        result.ticking().len(),
        None,
        cfg.max_detonated,
        cfg.max_ticking,
    );
    assert!(
        violations.is_empty(),
        "should not violate max_detonated=1 with 1 detonated: {:?}",
        violations
    );
}

// ─── check_ratchet pure function tests (additional coverage) ──────────────────

#[test]
fn test_check_ratchet_all_zero_no_baseline() {
    let violations = check_ratchet(0, 0, None, None, None);
    assert!(violations.is_empty());
}

#[test]
fn test_check_ratchet_four_violations_at_once() {
    let baseline = Baseline {
        generated_at: "2025-01-01T00:00:00Z".to_string(),
        detonated: 1,
        ticking: 1,
    };
    // detonated=5 > max_detonated=2 AND > baseline=1
    // ticking=10 > max_ticking=5 AND > baseline=1
    let violations = check_ratchet(5, 10, Some(&baseline), Some(2), Some(5));
    assert_eq!(violations.len(), 4);
}

#[test]
fn test_check_ratchet_baseline_equal_not_violated() {
    let baseline = Baseline {
        generated_at: "2025-01-01T00:00:00Z".to_string(),
        detonated: 3,
        ticking: 2,
    };
    // Equal counts should not trigger ratchet
    let violations = check_ratchet(3, 2, Some(&baseline), None, None);
    assert!(violations.is_empty());
}

/// Check that load_baseline on a directory with a valid .timebomb-baseline.json
/// roundtrips correctly through save_baseline.
#[test]
fn test_load_baseline_from_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".timebomb-baseline.json");

    let original = Baseline {
        generated_at: "2025-06-01T00:00:00+00:00".to_string(),
        detonated: 7,
        ticking: 3,
    };
    timebomb::baseline::save_baseline(&original, &path).unwrap();

    let loaded = load_baseline(&path).unwrap().unwrap();
    assert_eq!(loaded.detonated, 7);
    assert_eq!(loaded.ticking, 3);
    assert_eq!(loaded.generated_at, original.generated_at);
}

/// Confirm the baseline file is valid JSON (not just TOML or other format).
#[test]
fn test_baseline_file_is_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline.json");

    let baseline = Baseline {
        generated_at: "2025-01-01T00:00:00Z".to_string(),
        detonated: 1,
        ticking: 2,
    };
    timebomb::baseline::save_baseline(&baseline, &path).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["detonated"], 1);
    assert_eq!(parsed["ticking"], 2);
    assert_eq!(parsed["generated_at"], "2025-01-01T00:00:00Z");
}

/// Verify load_baseline returns Err when the file exists but contains invalid JSON.
#[test]
fn test_load_baseline_corrupt_file_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, b"not json at all").unwrap();

    let result = load_baseline(Path::new(path.to_str().unwrap()));
    assert!(result.is_err(), "corrupt JSON should return Err");
}
