//! Integration tests for the `timebomb defuse` subcommand.

use chrono::NaiveDate;
use std::io::Write;
use timebomb::cli::{Cli, Command};

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn today() -> NaiveDate {
    date("2026-03-22")
}

// ── run_fix integration tests ────────────────────────────────────────────────

#[test]
fn test_defuse_no_detonated_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join("ok.rs")).unwrap();
    writeln!(f, "// TODO[2099-01-01]: far future, not expired").unwrap();

    let cfg = timebomb::config::Config::default();
    let summary = timebomb::fix::run_fix(dir.path(), &cfg, today()).unwrap();

    assert_eq!(summary.extended, 0);
    assert_eq!(summary.deleted, 0);
    assert_eq!(summary.skipped, 0);
}

// ── CLI argument parsing tests ───────────────────────────────────────────────

#[test]
fn test_cli_defuse_defaults() {
    use clap::Parser;
    let cli = Cli::parse_from(["timebomb", "defuse"]);
    match cli.command {
        Command::Defuse(args) => {
            assert_eq!(args.path, ".");
            assert!(args.config.is_none());
            assert!(args.fuse.is_none());
        }
        _ => panic!("expected Defuse"),
    }
}

#[test]
fn test_cli_defuse_custom_path() {
    use clap::Parser;
    let cli = Cli::parse_from(["timebomb", "defuse", "./src"]);
    match cli.command {
        Command::Defuse(args) => {
            assert_eq!(args.path, "./src");
        }
        _ => panic!("expected Defuse"),
    }
}
