use clap::Parser;
use timebomb::cli::{Cli, Command};

fn parse(args: &[&str]) -> Cli {
    Cli::parse_from(args)
}

fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(args)
}

#[test]
fn test_cli_sweep_changed_flag() {
    let cli = parse(&["timebomb", "sweep", "--changed"]);
    match cli.command {
        Command::Sweep(args) => {
            assert!(args.changed, "--changed should be true");
            assert!(args.base.is_none(), "--base should default to None");
        }
        _ => panic!("expected Sweep"),
    }
}

#[test]
fn test_cli_sweep_changed_with_base() {
    let cli = parse(&["timebomb", "sweep", "--changed", "--base", "origin/main"]);
    match cli.command {
        Command::Sweep(args) => {
            assert!(args.changed, "--changed should be true");
            assert_eq!(
                args.base,
                Some("origin/main".to_string()),
                "--base should be origin/main"
            );
        }
        _ => panic!("expected Sweep"),
    }
}

#[test]
fn test_cli_sweep_base_requires_changed() {
    // --base without --changed should be rejected by clap (requires = "changed")
    let result = try_parse(&["timebomb", "sweep", "--base", "origin/main"]);
    assert!(
        result.is_err(),
        "--base without --changed should produce a clap error"
    );
}
