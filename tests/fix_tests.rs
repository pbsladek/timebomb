//! Integration tests for the `timebomb fix` subcommand.

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
fn test_fix_no_expired_exits_zero() {
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
fn test_cli_fix_defaults() {
    use clap::Parser;
    let cli = Cli::parse_from(["timebomb", "fix"]);
    match cli.command {
        Command::Fix(args) => {
            assert_eq!(args.path, ".");
            assert!(args.config.is_none());
            assert!(args.warn_within.is_none());
        }
        _ => panic!("expected Fix"),
    }
}

#[test]
fn test_cli_fix_custom_path() {
    use clap::Parser;
    let cli = Cli::parse_from(["timebomb", "fix", "./src"]);
    match cli.command {
        Command::Fix(args) => {
            assert_eq!(args.path, "./src");
        }
        _ => panic!("expected Fix"),
    }
}
