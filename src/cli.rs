use clap::{Parser, Subcommand, ValueEnum};

/// timebomb — enforce expiring TODO/FIXME annotations in source code
#[derive(Debug, Parser)]
#[command(
    name = "timebomb",
    version,
    about = "Scan source code for expiring TODO/FIXME annotations",
    long_about = "timebomb scans your source code for structured TODO/FIXME annotations \
                  with expiry dates and fails in CI when deadlines have passed.\n\n\
                  Annotation format:  // TODO[2026-06-01]: message\n\
                  With owner:         // TODO[2026-06-01][alice]: message"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scan for annotations and exit non-zero if any have expired
    Check(CheckArgs),

    /// List all annotations sorted by expiry date
    List(ListArgs),

    /// Insert a timebomb annotation into a source file
    Add(AddArgs),

    /// Show annotation counts broken down by owner and tag
    Stats(StatsArgs),

    /// Manage the git pre-commit hook
    Hook(HookArgs),

    /// Compare two report JSON snapshots and show annotation debt trajectory
    Trend(TrendArgs),
}

/// Arguments for the `check` subcommand.
#[derive(Debug, clap::Args)]
pub struct CheckArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Warn on annotations expiring within this window (e.g. "30d")
    #[arg(long, value_name = "DURATION")]
    pub warn_within: Option<String>,

    /// Exit with code 1 if any annotations are in the warning window (not just expired)
    #[arg(long, default_value_t = false)]
    pub fail_on_warn: bool,

    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Only report annotations touched in the git diff against this ref (e.g. "HEAD", "main")
    #[arg(long, value_name = "REF")]
    pub since: Option<String>,

    /// Enrich annotations without an explicit owner with git blame author
    #[arg(long)]
    pub blame: bool,
}

/// Arguments for the `list` subcommand.
#[derive(Debug, clap::Args)]
pub struct ListArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Only show expired annotations
    #[arg(long, default_value_t = false)]
    pub expired: bool,

    /// Only show annotations expiring within this window (e.g. "14d")
    #[arg(long, value_name = "DURATION", conflicts_with = "expired")]
    pub expiring_soon: Option<String>,

    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Warn-within threshold used for status classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub warn_within: Option<String>,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Enrich annotations without an explicit owner with git blame author
    #[arg(long)]
    pub blame: bool,
}

/// Arguments for the `add` subcommand.
#[derive(Debug, clap::Args)]
pub struct AddArgs {
    /// File and line to annotate, e.g. "src/main.rs:42"
    #[arg(value_name = "FILE:LINE")]
    pub target: String,

    /// Annotation message
    #[arg(long, value_name = "TEXT")]
    pub message: String,

    /// Tag to use (default: TODO)
    #[arg(long, default_value = "TODO", value_name = "TAG")]
    pub tag: String,

    /// Owner of the annotation, e.g. "alice" or "team-backend"
    #[arg(long, value_name = "OWNER")]
    pub owner: Option<String>,

    /// Expiry date in YYYY-MM-DD format
    #[arg(long, value_name = "YYYY-MM-DD", conflicts_with = "in_days")]
    pub date: Option<String>,

    /// Expiry date as number of days from today
    #[arg(long, value_name = "DAYS", conflicts_with = "date")]
    pub in_days: Option<u32>,

    /// Skip the confirmation prompt and write immediately
    #[arg(long, default_value_t = false)]
    pub yes: bool,
}

/// Arguments for the `stats` subcommand.
#[derive(Debug, clap::Args)]
pub struct StatsArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Group results by this dimension (default: both)
    #[arg(long, value_name = "DIMENSION")]
    pub by: Option<GroupBy>,

    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Warn-within threshold used for status classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub warn_within: Option<String>,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,
}

/// Arguments for the `hook` subcommand.
#[derive(Debug, clap::Args)]
pub struct HookArgs {
    #[command(subcommand)]
    pub command: HookCommand,
}

/// Subcommands under `hook`.
#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Install the timebomb git pre-commit hook
    Install(HookInstallArgs),
    /// Remove the timebomb git pre-commit hook
    Uninstall(HookInstallArgs),
}

/// Arguments for `hook install` / `hook uninstall`.
#[derive(Debug, clap::Args)]
pub struct HookInstallArgs {
    /// Path to the git repository root (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Skip confirmation prompts
    #[arg(short, long)]
    pub yes: bool,
}

/// Arguments for the `trend` subcommand.
#[derive(Debug, clap::Args)]
pub struct TrendArgs {
    /// Path to the earlier report JSON file (baseline)
    pub report_a: String,
    /// Path to the newer report JSON file (current)
    pub report_b: String,
    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,
}

/// The --by flag value for `stats`.
#[derive(Debug, Clone, PartialEq, Eq, ValueEnum)]
pub enum GroupBy {
    /// Break down by annotation owner
    Owner,
    /// Break down by tag (TODO, FIXME, etc.)
    Tag,
}

/// The --format flag value.
#[derive(Debug, Clone, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    /// Human-readable terminal output with color
    Terminal,
    /// Machine-readable JSON
    Json,
    /// GitHub Actions annotation format
    Github,
}

impl FormatArg {
    /// Convert to the `output::OutputFormat` type.
    pub fn to_output_format(&self) -> crate::output::OutputFormat {
        match self {
            FormatArg::Terminal => crate::output::OutputFormat::Terminal,
            FormatArg::Json => crate::output::OutputFormat::Json,
            FormatArg::Github => crate::output::OutputFormat::GitHub,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    // ── check subcommand ──────────────────────────────────────────────────────

    #[test]
    fn test_check_defaults() {
        let cli = parse(&["timebomb", "check"]);
        match cli.command {
            Command::Check(args) => {
                assert_eq!(args.path, ".");
                assert!(args.warn_within.is_none());
                assert!(!args.fail_on_warn);
                assert!(args.format.is_none());
                assert!(args.config.is_none());
                assert!(args.since.is_none());
            }
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_custom_path() {
        let cli = parse(&["timebomb", "check", "./src"]);
        match cli.command {
            Command::Check(args) => assert_eq!(args.path, "./src"),
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_warn_within() {
        let cli = parse(&["timebomb", "check", "--warn-within", "30d"]);
        match cli.command {
            Command::Check(args) => {
                assert_eq!(args.warn_within, Some("30d".to_string()));
            }
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_fail_on_warn() {
        let cli = parse(&["timebomb", "check", "--fail-on-warn"]);
        match cli.command {
            Command::Check(args) => assert!(args.fail_on_warn),
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_format_json() {
        let cli = parse(&["timebomb", "check", "--format", "json"]);
        match cli.command {
            Command::Check(args) => assert_eq!(args.format, Some(FormatArg::Json)),
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_format_github() {
        let cli = parse(&["timebomb", "check", "--format", "github"]);
        match cli.command {
            Command::Check(args) => assert_eq!(args.format, Some(FormatArg::Github)),
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_format_terminal() {
        let cli = parse(&["timebomb", "check", "--format", "terminal"]);
        match cli.command {
            Command::Check(args) => assert_eq!(args.format, Some(FormatArg::Terminal)),
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_config_flag() {
        let cli = parse(&["timebomb", "check", "--config", "my.toml"]);
        match cli.command {
            Command::Check(args) => assert_eq!(args.config, Some("my.toml".to_string())),
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_all_flags_combined() {
        let cli = parse(&[
            "timebomb",
            "check",
            "./src",
            "--warn-within",
            "14d",
            "--fail-on-warn",
            "--format",
            "json",
            "--config",
            ".timebomb.toml",
        ]);
        match cli.command {
            Command::Check(args) => {
                assert_eq!(args.path, "./src");
                assert_eq!(args.warn_within, Some("14d".to_string()));
                assert!(args.fail_on_warn);
                assert_eq!(args.format, Some(FormatArg::Json));
                assert_eq!(args.config, Some(".timebomb.toml".to_string()));
            }
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_since_flag() {
        let cli = parse(&["timebomb", "check", "--since", "main"]);
        match cli.command {
            Command::Check(args) => assert_eq!(args.since, Some("main".to_string())),
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_check_since_head() {
        let cli = parse(&["timebomb", "check", "--since", "HEAD"]);
        match cli.command {
            Command::Check(args) => assert_eq!(args.since, Some("HEAD".to_string())),
            _ => panic!("expected Check"),
        }
    }

    // ── add subcommand ────────────────────────────────────────────────────────

    #[test]
    fn test_add_defaults() {
        let cli = parse(&[
            "timebomb",
            "add",
            "src/main.rs:42",
            "--message",
            "remove this",
        ]);
        match cli.command {
            Command::Add(args) => {
                assert_eq!(args.target, "src/main.rs:42");
                assert_eq!(args.message, "remove this");
                assert_eq!(args.tag, "TODO");
                assert!(args.owner.is_none());
                assert!(args.date.is_none());
                assert!(args.in_days.is_none());
                assert!(!args.yes);
            }
            _ => panic!("expected Add"),
        }
    }

    #[test]
    fn test_add_all_flags() {
        let cli = parse(&[
            "timebomb",
            "add",
            "src/auth.rs:10",
            "--message",
            "remove oauth flow",
            "--tag",
            "FIXME",
            "--owner",
            "alice",
            "--date",
            "2026-09-01",
            "--yes",
        ]);
        match cli.command {
            Command::Add(args) => {
                assert_eq!(args.target, "src/auth.rs:10");
                assert_eq!(args.tag, "FIXME");
                assert_eq!(args.owner, Some("alice".to_string()));
                assert_eq!(args.date, Some("2026-09-01".to_string()));
                assert!(args.yes);
            }
            _ => panic!("expected Add"),
        }
    }

    #[test]
    fn test_add_in_days() {
        let cli = parse(&[
            "timebomb",
            "add",
            "src/lib.rs:1",
            "--message",
            "cleanup",
            "--in-days",
            "90",
        ]);
        match cli.command {
            Command::Add(args) => assert_eq!(args.in_days, Some(90)),
            _ => panic!("expected Add"),
        }
    }

    #[test]
    fn test_add_date_and_in_days_conflict() {
        let result = try_parse(&[
            "timebomb",
            "add",
            "src/lib.rs:1",
            "--message",
            "cleanup",
            "--date",
            "2026-01-01",
            "--in-days",
            "30",
        ]);
        assert!(result.is_err(), "--date and --in-days should conflict");
    }

    // ── stats subcommand ──────────────────────────────────────────────────────

    #[test]
    fn test_stats_defaults() {
        let cli = parse(&["timebomb", "stats"]);
        match cli.command {
            Command::Stats(args) => {
                assert_eq!(args.path, ".");
                assert!(args.by.is_none());
                assert!(args.format.is_none());
                assert!(args.warn_within.is_none());
                assert!(args.config.is_none());
            }
            _ => panic!("expected Stats"),
        }
    }

    #[test]
    fn test_stats_by_owner() {
        let cli = parse(&["timebomb", "stats", "--by", "owner"]);
        match cli.command {
            Command::Stats(args) => assert_eq!(args.by, Some(GroupBy::Owner)),
            _ => panic!("expected Stats"),
        }
    }

    #[test]
    fn test_stats_by_tag() {
        let cli = parse(&["timebomb", "stats", "--by", "tag"]);
        match cli.command {
            Command::Stats(args) => assert_eq!(args.by, Some(GroupBy::Tag)),
            _ => panic!("expected Stats"),
        }
    }

    #[test]
    fn test_stats_all_flags() {
        let cli = parse(&[
            "timebomb",
            "stats",
            "./src",
            "--by",
            "owner",
            "--format",
            "json",
            "--warn-within",
            "14d",
            "--config",
            "custom.toml",
        ]);
        match cli.command {
            Command::Stats(args) => {
                assert_eq!(args.path, "./src");
                assert_eq!(args.by, Some(GroupBy::Owner));
                assert_eq!(args.format, Some(FormatArg::Json));
                assert_eq!(args.warn_within, Some("14d".to_string()));
                assert_eq!(args.config, Some("custom.toml".to_string()));
            }
            _ => panic!("expected Stats"),
        }
    }

    // ── list subcommand ───────────────────────────────────────────────────────

    #[test]
    fn test_list_defaults() {
        let cli = parse(&["timebomb", "list"]);
        match cli.command {
            Command::List(args) => {
                assert_eq!(args.path, ".");
                assert!(!args.expired);
                assert!(args.expiring_soon.is_none());
                assert!(args.format.is_none());
                assert!(args.warn_within.is_none());
                assert!(args.config.is_none());
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn test_list_expired_flag() {
        let cli = parse(&["timebomb", "list", "--expired"]);
        match cli.command {
            Command::List(args) => assert!(args.expired),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn test_list_expiring_soon() {
        let cli = parse(&["timebomb", "list", "--expiring-soon", "14d"]);
        match cli.command {
            Command::List(args) => {
                assert_eq!(args.expiring_soon, Some("14d".to_string()));
                assert!(!args.expired);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn test_list_expired_and_expiring_soon_conflict() {
        // --expired and --expiring-soon should conflict
        let result = try_parse(&["timebomb", "list", "--expired", "--expiring-soon", "14d"]);
        assert!(result.is_err(), "conflicting flags should produce an error");
    }

    #[test]
    fn test_list_format_json() {
        let cli = parse(&["timebomb", "list", "--format", "json"]);
        match cli.command {
            Command::List(args) => assert_eq!(args.format, Some(FormatArg::Json)),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn test_list_warn_within() {
        let cli = parse(&["timebomb", "list", "--warn-within", "7d"]);
        match cli.command {
            Command::List(args) => assert_eq!(args.warn_within, Some("7d".to_string())),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn test_list_custom_path() {
        let cli = parse(&["timebomb", "list", "./my/project"]);
        match cli.command {
            Command::List(args) => assert_eq!(args.path, "./my/project"),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn test_list_all_flags_combined() {
        let cli = parse(&[
            "timebomb",
            "list",
            "./src",
            "--expired",
            "--format",
            "github",
            "--warn-within",
            "30d",
            "--config",
            "custom.toml",
        ]);
        match cli.command {
            Command::List(args) => {
                assert_eq!(args.path, "./src");
                assert!(args.expired);
                assert_eq!(args.format, Some(FormatArg::Github));
                assert_eq!(args.warn_within, Some("30d".to_string()));
                assert_eq!(args.config, Some("custom.toml".to_string()));
            }
            _ => panic!("expected List"),
        }
    }

    // ── FormatArg conversions ─────────────────────────────────────────────────

    #[test]
    fn test_format_arg_to_output_format_terminal() {
        assert_eq!(
            FormatArg::Terminal.to_output_format(),
            crate::output::OutputFormat::Terminal
        );
    }

    #[test]
    fn test_format_arg_to_output_format_json() {
        assert_eq!(
            FormatArg::Json.to_output_format(),
            crate::output::OutputFormat::Json
        );
    }

    #[test]
    fn test_format_arg_to_output_format_github() {
        assert_eq!(
            FormatArg::Github.to_output_format(),
            crate::output::OutputFormat::GitHub
        );
    }

    // ── unknown subcommand ────────────────────────────────────────────────────

    #[test]
    fn test_unknown_subcommand_is_error() {
        let result = try_parse(&["timebomb", "run"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_subcommand_is_error() {
        let result = try_parse(&["timebomb"]);
        assert!(result.is_err());
    }
}
