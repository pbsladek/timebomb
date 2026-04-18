use clap::{Parser, Subcommand, ValueEnum};
pub use clap_complete::Shell;

/// timebomb — enforce expiring TODO/FIXME fuses in source code
#[derive(Debug, Parser)]
#[command(
    name = "timebomb",
    version,
    about = "Sweep source code for ticking fuses and detonate in CI when deadlines pass",
    long_about = "timebomb sweeps your source code for structured TODO/FIXME fuses \
                  with expiry dates and fails in CI when deadlines have passed.\n\n\
                  Fuse format:  // TODO[2026-06-01]: message\n\
                  With owner:   // TODO[2026-06-01][alice]: message"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sweep for fuses and exit non-zero if any have detonated
    Sweep(SweepArgs),

    /// List all fuses sorted by expiry date
    Manifest(ManifestArgs),

    /// Show the most urgent detonated and ticking fuses
    Armory(ArmoryArgs),

    /// Insert a timebomb fuse into a source file
    Plant(PlantArgs),

    /// Bump the expiry date on an existing fuse in-place
    Delay(DelayArgs),

    /// Remove a fuse from a source file
    Disarm(DisarmArgs),

    /// Show fuse counts broken down by owner and tag
    Intel(IntelArgs),

    /// Manage the git pre-commit tripwire
    Tripwire(TripwireArgs),

    /// Compare two report JSON snapshots and show fuse debt trajectory
    Fallout(FalloutArgs),

    /// Interactively defuse detonated fuses: extend, delete, or skip each one
    Defuse(DefuseArgs),

    /// Save or show the fuse count baseline for ratchet enforcement
    Bunker(BunkerArgs),

    /// Print a shell completion script to stdout
    Completions(CompletionsArgs),
}

/// Arguments for the `sweep` subcommand.
#[derive(Debug, clap::Args)]
pub struct SweepArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Warn on fuses expiring within this window (e.g. "30d")
    #[arg(long, value_name = "DURATION")]
    pub fuse: Option<String>,

    /// Exit with code 1 if any fuses are in the ticking window (not just detonated)
    #[arg(long, default_value_t = false)]
    pub fail_on_ticking: bool,

    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Only report fuses touched in the git diff against this ref (e.g. "HEAD", "main")
    #[arg(long, value_name = "REF")]
    pub since: Option<String>,

    /// Enrich fuses without an explicit owner with git blame author
    #[arg(long)]
    pub blame: bool,

    /// Only report fuses on lines changed in the git diff
    #[arg(long, default_value_t = false)]
    pub changed: bool,

    /// Base ref for --changed (default: HEAD)
    #[arg(long, value_name = "REF", requires = "changed")]
    pub base: Option<String>,

    /// Only show fuses belonging to this owner (case-insensitive)
    #[arg(long, value_name = "OWNER")]
    pub owner: Option<String>,

    /// Only show fuses with this tag (case-insensitive, e.g. "FIXME")
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Suppress all output; rely on the exit code only
    #[arg(long, default_value_t = false)]
    pub quiet: bool,

    /// Print only the summary line, not individual fuses
    #[arg(long, default_value_t = false, conflicts_with = "quiet")]
    pub summary: bool,

    /// Hard ceiling on detonated fuses; sweep exits 1 if exceeded (overrides config)
    #[arg(long, value_name = "N")]
    pub max_detonated: Option<u32>,

    /// Hard ceiling on ticking fuses; sweep exits 1 if exceeded (overrides config)
    #[arg(long, value_name = "N")]
    pub max_ticking: Option<u32>,

    /// Write a JSON report to this file in addition to normal output
    #[arg(long, value_name = "FILE")]
    pub output: Option<String>,

    /// Hide inert (safe) fuses from output
    #[arg(long, default_value_t = false)]
    pub no_inert: bool,

    /// Print a per-tag breakdown of detonated/ticking counts after the summary (terminal only)
    #[arg(long, default_value_t = false)]
    pub stats: bool,
}

/// Arguments for the `manifest` subcommand.
#[derive(Debug, clap::Args)]
pub struct ManifestArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Only show detonated fuses
    #[arg(long, default_value_t = false)]
    pub detonated: bool,

    /// Only show fuses ticking within this window (e.g. "14d")
    #[arg(long, value_name = "DURATION", conflicts_with = "detonated")]
    pub ticking: Option<String>,

    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Fuse-days threshold used for status classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub fuse: Option<String>,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Enrich fuses without an explicit owner with git blame author
    #[arg(long)]
    pub blame: bool,

    /// Only show fuses belonging to this owner (case-insensitive)
    #[arg(long, value_name = "OWNER")]
    pub owner: Option<String>,

    /// Only show fuses with this tag (case-insensitive, e.g. "TODO")
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Show only the N soonest-to-detonate fuses
    #[arg(long, value_name = "N")]
    pub next: Option<usize>,

    /// Sort order for the fuse list (default: date)
    #[arg(long, value_name = "BY")]
    pub sort: Option<SortBy>,

    /// Only show fuses from these files; may be repeated, supports globs (e.g. "src/auth/**")
    #[arg(long, value_name = "PATH")]
    pub file: Vec<String>,

    /// Only show fuses with expiry dates in this range (inclusive), e.g. --between 2026-01-01 2026-06-30
    #[arg(long, num_args = 2, value_names = ["START", "END"])]
    pub between: Option<Vec<String>>,

    /// Print only the count of matching fuses as a plain integer
    #[arg(long, default_value_t = false)]
    pub count: bool,

    /// Hide inert (safe) fuses from output
    #[arg(long, default_value_t = false)]
    pub no_inert: bool,

    /// Only show fuses with no explicit owner and no git blame result (combine with --blame)
    #[arg(long, default_value_t = false)]
    pub owner_missing: bool,

    /// Write the matching fuses as a JSON file (in addition to stdout output)
    #[arg(long, value_name = "FILE")]
    pub output: Option<String>,
}

/// Arguments for the `armory` subcommand.
#[derive(Debug, clap::Args)]
pub struct ArmoryArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Maximum number of fuses to show
    #[arg(long, default_value_t = 10, value_name = "N")]
    pub limit: usize,

    /// Fuse-days threshold used for ticking classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub fuse: Option<String>,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Enrich fuses without an explicit owner with git blame author
    #[arg(long)]
    pub blame: bool,

    /// Only show fuses belonging to this owner (case-insensitive)
    #[arg(long, value_name = "OWNER")]
    pub owner: Option<String>,

    /// Only show fuses with this tag (case-insensitive, e.g. "TODO")
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,
}

/// Arguments for the `plant` subcommand.
#[derive(Debug, clap::Args)]
pub struct PlantArgs {
    /// File and line to annotate, e.g. "src/main.rs:42"
    #[arg(value_name = "FILE[:LINE]")]
    pub target: String,

    /// Fuse message (what needs to be done / why)
    #[arg(value_name = "MESSAGE")]
    pub message: String,

    /// Search for a pattern instead of specifying :LINE
    #[arg(long, value_name = "PATTERN")]
    pub search: Option<String>,

    /// Tag to use (default: TODO)
    #[arg(long, default_value = "TODO", value_name = "TAG")]
    pub tag: String,

    /// Owner of the fuse, e.g. "alice" or "team-backend"
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

/// Arguments for the `delay` subcommand.
#[derive(Debug, clap::Args)]
pub struct DelayArgs {
    /// Target file and line, e.g. "src/main.rs:42"
    #[arg(value_name = "FILE[:LINE]")]
    pub target: String,

    /// New expiry date as YYYY-MM-DD
    #[arg(long, value_name = "DATE", conflicts_with = "in_days")]
    pub date: Option<String>,

    /// New expiry as number of days from today
    #[arg(long, value_name = "DAYS", conflicts_with = "date")]
    pub in_days: Option<u32>,

    /// Reason for delaying (appended to the fuse message)
    #[arg(long, value_name = "TEXT")]
    pub reason: Option<String>,

    /// Search for a pattern instead of specifying :LINE
    #[arg(long, value_name = "PATTERN")]
    pub search: Option<String>,

    /// Skip confirmation prompt
    #[arg(long, default_value_t = false)]
    pub yes: bool,
}

/// Arguments for the `disarm` subcommand.
#[derive(Debug, clap::Args)]
pub struct DisarmArgs {
    /// File and line to remove, e.g. "src/main.rs:42"
    /// Omit when using --all-detonated
    #[arg(value_name = "FILE[:LINE]")]
    pub target: Option<String>,

    /// Search for a pattern to find the fuse to disarm
    #[arg(long, value_name = "PATTERN", conflicts_with = "all_detonated")]
    pub search: Option<String>,

    /// Remove all detonated fuses across the scan path
    #[arg(long, conflicts_with = "target")]
    pub all_detonated: bool,

    /// Path to scan (used with --all-detonated, default: current directory)
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub path: String,

    /// Path to config file (used with --all-detonated)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Skip confirmation prompt
    #[arg(long, short, default_value_t = false)]
    pub yes: bool,
}

/// Arguments for the `intel` subcommand.
#[derive(Debug, clap::Args)]
pub struct IntelArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Group results by this dimension (default: both)
    #[arg(long, value_name = "DIMENSION")]
    pub by: Option<GroupBy>,

    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Fuse-days threshold used for status classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub fuse: Option<String>,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Only count fuses belonging to this owner (case-insensitive)
    #[arg(long, value_name = "OWNER")]
    pub owner: Option<String>,

    /// Only count fuses with this tag (case-insensitive, e.g. "TODO")
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,
}

/// Arguments for the `tripwire` subcommand.
#[derive(Debug, clap::Args)]
pub struct TripwireArgs {
    #[command(subcommand)]
    pub command: TripwireCommand,
}

/// Subcommands under `tripwire`.
#[derive(Debug, Subcommand)]
pub enum TripwireCommand {
    /// Install the timebomb git pre-commit tripwire
    Set(TripwireSetArgs),
    /// Remove the timebomb git pre-commit tripwire
    Cut(TripwireSetArgs),
}

/// Arguments for `tripwire set` / `tripwire cut`.
#[derive(Debug, clap::Args)]
pub struct TripwireSetArgs {
    /// Path to the git repository root (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Skip confirmation prompts
    #[arg(short, long)]
    pub yes: bool,
}

/// Arguments for the `fallout` subcommand.
#[derive(Debug, clap::Args)]
pub struct FalloutArgs {
    /// Path to the earlier report JSON file (baseline)
    pub report_a: String,
    /// Path to the newer report JSON file (current)
    pub report_b: String,
    /// Output format
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,
}

/// Arguments for the `defuse` subcommand.
#[derive(Debug, clap::Args)]
pub struct DefuseArgs {
    /// Directory to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Fuse-days threshold used for status classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub fuse: Option<String>,
}

/// Arguments for the `bunker` subcommand.
#[derive(Debug, clap::Args)]
pub struct BunkerArgs {
    #[command(subcommand)]
    pub command: BaselineCommand,
}

/// Subcommands under `bunker`.
#[derive(Debug, Subcommand)]
pub enum BaselineCommand {
    /// Record current fuse counts as the baseline
    Save(BunkerSaveArgs),
    /// Compare current counts against the saved baseline
    Show(BunkerShowArgs),
}

/// Arguments for `bunker save`.
#[derive(Debug, clap::Args)]
pub struct BunkerSaveArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Path to the baseline file to write
    #[arg(long, default_value = ".timebomb-baseline.json", value_name = "FILE")]
    pub baseline_file: String,

    /// Fuse-days threshold used for status classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub fuse: Option<String>,
}

/// Arguments for `bunker show`.
#[derive(Debug, clap::Args)]
pub struct BunkerShowArgs {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Path to config file (default: .timebomb.toml in scan root or cwd)
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Path to the baseline file to read
    #[arg(long, default_value = ".timebomb-baseline.json", value_name = "FILE")]
    pub baseline_file: String,

    /// Fuse-days threshold used for status classification (e.g. "14d")
    #[arg(long, value_name = "DURATION")]
    pub fuse: Option<String>,
}

/// Arguments for the `completions` subcommand.
#[derive(Debug, clap::Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    pub shell: Shell,
}

/// The --sort flag value for `manifest`.
#[derive(Debug, Clone, PartialEq, Eq, ValueEnum)]
pub enum SortBy {
    /// Sort by expiry date ascending (default)
    Date,
    /// Sort by file path then line number
    File,
    /// Sort by owner name then date
    Owner,
    /// Sort by status (detonated → ticking → inert) then date
    Status,
}

/// The --by flag value for `intel`.
#[derive(Debug, Clone, PartialEq, Eq, ValueEnum)]
pub enum GroupBy {
    /// Break down by fuse owner
    Owner,
    /// Break down by tag (TODO, FIXME, etc.)
    Tag,
    /// Break down by expiry month (timeline view)
    Month,
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
    /// Comma-separated values
    Csv,
    /// Fixed-width aligned table (manifest only)
    Table,
}

impl FormatArg {
    /// Convert to the `output::OutputFormat` type.
    pub fn to_output_format(&self) -> crate::output::OutputFormat {
        match self {
            FormatArg::Terminal => crate::output::OutputFormat::Terminal,
            FormatArg::Json => crate::output::OutputFormat::Json,
            FormatArg::Github => crate::output::OutputFormat::GitHub,
            FormatArg::Csv => crate::output::OutputFormat::Csv,
            FormatArg::Table => crate::output::OutputFormat::Table,
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

    // ── sweep subcommand ──────────────────────────────────────────────────────

    #[test]
    fn test_sweep_defaults() {
        let cli = parse(&["timebomb", "sweep"]);
        match cli.command {
            Command::Sweep(args) => {
                assert_eq!(args.path, ".");
                assert!(args.fuse.is_none());
                assert!(!args.fail_on_ticking);
                assert!(args.format.is_none());
                assert!(args.config.is_none());
                assert!(args.since.is_none());
            }
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_custom_path() {
        let cli = parse(&["timebomb", "sweep", "./src"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.path, "./src"),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_fuse_flag() {
        let cli = parse(&["timebomb", "sweep", "--fuse", "30d"]);
        match cli.command {
            Command::Sweep(args) => {
                assert_eq!(args.fuse, Some("30d".to_string()));
            }
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_fail_on_ticking() {
        let cli = parse(&["timebomb", "sweep", "--fail-on-ticking"]);
        match cli.command {
            Command::Sweep(args) => assert!(args.fail_on_ticking),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_format_json() {
        let cli = parse(&["timebomb", "sweep", "--format", "json"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.format, Some(FormatArg::Json)),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_format_github() {
        let cli = parse(&["timebomb", "sweep", "--format", "github"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.format, Some(FormatArg::Github)),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_format_terminal() {
        let cli = parse(&["timebomb", "sweep", "--format", "terminal"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.format, Some(FormatArg::Terminal)),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_config_flag() {
        let cli = parse(&["timebomb", "sweep", "--config", "my.toml"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.config, Some("my.toml".to_string())),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_all_flags_combined() {
        let cli = parse(&[
            "timebomb",
            "sweep",
            "./src",
            "--fuse",
            "14d",
            "--fail-on-ticking",
            "--format",
            "json",
            "--config",
            ".timebomb.toml",
        ]);
        match cli.command {
            Command::Sweep(args) => {
                assert_eq!(args.path, "./src");
                assert_eq!(args.fuse, Some("14d".to_string()));
                assert!(args.fail_on_ticking);
                assert_eq!(args.format, Some(FormatArg::Json));
                assert_eq!(args.config, Some(".timebomb.toml".to_string()));
            }
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_since_flag() {
        let cli = parse(&["timebomb", "sweep", "--since", "main"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.since, Some("main".to_string())),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_since_head() {
        let cli = parse(&["timebomb", "sweep", "--since", "HEAD"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.since, Some("HEAD".to_string())),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_owner_flag() {
        let cli = parse(&["timebomb", "sweep", "--owner", "alice"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.owner, Some("alice".to_string())),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_manifest_owner_flag() {
        let cli = parse(&["timebomb", "manifest", "--owner", "bob"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.owner, Some("bob".to_string())),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_sweep_tag_flag() {
        let cli = parse(&["timebomb", "sweep", "--tag", "FIXME"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.tag, Some("FIXME".to_string())),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_quiet_flag() {
        let cli = parse(&["timebomb", "sweep", "--quiet"]);
        match cli.command {
            Command::Sweep(args) => assert!(args.quiet),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_quiet_default_false() {
        let cli = parse(&["timebomb", "sweep"]);
        match cli.command {
            Command::Sweep(args) => assert!(!args.quiet),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_manifest_tag_flag() {
        let cli = parse(&["timebomb", "manifest", "--tag", "TODO"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.tag, Some("TODO".to_string())),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_next_flag() {
        let cli = parse(&["timebomb", "manifest", "--next", "5"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.next, Some(5)),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_next_default_none() {
        let cli = parse(&["timebomb", "manifest"]);
        match cli.command {
            Command::Manifest(args) => assert!(args.next.is_none()),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_armory_defaults() {
        let cli = parse(&["timebomb", "armory"]);
        match cli.command {
            Command::Armory(args) => {
                assert_eq!(args.path, ".");
                assert_eq!(args.limit, 10);
                assert!(args.fuse.is_none());
                assert!(args.config.is_none());
                assert!(!args.blame);
                assert!(args.owner.is_none());
                assert!(args.tag.is_none());
            }
            _ => panic!("expected Armory"),
        }
    }

    #[test]
    fn test_armory_all_flags() {
        let cli = parse(&[
            "timebomb",
            "armory",
            "./src",
            "--limit",
            "5",
            "--fuse",
            "14d",
            "--config",
            ".timebomb.toml",
            "--blame",
            "--owner",
            "alice",
            "--tag",
            "FIXME",
        ]);
        match cli.command {
            Command::Armory(args) => {
                assert_eq!(args.path, "./src");
                assert_eq!(args.limit, 5);
                assert_eq!(args.fuse, Some("14d".to_string()));
                assert_eq!(args.config, Some(".timebomb.toml".to_string()));
                assert!(args.blame);
                assert_eq!(args.owner, Some("alice".to_string()));
                assert_eq!(args.tag, Some("FIXME".to_string()));
            }
            _ => panic!("expected Armory"),
        }
    }

    #[test]
    fn test_sweep_summary_flag() {
        let cli = parse(&["timebomb", "sweep", "--summary"]);
        match cli.command {
            Command::Sweep(args) => assert!(args.summary),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_summary_and_quiet_conflict() {
        let result = try_parse(&["timebomb", "sweep", "--summary", "--quiet"]);
        assert!(result.is_err(), "--summary and --quiet should conflict");
    }

    #[test]
    fn test_sweep_max_detonated_flag() {
        let cli = parse(&["timebomb", "sweep", "--max-detonated", "0"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.max_detonated, Some(0)),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_max_ticking_flag() {
        let cli = parse(&["timebomb", "sweep", "--max-ticking", "5"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.max_ticking, Some(5)),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_max_flags_default_none() {
        let cli = parse(&["timebomb", "sweep"]);
        match cli.command {
            Command::Sweep(args) => {
                assert!(args.max_detonated.is_none());
                assert!(args.max_ticking.is_none());
            }
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_manifest_sort_date() {
        let cli = parse(&["timebomb", "manifest", "--sort", "date"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.sort, Some(SortBy::Date)),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_sort_file() {
        let cli = parse(&["timebomb", "manifest", "--sort", "file"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.sort, Some(SortBy::File)),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_sort_owner() {
        let cli = parse(&["timebomb", "manifest", "--sort", "owner"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.sort, Some(SortBy::Owner)),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_sort_status() {
        let cli = parse(&["timebomb", "manifest", "--sort", "status"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.sort, Some(SortBy::Status)),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_sort_default_none() {
        let cli = parse(&["timebomb", "manifest"]);
        match cli.command {
            Command::Manifest(args) => assert!(args.sort.is_none()),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_sweep_output_flag() {
        let cli = parse(&["timebomb", "sweep", "--output", "report.json"]);
        match cli.command {
            Command::Sweep(args) => assert_eq!(args.output, Some("report.json".to_string())),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_sweep_output_default_none() {
        let cli = parse(&["timebomb", "sweep"]);
        match cli.command {
            Command::Sweep(args) => assert!(args.output.is_none()),
            _ => panic!("expected Sweep"),
        }
    }

    #[test]
    fn test_manifest_file_single() {
        let cli = parse(&["timebomb", "manifest", "--file", "src/auth/login.rs"]);
        match cli.command {
            Command::Manifest(args) => {
                assert_eq!(args.file, vec!["src/auth/login.rs".to_string()])
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_file_multiple() {
        let cli = parse(&[
            "timebomb",
            "manifest",
            "--file",
            "src/auth/login.rs",
            "--file",
            "src/db/schema.sql",
        ]);
        match cli.command {
            Command::Manifest(args) => {
                assert_eq!(
                    args.file,
                    vec![
                        "src/auth/login.rs".to_string(),
                        "src/db/schema.sql".to_string(),
                    ]
                )
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_file_default_empty() {
        let cli = parse(&["timebomb", "manifest"]);
        match cli.command {
            Command::Manifest(args) => assert!(args.file.is_empty()),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_between_flag() {
        let cli = parse(&[
            "timebomb",
            "manifest",
            "--between",
            "2026-01-01",
            "2026-06-30",
        ]);
        match cli.command {
            Command::Manifest(args) => {
                let dates = args.between.unwrap();
                assert_eq!(dates[0], "2026-01-01");
                assert_eq!(dates[1], "2026-06-30");
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_between_default_none() {
        let cli = parse(&["timebomb", "manifest"]);
        match cli.command {
            Command::Manifest(args) => assert!(args.between.is_none()),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_count_flag() {
        let cli = parse(&["timebomb", "manifest", "--count"]);
        match cli.command {
            Command::Manifest(args) => assert!(args.count),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_count_default_false() {
        let cli = parse(&["timebomb", "manifest"]);
        match cli.command {
            Command::Manifest(args) => assert!(!args.count),
            _ => panic!("expected Manifest"),
        }
    }

    // ── plant subcommand ────────────────────────────────────────────────────────

    #[test]
    fn test_plant_message_positional() {
        // Message is now positional
        let cli = parse(&[
            "timebomb",
            "plant",
            "src/main.rs:42",
            "--in-days",
            "90",
            "the message",
        ]);
        match cli.command {
            Command::Plant(args) => {
                assert_eq!(args.target, "src/main.rs:42");
                assert_eq!(args.message, "the message");
                assert_eq!(args.in_days, Some(90));
            }
            _ => panic!("expected Plant"),
        }
    }

    #[test]
    fn test_plant_with_search() {
        let cli = parse(&[
            "timebomb",
            "plant",
            "src/foo.rs",
            "--search",
            "legacy_auth",
            "--in-days",
            "30",
            "msg",
        ]);
        match cli.command {
            Command::Plant(args) => {
                assert_eq!(args.target, "src/foo.rs");
                assert_eq!(args.search, Some("legacy_auth".to_string()));
                assert_eq!(args.in_days, Some(30));
                assert_eq!(args.message, "msg");
            }
            _ => panic!("expected Plant"),
        }
    }

    #[test]
    fn test_plant_defaults() {
        let cli = parse(&["timebomb", "plant", "src/main.rs:42", "remove this"]);
        match cli.command {
            Command::Plant(args) => {
                assert_eq!(args.target, "src/main.rs:42");
                assert_eq!(args.message, "remove this");
                assert_eq!(args.tag, "TODO");
                assert!(args.owner.is_none());
                assert!(args.date.is_none());
                assert!(args.in_days.is_none());
                assert!(!args.yes);
                assert!(args.search.is_none());
            }
            _ => panic!("expected Plant"),
        }
    }

    #[test]
    fn test_plant_all_flags() {
        let cli = parse(&[
            "timebomb",
            "plant",
            "src/auth.rs:10",
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
            Command::Plant(args) => {
                assert_eq!(args.target, "src/auth.rs:10");
                assert_eq!(args.message, "remove oauth flow");
                assert_eq!(args.tag, "FIXME");
                assert_eq!(args.owner, Some("alice".to_string()));
                assert_eq!(args.date, Some("2026-09-01".to_string()));
                assert!(args.yes);
            }
            _ => panic!("expected Plant"),
        }
    }

    #[test]
    fn test_plant_in_days() {
        let cli = parse(&[
            "timebomb",
            "plant",
            "src/lib.rs:1",
            "cleanup",
            "--in-days",
            "90",
        ]);
        match cli.command {
            Command::Plant(args) => assert_eq!(args.in_days, Some(90)),
            _ => panic!("expected Plant"),
        }
    }

    #[test]
    fn test_plant_date_and_in_days_conflict() {
        let result = try_parse(&[
            "timebomb",
            "plant",
            "src/lib.rs:1",
            "cleanup",
            "--date",
            "2026-01-01",
            "--in-days",
            "30",
        ]);
        assert!(result.is_err(), "--date and --in-days should conflict");
    }

    // ── delay subcommand ─────────────────────────────────────────────────────

    #[test]
    fn test_delay_defaults() {
        let cli = parse(&["timebomb", "delay", "src/main.rs:42", "--in-days", "30"]);
        match cli.command {
            Command::Delay(args) => {
                assert_eq!(args.target, "src/main.rs:42");
                assert_eq!(args.in_days, Some(30));
                assert!(args.date.is_none());
                assert!(args.reason.is_none());
                assert!(args.search.is_none());
                assert!(!args.yes);
            }
            _ => panic!("expected Delay"),
        }
    }

    #[test]
    fn test_delay_with_search() {
        let cli = parse(&[
            "timebomb",
            "delay",
            "src/main.rs",
            "--search",
            "pattern",
            "--in-days",
            "30",
        ]);
        match cli.command {
            Command::Delay(args) => {
                assert_eq!(args.target, "src/main.rs");
                assert_eq!(args.search, Some("pattern".to_string()));
                assert_eq!(args.in_days, Some(30));
            }
            _ => panic!("expected Delay"),
        }
    }

    #[test]
    fn test_delay_with_date() {
        let cli = parse(&[
            "timebomb",
            "delay",
            "src/main.rs:42",
            "--date",
            "2027-01-01",
        ]);
        match cli.command {
            Command::Delay(args) => {
                assert_eq!(args.date, Some("2027-01-01".to_string()));
                assert!(args.in_days.is_none());
            }
            _ => panic!("expected Delay"),
        }
    }

    #[test]
    fn test_delay_with_reason() {
        let cli = parse(&[
            "timebomb",
            "delay",
            "src/main.rs:42",
            "--in-days",
            "30",
            "--reason",
            "blocked upstream",
        ]);
        match cli.command {
            Command::Delay(args) => {
                assert_eq!(args.reason, Some("blocked upstream".to_string()));
            }
            _ => panic!("expected Delay"),
        }
    }

    // ── disarm subcommand ─────────────────────────────────────────────────────

    #[test]
    fn test_disarm_by_target() {
        let cli = parse(&["timebomb", "disarm", "src/main.rs:42"]);
        match cli.command {
            Command::Disarm(args) => {
                assert_eq!(args.target, Some("src/main.rs:42".to_string()));
                assert!(args.search.is_none());
                assert!(!args.all_detonated);
            }
            _ => panic!("expected Disarm"),
        }
    }

    #[test]
    fn test_disarm_with_search() {
        let cli = parse(&["timebomb", "disarm", "src/main.rs", "--search", "pattern"]);
        match cli.command {
            Command::Disarm(args) => {
                assert_eq!(args.target, Some("src/main.rs".to_string()));
                assert_eq!(args.search, Some("pattern".to_string()));
            }
            _ => panic!("expected Disarm"),
        }
    }

    #[test]
    fn test_disarm_all_detonated() {
        let cli = parse(&["timebomb", "disarm", "--all-detonated", "--path", "./src"]);
        match cli.command {
            Command::Disarm(args) => {
                assert!(args.all_detonated);
                assert_eq!(args.path, "./src");
                assert!(args.target.is_none());
            }
            _ => panic!("expected Disarm"),
        }
    }

    #[test]
    fn test_disarm_all_detonated_default_path() {
        let cli = parse(&["timebomb", "disarm", "--all-detonated"]);
        match cli.command {
            Command::Disarm(args) => {
                assert!(args.all_detonated);
                assert_eq!(args.path, ".");
            }
            _ => panic!("expected Disarm"),
        }
    }

    #[test]
    fn test_disarm_yes_flag() {
        let cli = parse(&["timebomb", "disarm", "src/main.rs:42", "--yes"]);
        match cli.command {
            Command::Disarm(args) => assert!(args.yes),
            _ => panic!("expected Disarm"),
        }
    }

    // ── intel subcommand ──────────────────────────────────────────────────────

    #[test]
    fn test_intel_defaults() {
        let cli = parse(&["timebomb", "intel"]);
        match cli.command {
            Command::Intel(args) => {
                assert_eq!(args.path, ".");
                assert!(args.by.is_none());
                assert!(args.format.is_none());
                assert!(args.fuse.is_none());
                assert!(args.config.is_none());
            }
            _ => panic!("expected Intel"),
        }
    }

    #[test]
    fn test_intel_by_owner() {
        let cli = parse(&["timebomb", "intel", "--by", "owner"]);
        match cli.command {
            Command::Intel(args) => assert_eq!(args.by, Some(GroupBy::Owner)),
            _ => panic!("expected Intel"),
        }
    }

    #[test]
    fn test_intel_by_tag() {
        let cli = parse(&["timebomb", "intel", "--by", "tag"]);
        match cli.command {
            Command::Intel(args) => assert_eq!(args.by, Some(GroupBy::Tag)),
            _ => panic!("expected Intel"),
        }
    }

    #[test]
    fn test_intel_all_flags() {
        let cli = parse(&[
            "timebomb",
            "intel",
            "./src",
            "--by",
            "owner",
            "--format",
            "json",
            "--fuse",
            "14d",
            "--config",
            "custom.toml",
        ]);
        match cli.command {
            Command::Intel(args) => {
                assert_eq!(args.path, "./src");
                assert_eq!(args.by, Some(GroupBy::Owner));
                assert_eq!(args.format, Some(FormatArg::Json));
                assert_eq!(args.fuse, Some("14d".to_string()));
                assert_eq!(args.config, Some("custom.toml".to_string()));
            }
            _ => panic!("expected Intel"),
        }
    }

    // ── manifest subcommand ───────────────────────────────────────────────────

    #[test]
    fn test_manifest_defaults() {
        let cli = parse(&["timebomb", "manifest"]);
        match cli.command {
            Command::Manifest(args) => {
                assert_eq!(args.path, ".");
                assert!(!args.detonated);
                assert!(args.ticking.is_none());
                assert!(args.format.is_none());
                assert!(args.fuse.is_none());
                assert!(args.config.is_none());
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_detonated_flag() {
        let cli = parse(&["timebomb", "manifest", "--detonated"]);
        match cli.command {
            Command::Manifest(args) => assert!(args.detonated),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_ticking() {
        let cli = parse(&["timebomb", "manifest", "--ticking", "14d"]);
        match cli.command {
            Command::Manifest(args) => {
                assert_eq!(args.ticking, Some("14d".to_string()));
                assert!(!args.detonated);
            }
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_detonated_and_ticking_conflict() {
        // --detonated and --ticking should conflict
        let result = try_parse(&["timebomb", "manifest", "--detonated", "--ticking", "14d"]);
        assert!(result.is_err(), "conflicting flags should produce an error");
    }

    #[test]
    fn test_manifest_format_json() {
        let cli = parse(&["timebomb", "manifest", "--format", "json"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.format, Some(FormatArg::Json)),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_fuse_flag() {
        let cli = parse(&["timebomb", "manifest", "--fuse", "7d"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.fuse, Some("7d".to_string())),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_custom_path() {
        let cli = parse(&["timebomb", "manifest", "./my/project"]);
        match cli.command {
            Command::Manifest(args) => assert_eq!(args.path, "./my/project"),
            _ => panic!("expected Manifest"),
        }
    }

    #[test]
    fn test_manifest_all_flags_combined() {
        let cli = parse(&[
            "timebomb",
            "manifest",
            "./src",
            "--detonated",
            "--format",
            "github",
            "--fuse",
            "30d",
            "--config",
            "custom.toml",
        ]);
        match cli.command {
            Command::Manifest(args) => {
                assert_eq!(args.path, "./src");
                assert!(args.detonated);
                assert_eq!(args.format, Some(FormatArg::Github));
                assert_eq!(args.fuse, Some("30d".to_string()));
                assert_eq!(args.config, Some("custom.toml".to_string()));
            }
            _ => panic!("expected Manifest"),
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
