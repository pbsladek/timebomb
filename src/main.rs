use timebomb::add::{parse_target, run_add};
use timebomb::annotation;
use timebomb::armory::{print_armory, select_armory_fuses};
use timebomb::baseline;
use timebomb::blame::enrich_with_blame;
use timebomb::config::{self, load_config, CliOverrides};
use timebomb::diff;
use timebomb::error::{parse_duration_days, Error};
use timebomb::fix;
use timebomb::git::{git_changed_files, is_git_repo};
use timebomb::hook;
use timebomb::output::{
    print_agent_summary, print_explain, print_fix_plan_json, print_json_list, print_list,
    print_scan_result, print_scan_summary, write_json_report, OutputFormat,
};
use timebomb::remove::{run_remove, run_remove_all_expired};
use timebomb::scanner::scan;
use timebomb::snooze::run_snooze;
use timebomb::stats::{compute_stats, print_stats, print_stats_month};
use timebomb::trend;

use chrono::Local;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process;
use timebomb::cli::{BaselineCommand, Cli, Command, FixPlanArg, GroupBy, SortBy, TripwireCommand};

fn main() {
    let cli = Cli::parse();

    let today = Local::now().date_naive();

    let exit_code = match run(cli, today) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {}", e);
            2
        }
    };

    process::exit(exit_code);
}

/// Resolve the fuse warning window: CLI flag > `TIMEBOMB_FUSE_DAYS` env var > None.
///
/// The env var accepts a plain integer ("30") or a duration string ("30d"). CLI always wins.
fn resolve_fuse_arg(cli_fuse: Option<String>) -> Option<String> {
    cli_fuse.or_else(|| {
        std::env::var("TIMEBOMB_FUSE_DAYS").ok().map(|v| {
            if v.ends_with('d') {
                v
            } else {
                format!("{}d", v)
            }
        })
    })
}

/// Returns true if `fuse_file` (relative path) matches a user-supplied filter string.
///
/// Three-step resolution:
/// 1. Strip a leading `./` so shell tab-completion and `git diff --name-only` output works.
/// 2. If the filter contains glob metacharacters, compile and match with `globset`.
/// 3. Otherwise fall back to a component-aware suffix match (`ends_with`).
fn file_matches(fuse_file: &Path, filter: &str) -> bool {
    // Step 1: strip leading ./
    let normalized = filter
        .strip_prefix("./")
        .or_else(|| filter.strip_prefix(".\\"))
        .unwrap_or(filter);

    // Step 2: glob
    if normalized.contains('*')
        || normalized.contains('?')
        || normalized.contains('[')
        || normalized.contains('{')
    {
        if let Ok(glob) = globset::Glob::new(normalized) {
            return glob.compile_matcher().is_match(fuse_file);
        }
    }

    // Step 3: component-aware suffix match
    fuse_file.ends_with(Path::new(normalized))
}

/// Returns true if the fuse message contains the user-supplied text.
fn message_matches(fuse: &annotation::Fuse, filter: &str) -> bool {
    fuse.message.to_lowercase().contains(&filter.to_lowercase())
}

fn normalize_target_file(scan_path: &Path, target_file: &Path) -> timebomb::error::Result<PathBuf> {
    if target_file.is_absolute() {
        let canonical = canonicalize_path(target_file)?;
        Ok(canonical
            .strip_prefix(scan_path)
            .unwrap_or(&canonical)
            .to_path_buf())
    } else {
        let normalized = target_file
            .strip_prefix("./")
            .or_else(|_| target_file.strip_prefix(".\\"))
            .unwrap_or(target_file);
        Ok(normalized.to_path_buf())
    }
}

fn find_fuse_at_target<'a>(
    fuses: &'a [annotation::Fuse],
    target_file: &Path,
    target_line: usize,
) -> Option<&'a annotation::Fuse> {
    fuses
        .iter()
        .find(|fuse| fuse.line == target_line && fuse.file == target_file)
}

/// Numeric order for status-based sorting: detonated first, then ticking, then inert.
fn status_order(status: &timebomb::annotation::Status) -> u8 {
    match status {
        timebomb::annotation::Status::Detonated => 0,
        timebomb::annotation::Status::Ticking => 1,
        timebomb::annotation::Status::Inert => 2,
    }
}

fn run(cli: Cli, today: chrono::NaiveDate) -> timebomb::error::Result<i32> {
    match cli.command {
        Command::Sweep(args) => {
            let scan_path = canonicalize_path(Path::new(&args.path))?;
            let overrides = CliOverrides::new(resolve_fuse_arg(args.fuse), args.fail_on_ticking);
            let mut cfg = resolve_config(args.config.as_deref(), &scan_path, &overrides)?;

            // --since: restrict scan to files changed relative to the given git ref.
            if let Some(ref git_ref) = args.since {
                if !is_git_repo(&scan_path) {
                    return Err(Error::InvalidArgument(format!(
                        "--since requires a git repository, but '{}' is not one",
                        scan_path.display()
                    )));
                }
                let changed = git_changed_files(&scan_path, git_ref)?;
                cfg.diff_files = Some(changed);
            }

            let format = match args.format {
                Some(ref f) => f.to_output_format(),
                None => OutputFormat::auto_detect(),
            };

            if matches!(format, OutputFormat::Csv | OutputFormat::Table) {
                return Err(Error::InvalidArgument(
                    "--format csv and --format table are not supported for sweep; use terminal, json, or github"
                        .to_string(),
                ));
            }

            let mut result = scan(&scan_path, &cfg, today)?;

            if args.blame {
                enrich_with_blame(&mut result.fuses, &scan_path);
            }

            // --changed: retain only fuses that fall on lines modified in the git diff.
            if args.changed {
                let base = args.base.as_deref().unwrap_or("HEAD");
                let line_ranges = diff::git_changed_line_ranges(&scan_path, base)?;
                result.fuses.retain(|fuse| {
                    line_ranges
                        .get(&fuse.file)
                        .map(|ranges| ranges.iter().any(|r| r.contains(&fuse.line)))
                        .unwrap_or(false)
                });
            }

            // --owner: retain only fuses whose owner matches (case-insensitive).
            if let Some(ref owner_filter) = args.owner {
                let lower = owner_filter.to_lowercase();
                result.fuses.retain(|fuse| {
                    fuse.owner
                        .as_deref()
                        .or(fuse.blamed_owner.as_deref())
                        .map(|o| o.to_lowercase() == lower)
                        .unwrap_or(false)
                });
            }

            // --tag: retain only fuses whose tag matches (case-insensitive).
            if let Some(ref tag_filter) = args.tag {
                let lower = tag_filter.to_lowercase();
                result.fuses.retain(|fuse| fuse.tag.to_lowercase() == lower);
            }

            // --message: retain only fuses whose message contains the filter text.
            if let Some(ref message_filter) = args.message {
                result
                    .fuses
                    .retain(|fuse| message_matches(fuse, message_filter));
            }

            // --no-inert: drop inert fuses from display only.
            // This does NOT affect the exit code: result.detonated() and
            // result.ticking() filter by status, so removing inert fuses from
            // result.fuses has no impact on ratchet counts.
            if args.no_inert {
                result
                    .fuses
                    .retain(|fuse| fuse.status != timebomb::annotation::Status::Inert);
            }

            // --max-detonated / --max-ticking: CLI overrides for ratchet ceilings.
            if let Some(n) = args.max_detonated {
                cfg.max_detonated = Some(n as usize);
            }
            if let Some(n) = args.max_ticking {
                cfg.max_ticking = Some(n as usize);
            }

            // Ratchet check: compare counts against saved baseline and/or config limits.
            let baseline_path = scan_path.join(".timebomb-baseline.json");
            let loaded_baseline = baseline::load_baseline(&baseline_path)?;
            let violations = baseline::check_ratchet(
                result.detonated().len(),
                result.ticking().len(),
                loaded_baseline.as_ref(),
                cfg.max_detonated,
                cfg.max_ticking,
            );

            if !args.quiet {
                if args.agent_summary {
                    let failed = result.has_detonated()
                        || (cfg.fail_on_ticking && result.is_ticking())
                        || !violations.is_empty();
                    print_agent_summary(&result, failed);
                } else if let Some(FixPlanArg::Json) = args.fix_plan {
                    print_fix_plan_json(&result);
                } else if args.summary {
                    print_scan_summary(&result);
                } else {
                    print_scan_result(&result, &format, cfg.fuse_days, today, args.stats);
                }
            }

            // --output: write a JSON report to a file regardless of --format.
            if let Some(ref out_path) = args.output {
                write_json_report(&result, Path::new(out_path), today).map_err(|e| Error::Io {
                    source: e,
                    path: Some(PathBuf::from(out_path)),
                })?;
            }

            if !violations.is_empty() {
                for v in &violations {
                    eprintln!("ratchet: {v}");
                }
                return Ok(1);
            }

            if result.has_detonated() {
                return Ok(1);
            }
            if cfg.fail_on_ticking && result.is_ticking() {
                return Ok(1);
            }
            Ok(0)
        }

        Command::Manifest(args) => {
            let scan_path = canonicalize_path(Path::new(&args.path))?;
            let overrides = CliOverrides::new(resolve_fuse_arg(args.fuse), false);
            let cfg = resolve_config(args.config.as_deref(), &scan_path, &overrides)?;

            let format = match args.format {
                Some(ref f) => f.to_output_format(),
                None => OutputFormat::auto_detect(),
            };

            let mut result = scan(&scan_path, &cfg, today)?;

            if args.blame {
                enrich_with_blame(&mut result.fuses, &scan_path);
            }

            // --owner: retain only fuses whose owner matches (case-insensitive).
            if let Some(ref owner_filter) = args.owner {
                let lower = owner_filter.to_lowercase();
                result.fuses.retain(|fuse| {
                    fuse.owner
                        .as_deref()
                        .or(fuse.blamed_owner.as_deref())
                        .map(|o| o.to_lowercase() == lower)
                        .unwrap_or(false)
                });
            }

            // --tag: retain only fuses whose tag matches (case-insensitive).
            if let Some(ref tag_filter) = args.tag {
                let lower = tag_filter.to_lowercase();
                result.fuses.retain(|fuse| fuse.tag.to_lowercase() == lower);
            }

            // --message: retain only fuses whose message contains the filter text.
            if let Some(ref message_filter) = args.message {
                result
                    .fuses
                    .retain(|fuse| message_matches(fuse, message_filter));
            }

            // --no-inert: drop inert fuses before status filtering.
            if args.no_inert {
                result
                    .fuses
                    .retain(|fuse| fuse.status != timebomb::annotation::Status::Inert);
            }

            // --owner-missing: keep only fuses with no explicit owner and no blame result.
            if args.owner_missing {
                result
                    .fuses
                    .retain(|fuse| fuse.owner.is_none() && fuse.blamed_owner.is_none());
            }

            let mut fuses: Vec<&annotation::Fuse> = if args.detonated {
                result.detonated()
            } else if let Some(ref soon_str) = args.ticking {
                let days = parse_duration_days(soon_str)?;
                result
                    .fuses
                    .iter()
                    .filter(|a| {
                        let days_remaining = a.days_from_today(today);
                        days_remaining >= 0 && days_remaining <= days as i64
                    })
                    .collect()
            } else {
                result.fuses.iter().collect()
            };

            // --file: retain fuses whose file matches any of the given filters.
            // Each filter supports globs, ./ normalization, and suffix matching.
            if !args.file.is_empty() {
                fuses.retain(|f| args.file.iter().any(|filter| file_matches(&f.file, filter)));
            }

            // --between START END: retain only fuses whose date falls in the range (inclusive).
            if let Some(ref dates) = args.between {
                let start =
                    chrono::NaiveDate::parse_from_str(&dates[0], "%Y-%m-%d").map_err(|_| {
                        Error::InvalidArgument(format!(
                            "--between: invalid start date '{}', expected YYYY-MM-DD",
                            dates[0]
                        ))
                    })?;
                let end =
                    chrono::NaiveDate::parse_from_str(&dates[1], "%Y-%m-%d").map_err(|_| {
                        Error::InvalidArgument(format!(
                            "--between: invalid end date '{}', expected YYYY-MM-DD",
                            dates[1]
                        ))
                    })?;
                if start > end {
                    return Err(Error::InvalidArgument(format!(
                        "--between: start date '{}' is after end date '{}'",
                        dates[0], dates[1]
                    )));
                }
                fuses.retain(|f| f.date >= start && f.date <= end);
            }

            // --sort: re-sort if a non-default order was requested.
            match args.sort {
                None | Some(SortBy::Date) => {} // already date-ascending from scan()
                Some(SortBy::File) => {
                    fuses.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
                }
                Some(SortBy::Owner) => {
                    fuses.sort_by(|a, b| {
                        a.owner
                            .as_deref()
                            .unwrap_or("")
                            .cmp(b.owner.as_deref().unwrap_or(""))
                            .then(a.date.cmp(&b.date))
                    });
                }
                Some(SortBy::Status) => {
                    fuses.sort_by(|a, b| {
                        status_order(&a.status)
                            .cmp(&status_order(&b.status))
                            .then(a.date.cmp(&b.date))
                    });
                }
            }

            // --next: show only the N soonest fuses (applied after sort).
            if let Some(n) = args.next {
                fuses.truncate(n);
            }

            if args.path_only {
                let paths: BTreeSet<_> = fuses.iter().map(|fuse| &fuse.file).collect();
                for path in paths {
                    println!("{}", path.display());
                }
            } else if args.count {
                println!("{}", fuses.len());
            } else {
                print_list(&fuses, &format, cfg.fuse_days, &scan_path, today);
            }

            // --output: write the fuse list to a file in the requested format.
            if let Some(ref out_path) = args.output {
                let file = std::fs::File::create(out_path).map_err(|e| Error::Io {
                    source: e,
                    path: Some(PathBuf::from(out_path)),
                })?;
                match format {
                    OutputFormat::Csv => {
                        use timebomb::output::print_csv_list_to_writer;
                        print_csv_list_to_writer(&fuses, file).map_err(|e| Error::Io {
                            source: e,
                            path: Some(PathBuf::from(out_path)),
                        })?;
                    }
                    OutputFormat::Table => {
                        use timebomb::output::print_table_list_to_writer;
                        print_table_list_to_writer(&fuses, file).map_err(|e| Error::Io {
                            source: e,
                            path: Some(PathBuf::from(out_path)),
                        })?;
                    }
                    _ => {
                        use timebomb::output::print_json_list_to_writer;
                        print_json_list_to_writer(&fuses, file, today).map_err(|e| Error::Io {
                            source: e,
                            path: Some(PathBuf::from(out_path)),
                        })?;
                    }
                }
            }

            Ok(0)
        }

        Command::Armory(args) => {
            let scan_path = canonicalize_path(Path::new(&args.path))?;
            let overrides = CliOverrides::new(resolve_fuse_arg(args.fuse), false);
            let cfg = resolve_config(args.config.as_deref(), &scan_path, &overrides)?;

            let mut result = scan(&scan_path, &cfg, today)?;

            if args.blame {
                enrich_with_blame(&mut result.fuses, &scan_path);
            }

            if let Some(ref owner_filter) = args.owner {
                let lower = owner_filter.to_lowercase();
                result.fuses.retain(|fuse| {
                    fuse.owner
                        .as_deref()
                        .or(fuse.blamed_owner.as_deref())
                        .map(|o| o.to_lowercase() == lower)
                        .unwrap_or(false)
                });
            }

            if let Some(ref tag_filter) = args.tag {
                let lower = tag_filter.to_lowercase();
                result.fuses.retain(|fuse| fuse.tag.to_lowercase() == lower);
            }

            if let Some(ref message_filter) = args.message {
                result
                    .fuses
                    .retain(|fuse| message_matches(fuse, message_filter));
            }

            let limit = if args.count {
                usize::MAX
            } else if args.oldest {
                1
            } else {
                args.limit
            };
            let fuses = select_armory_fuses(&result.fuses, today, limit);
            if args.count {
                println!("{}", fuses.len());
            } else if args.json {
                print_json_list(&fuses, today);
            } else {
                print_armory(&fuses, today, args.oldest);
            }
            Ok(0)
        }

        Command::Explain(args) => {
            let scan_path = canonicalize_path(Path::new(&args.path))?;
            let overrides = CliOverrides::new(resolve_fuse_arg(args.fuse), false);
            let cfg = resolve_config(args.config.as_deref(), &scan_path, &overrides)?;
            let (target_file, target_line) = parse_target(&args.target)?;
            let target_file = normalize_target_file(&scan_path, &target_file)?;

            let mut result = scan(&scan_path, &cfg, today)?;

            if args.blame {
                enrich_with_blame(&mut result.fuses, &scan_path);
            }

            let fuse =
                find_fuse_at_target(&result.fuses, &target_file, target_line).ok_or_else(|| {
                    Error::InvalidArgument(format!(
                        "no fuse found at {}:{}",
                        target_file.display(),
                        target_line
                    ))
                })?;
            print_explain(fuse, today);
            Ok(0)
        }

        Command::Plant(args) => run_add(
            &args.target,
            &args.tag,
            args.owner.as_deref(),
            args.date.as_deref(),
            args.in_days,
            args.yes,
            &args.message,
            today,
            args.search.as_deref(),
        ),

        Command::Delay(args) => run_snooze(
            &args.target,
            args.date.as_deref(),
            args.in_days,
            args.reason.as_deref(),
            args.yes,
            today,
            args.search.as_deref(),
        ),

        Command::Disarm(args) => {
            if args.all_detonated {
                let scan_path = canonicalize_path(Path::new(&args.path))?;
                let overrides = CliOverrides::default();
                let cfg = resolve_config(args.config.as_deref(), &scan_path, &overrides)?;
                run_remove_all_expired(&scan_path, &cfg, today, args.yes)
            } else if let Some(ref target) = args.target {
                run_remove(target, args.search.as_deref(), args.yes)
            } else {
                Err(Error::InvalidArgument(
                    "either a target FILE[:LINE] or --all-detonated is required".to_string(),
                ))
            }
        }

        Command::Intel(args) => {
            let scan_path = canonicalize_path(Path::new(&args.path))?;
            let overrides = CliOverrides::new(resolve_fuse_arg(args.fuse), false);
            let cfg = resolve_config(args.config.as_deref(), &scan_path, &overrides)?;

            let format = match args.format {
                Some(ref f) => f.to_output_format(),
                None => OutputFormat::auto_detect(),
            };

            let mut result = scan(&scan_path, &cfg, today)?;

            if let Some(ref owner_filter) = args.owner {
                let lower = owner_filter.to_lowercase();
                result.fuses.retain(|fuse| {
                    fuse.owner
                        .as_deref()
                        .or(fuse.blamed_owner.as_deref())
                        .map(|o| o.to_lowercase() == lower)
                        .unwrap_or(false)
                });
            }

            if let Some(ref tag_filter) = args.tag {
                let lower = tag_filter.to_lowercase();
                result.fuses.retain(|fuse| fuse.tag.to_lowercase() == lower);
            }

            if let Some(ref message_filter) = args.message {
                result
                    .fuses
                    .retain(|fuse| message_matches(fuse, message_filter));
            }

            let stats = compute_stats(&result.fuses);
            match args.by {
                None | Some(GroupBy::Owner) | Some(GroupBy::Tag) => {
                    print_stats(&stats, &format);
                }
                Some(GroupBy::Month) => {
                    print_stats_month(&stats, &format);
                }
            }
            Ok(0)
        }

        Command::Tripwire(args) => match args.command {
            TripwireCommand::Set(a) => {
                let path = canonicalize_path(Path::new(&a.path))?;
                hook::run_hook_install(&path, a.yes)
            }
            TripwireCommand::Cut(a) => {
                let path = canonicalize_path(Path::new(&a.path))?;
                hook::run_hook_uninstall(&path, a.yes)
            }
        },

        Command::Fallout(args) => {
            let format = match args.format {
                Some(ref f) => f.to_output_format(),
                None => OutputFormat::auto_detect(),
            };
            trend::run_trend(
                Path::new(&args.report_a),
                Path::new(&args.report_b),
                &format,
            )
        }

        Command::Defuse(args) => {
            let scan_path = canonicalize_path(Path::new(&args.path))?;
            let overrides = CliOverrides::new(resolve_fuse_arg(args.fuse), false);
            let cfg = resolve_config(args.config.as_deref(), &scan_path, &overrides)?;
            let summary = fix::run_fix(&scan_path, &cfg, today)?;
            println!(
                "\nExtended: {}  Deleted: {}  Skipped: {}",
                summary.extended, summary.deleted, summary.skipped
            );
            Ok(0)
        }

        Command::Bunker(args) => match args.command {
            BaselineCommand::Save(a) => {
                let scan_path = canonicalize_path(Path::new(&a.path))?;
                let overrides = CliOverrides::new(resolve_fuse_arg(a.fuse), false);
                let cfg = resolve_config(a.config.as_deref(), &scan_path, &overrides)?;
                let baseline_path = Path::new(&a.baseline_file);
                // Use the full RFC 3339 timestamp from the local clock, not just the date.
                let generated_at = Local::now().to_rfc3339();
                baseline::run_baseline_save(&scan_path, &cfg, today, baseline_path, &generated_at)
            }
            BaselineCommand::Show(a) => {
                let scan_path = canonicalize_path(Path::new(&a.path))?;
                let overrides = CliOverrides::new(resolve_fuse_arg(a.fuse), false);
                let cfg = resolve_config(a.config.as_deref(), &scan_path, &overrides)?;
                let baseline_path = Path::new(&a.baseline_file);
                baseline::run_baseline_show(&scan_path, &cfg, today, baseline_path)
            }
        },

        Command::Completions(args) => {
            let mut cmd = Cli::command();
            generate(args.shell, &mut cmd, "timebomb", &mut std::io::stdout());
            Ok(0)
        }
    }
}

/// Resolve configuration from (in priority order):
///   1. An explicit `--config <file>` path
///   2. `<scan_path>/.timebomb.toml`
///   3. `./.timebomb.toml` in the current working directory (CWD fallback)
///   4. Built-in defaults
///
/// CLI `overrides` are applied on top of whatever file is found.
fn resolve_config(
    config_flag: Option<&str>,
    scan_path: &Path,
    overrides: &CliOverrides,
) -> timebomb::error::Result<config::Config> {
    if let Some(cfg_path_str) = config_flag {
        // Explicit --config flag: load exactly that file (error if missing)
        let cfg_file_path = Path::new(cfg_path_str);
        let content = std::fs::read_to_string(cfg_file_path).map_err(|e| Error::ConfigRead {
            source: e,
            path: cfg_file_path.to_path_buf(),
        })?;
        let file_cfg: config::ConfigFile =
            toml::from_str(&content).map_err(|e| Error::ConfigParse {
                source: e,
                path: cfg_file_path.to_path_buf(),
            })?;
        return merge_file_config(file_cfg, overrides);
    }

    // No --config flag: look in the scan directory first, then CWD.
    let scan_dir_config = scan_path.join(".timebomb.toml");
    if scan_dir_config.exists() {
        return load_config(scan_path, overrides);
    }

    // CWD fallback (only when scan_path != CWD)
    let cwd_config = PathBuf::from(".timebomb.toml");
    if cwd_config.exists() {
        let cwd = std::env::current_dir().map_err(|e| Error::Io {
            source: e,
            path: None,
        })?;
        // Avoid reading the same file twice if scan_path == cwd
        if cwd != scan_path {
            return load_config(&cwd, overrides);
        }
    }

    // No config file found anywhere — use defaults + overrides
    load_config(scan_path, overrides)
}

/// Resolve a path, returning an error if it doesn't exist.
fn canonicalize_path(path: &Path) -> timebomb::error::Result<PathBuf> {
    path.canonicalize().map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })
}

/// Merge a `ConfigFile` with CLI overrides, producing a resolved `Config`.
fn merge_file_config(
    file_cfg: config::ConfigFile,
    overrides: &CliOverrides,
) -> timebomb::error::Result<config::Config> {
    use config::Config;
    let defaults = Config::default();

    let triggers = file_cfg.triggers.unwrap_or(defaults.triggers);
    let mut fuse_days = file_cfg.fuse_days.unwrap_or(defaults.fuse_days);
    let exclude_patterns = file_cfg.exclude.unwrap_or(defaults.exclude_patterns);
    let extensions = file_cfg.extensions.unwrap_or(defaults.extensions);

    if let Some(ref w) = overrides.fuse {
        fuse_days = parse_duration_days(w)?;
    }

    Ok(Config {
        triggers,
        fuse_days,
        exclude_patterns,
        extensions,
        fail_on_ticking: overrides.fail_on_ticking,
        diff_files: None,
        max_detonated: file_cfg.max_detonated,
        max_ticking: file_cfg.max_ticking,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    use timebomb::cli::Cli;

    fn fixed_today() -> NaiveDate {
        NaiveDate::parse_from_str("2025-06-01", "%Y-%m-%d").unwrap()
    }

    fn with_fuse_days_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let previous = std::env::var("TIMEBOMB_FUSE_DAYS").ok();

        match value {
            Some(v) => std::env::set_var("TIMEBOMB_FUSE_DAYS", v),
            None => std::env::remove_var("TIMEBOMB_FUSE_DAYS"),
        }

        let result = f();

        match previous {
            Some(v) => std::env::set_var("TIMEBOMB_FUSE_DAYS", v),
            None => std::env::remove_var("TIMEBOMB_FUSE_DAYS"),
        }

        result
    }

    // ── sweep subcommand ──────────────────────────────────────────────────────

    #[test]
    fn test_sweep_no_detonated_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("ok.rs")).unwrap();
        writeln!(f, "// TODO[2099-01-01]: fine").unwrap();

        let cli = Cli::parse_from(["timebomb", "sweep", dir.path().to_str().unwrap()]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_sweep_detonated_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from(["timebomb", "sweep", dir.path().to_str().unwrap()]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn test_sweep_ticking_only_no_fail_on_ticking_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("soon.rs")).unwrap();
        // 8 days from our fixed today
        writeln!(f, "// TODO[2025-06-09]: ticking").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--fuse",
            "14d",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        // Ticking alone without --fail-on-ticking should exit 0
        assert_eq!(code, 0);
    }

    #[test]
    fn test_sweep_fail_on_ticking_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("soon.rs")).unwrap();
        writeln!(f, "// TODO[2025-06-09]: ticking").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--fuse",
            "14d",
            "--fail-on-ticking",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn test_sweep_json_format() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("lib.rs")).unwrap();
        writeln!(f, "// FIXME[2020-01-01]: old").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
        ]);
        // Should not error; exit code 1 because detonated
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn test_sweep_empty_dir_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::parse_from(["timebomb", "sweep", dir.path().to_str().unwrap()]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_sweep_nonexistent_path_is_error() {
        let cli = Cli::parse_from(["timebomb", "sweep", "/nonexistent/path/xyz"]);
        let result = run(cli, fixed_today());
        assert!(result.is_err());
    }

    #[test]
    fn test_sweep_with_explicit_config() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();

        // Write config that sets fuse_days = 30
        let cfg_path = dir.path().join("my.toml");
        {
            let mut f = std::fs::File::create(&cfg_path).unwrap();
            writeln!(f, "fuse_days = 30").unwrap();
        }

        let src_path = dir.path().join("main.rs");
        {
            let mut f = std::fs::File::create(&src_path).unwrap();
            writeln!(f, "// TODO[2099-01-01]: fine").unwrap();
        }

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--config",
            cfg_path.to_str().unwrap(),
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_sweep_agent_summary_exits_one_with_detonated() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--agent-summary",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn test_sweep_fix_plan_json_exits_one_with_detonated() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--fix-plan",
            "json",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1);
    }

    // ── manifest subcommand ───────────────────────────────────────────────────

    #[test]
    fn test_manifest_exits_zero_even_with_detonated() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from(["timebomb", "manifest", dir.path().to_str().unwrap()]);
        let code = run(cli, fixed_today()).unwrap();
        // manifest always exits 0
        assert_eq!(code, 0);
    }

    #[test]
    fn test_armory_exits_zero_with_detonated() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from(["timebomb", "armory", dir.path().to_str().unwrap()]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_armory_accepts_filters() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "// FIXME[2020-01-01][alice]: detonated").unwrap();
        writeln!(f, "// TODO[2020-01-01][bob]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "armory",
            dir.path().to_str().unwrap(),
            "--owner",
            "alice",
            "--tag",
            "FIXME",
            "--limit",
            "1",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_armory_count_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("mixed.rs")).unwrap();
        writeln!(f, "// FIXME[2020-01-01][alice]: detonated").unwrap();
        writeln!(f, "// TODO[2025-06-03][bob]: ticking").unwrap();
        writeln!(f, "// HACK[2099-01-01]: inert").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "armory",
            dir.path().to_str().unwrap(),
            "--fuse",
            "7d",
            "--count",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_armory_json_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("mixed.rs")).unwrap();
        writeln!(f, "// FIXME[2020-01-01][alice]: detonated").unwrap();
        writeln!(f, "// HACK[2099-01-01]: inert").unwrap();

        let cli = Cli::parse_from(["timebomb", "armory", dir.path().to_str().unwrap(), "--json"]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_explain_exits_zero_for_matching_fuse() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "fn main() {{}}").unwrap();
        writeln!(f, "// TODO[2020-01-01][alice]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "explain",
            "old.rs:2",
            "--path",
            dir.path().to_str().unwrap(),
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_explain_errors_when_no_fuse_at_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("old.rs")).unwrap();
        writeln!(f, "fn main() {{}}").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "explain",
            "old.rs:1",
            "--path",
            dir.path().to_str().unwrap(),
        ]);
        let result = run(cli, fixed_today());
        assert!(result.is_err());
    }

    #[test]
    fn test_manifest_detonated_filter() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("mixed.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();
        writeln!(f, "// FIXME[2099-01-01]: future").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--detonated",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_manifest_ticking_filter() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("mixed.rs")).unwrap();
        writeln!(f, "// TODO[2025-06-08]: ticking in 7 days").unwrap();
        writeln!(f, "// FIXME[2099-01-01]: far future").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--ticking",
            "14d",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_manifest_json_format() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    // ── --owner filter ────────────────────────────────────────────────────────

    #[test]
    fn test_sweep_owner_filter_excludes_unmatched() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("mixed.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01][alice]: alice's fuse").unwrap();
        writeln!(f, "// FIXME[2020-01-01][bob]: bob's fuse").unwrap();

        // Sweeping for alice should still exit 1 (detonated) but only alice's fuse passes
        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--owner",
            "alice",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1); // alice's fuse is detonated
    }

    #[test]
    fn test_sweep_owner_filter_no_match_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("mixed.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01][bob]: bob's detonated fuse").unwrap();

        // Sweeping for alice finds nothing → exits 0
        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--owner",
            "alice",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_sweep_owner_filter_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01][Alice]: uppercase owner").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--owner",
            "alice",
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1); // detonated, and owner matched case-insensitively
    }

    // ── sweep --output ────────────────────────────────────────────────────────

    #[test]
    fn test_sweep_output_writes_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let report_path = dir.path().join("report.json");
        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--output",
            report_path.to_str().unwrap(),
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 1); // detonated
        assert!(report_path.exists(), "report.json should have been written");
        let contents = std::fs::read_to_string(&report_path).unwrap();
        assert!(contents.contains("detonated"));
        assert!(contents.contains("swept_files"));
    }

    #[test]
    fn test_sweep_output_written_even_on_clean_scan() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2099-01-01]: future").unwrap();

        let report_path = dir.path().join("report.json");
        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--output",
            report_path.to_str().unwrap(),
        ]);
        let code = run(cli, fixed_today()).unwrap();
        assert_eq!(code, 0);
        assert!(report_path.exists());
    }

    // ── manifest --file ───────────────────────────────────────────────────────

    #[test]
    fn test_manifest_file_suffix_match() {
        let dir = tempfile::tempdir().unwrap();
        let mut f1 = std::fs::File::create(dir.path().join("auth.rs")).unwrap();
        writeln!(f1, "// TODO[2020-01-01]: auth fuse").unwrap();
        let mut f2 = std::fs::File::create(dir.path().join("db.rs")).unwrap();
        writeln!(f2, "// FIXME[2020-01-01]: db fuse").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--file",
            "auth.rs",
            "--format",
            "json",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_file_dotslash_normalized() {
        // --file ./auth.rs should match the same as --file auth.rs
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("auth.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: auth fuse").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--file",
            "./auth.rs", // leading ./ stripped before matching
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_file_multiple_or_logic() {
        let dir = tempfile::tempdir().unwrap();
        let mut f1 = std::fs::File::create(dir.path().join("auth.rs")).unwrap();
        writeln!(f1, "// TODO[2020-01-01]: auth fuse").unwrap();
        let mut f2 = std::fs::File::create(dir.path().join("db.rs")).unwrap();
        writeln!(f2, "// FIXME[2020-01-01]: db fuse").unwrap();
        // third file not in filter
        let mut f3 = std::fs::File::create(dir.path().join("other.rs")).unwrap();
        writeln!(f3, "// HACK[2020-01-01]: other fuse").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--file",
            "auth.rs",
            "--file",
            "db.rs",
            "--format",
            "json",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_file_glob_star() {
        let dir = tempfile::tempdir().unwrap();
        // Create a subdirectory
        std::fs::create_dir(dir.path().join("auth")).unwrap();
        let mut f1 = std::fs::File::create(dir.path().join("auth").join("login.rs")).unwrap();
        writeln!(f1, "// TODO[2020-01-01]: login fuse").unwrap();
        let mut f2 = std::fs::File::create(dir.path().join("db.rs")).unwrap();
        writeln!(f2, "// FIXME[2020-01-01]: db fuse — should be excluded").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--file",
            "auth/**",
            "--format",
            "json",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_file_glob_extension() {
        let dir = tempfile::tempdir().unwrap();
        let mut f1 = std::fs::File::create(dir.path().join("schema.sql")).unwrap();
        writeln!(f1, "-- TODO[2020-01-01]: sql fuse").unwrap();
        let mut f2 = std::fs::File::create(dir.path().join("main.rs")).unwrap();
        writeln!(f2, "// FIXME[2020-01-01]: rs fuse").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--file",
            "**/*.sql",
            "--format",
            "json",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_file_no_match_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("auth.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: auth fuse").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--file",
            "nonexistent.rs",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    // ── manifest --between ────────────────────────────────────────────────────

    #[test]
    fn test_manifest_between_includes_matching_dates() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2026-03-01]: in range").unwrap();
        writeln!(f, "// FIXME[2099-01-01]: out of range").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--between",
            "2026-01-01",
            "2026-06-30",
            "--format",
            "json",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_between_excludes_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        // Only a far-future fuse — should be excluded by the range
        writeln!(f, "// TODO[2099-01-01]: far future").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--between",
            "2026-01-01",
            "2026-06-30",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_between_invalid_date_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--between",
            "not-a-date",
            "2026-06-30",
        ]);
        assert!(run(cli, fixed_today()).is_err());
    }

    // ── --summary ─────────────────────────────────────────────────────────────

    #[test]
    fn test_sweep_summary_still_exits_one_on_detonated() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--summary",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    #[test]
    fn test_sweep_summary_exits_zero_when_clean() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2099-01-01]: future").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--summary",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    // ── --max-detonated / --max-ticking ───────────────────────────────────────

    #[test]
    fn test_sweep_max_detonated_zero_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--max-detonated",
            "0",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    #[test]
    fn test_sweep_max_detonated_high_allows_pass() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        // max-detonated=5 with 1 detonated → ratchet passes, but has_detonated still exits 1
        // (ratchet check only adds extra failures; the base detonated check still applies)
        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--max-detonated",
            "5",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    #[test]
    fn test_sweep_max_ticking_exceeded_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        // Three ticking fuses (within 30d of 2025-06-01)
        writeln!(f, "// TODO[2025-06-05]: ticking 1").unwrap();
        writeln!(f, "// FIXME[2025-06-10]: ticking 2").unwrap();
        writeln!(f, "// HACK[2025-06-15]: ticking 3").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--fuse",
            "30d",
            "--max-ticking",
            "2", // ceiling is 2, but there are 3
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    // ── manifest --sort ───────────────────────────────────────────────────────

    #[test]
    fn test_manifest_sort_file_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();
        writeln!(f, "// FIXME[2099-01-01]: future").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--sort",
            "file",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_sort_status_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2099-01-01]: future").unwrap();
        writeln!(f, "// FIXME[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--sort",
            "status",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    // ── --tag filter ──────────────────────────────────────────────────────────

    #[test]
    fn test_sweep_tag_filter_matches_only_that_tag() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: todo detonated").unwrap();
        writeln!(f, "// FIXME[2020-01-01]: fixme detonated").unwrap();

        // Only ask about FIXMEs — exits 1 because fixme is detonated
        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--tag",
            "FIXME",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    #[test]
    fn test_sweep_tag_filter_no_match_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        // Filtering by HACK finds nothing → exits 0
        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--tag",
            "HACK",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_sweep_tag_filter_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// FIXME[2020-01-01]: detonated fixme").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--tag",
            "fixme", // lowercase matches uppercase tag
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    #[test]
    fn test_sweep_message_filter_no_match_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: remove legacy oauth").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--message",
            "billing",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_sweep_message_filter_match_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: remove legacy oauth").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "sweep",
            dir.path().to_str().unwrap(),
            "--message",
            "OAUTH",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    #[test]
    fn test_sweep_quiet_suppresses_output_but_still_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from(["timebomb", "sweep", dir.path().to_str().unwrap(), "--quiet"]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 1);
    }

    #[test]
    fn test_manifest_tag_filter() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: a todo").unwrap();
        writeln!(f, "// FIXME[2020-01-01]: a fixme").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--tag",
            "FIXME",
            "--format",
            "json",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_next_truncates_to_n() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        // Three fuses with different dates
        writeln!(f, "// TODO[2020-01-01]: first").unwrap();
        writeln!(f, "// FIXME[2020-06-01]: second").unwrap();
        writeln!(f, "// HACK[2021-01-01]: third").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--next",
            "2",
            "--format",
            "json",
        ]);
        // Should exit 0 and only emit 2 fuses (tested via exit code; output not captured here)
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_next_zero_shows_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--next",
            "0",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    // ── manifest --count ──────────────────────────────────────────────────────

    #[test]
    fn test_manifest_count_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated").unwrap();
        writeln!(f, "// FIXME[2099-01-01]: future").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--count",
        ]);
        // manifest always exits 0 regardless of detonated
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_count_with_detonated_filter() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: detonated 1").unwrap();
        writeln!(f, "// FIXME[2021-01-01]: detonated 2").unwrap();
        writeln!(f, "// HACK[2099-01-01]: future").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--detonated",
            "--count",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_count_empty_dir_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--count",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    #[test]
    fn test_manifest_path_only_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = std::fs::File::create(dir.path().join("a.rs")).unwrap();
        writeln!(a, "// TODO[2020-01-01]: detonated").unwrap();
        let mut b = std::fs::File::create(dir.path().join("b.rs")).unwrap();
        writeln!(b, "// FIXME[2025-06-02]: ticking").unwrap();

        let cli = Cli::parse_from([
            "timebomb",
            "manifest",
            dir.path().to_str().unwrap(),
            "--path-only",
        ]);
        assert_eq!(run(cli, fixed_today()).unwrap(), 0);
    }

    // ── TIMEBOMB_FUSE_DAYS env var ────────────────────────────────────────────

    #[test]
    fn test_resolve_fuse_arg_cli_wins_over_env() {
        // CLI value should take priority over env var
        let result = with_fuse_days_env(Some("999"), || resolve_fuse_arg(Some("14d".to_string())));
        assert_eq!(result, Some("14d".to_string()));
    }

    #[test]
    fn test_resolve_fuse_arg_env_plain_number() {
        let result = with_fuse_days_env(Some("30"), || resolve_fuse_arg(None));
        assert_eq!(result, Some("30d".to_string()));
    }

    #[test]
    fn test_resolve_fuse_arg_env_with_d_suffix() {
        let result = with_fuse_days_env(Some("30d"), || resolve_fuse_arg(None));
        assert_eq!(result, Some("30d".to_string()));
    }

    #[test]
    fn test_resolve_fuse_arg_none_when_no_env() {
        let result = with_fuse_days_env(None, || resolve_fuse_arg(None));
        assert_eq!(result, None);
    }

    // ── file_matches ──────────────────────────────────────────────────────────

    #[test]
    fn test_file_matches_plain_suffix() {
        assert!(file_matches(Path::new("src/auth/login.rs"), "login.rs"));
        assert!(file_matches(
            Path::new("src/auth/login.rs"),
            "auth/login.rs"
        ));
        assert!(!file_matches(Path::new("src/auth/login.rs"), "db.rs"));
    }

    #[test]
    fn test_file_matches_dotslash_stripped() {
        assert!(file_matches(Path::new("auth/login.rs"), "./login.rs"));
        assert!(file_matches(Path::new("auth/login.rs"), "./auth/login.rs"));
    }

    #[test]
    fn test_file_matches_glob_doublestar() {
        assert!(file_matches(Path::new("src/auth/login.rs"), "src/auth/**"));
        assert!(!file_matches(Path::new("src/db/schema.sql"), "src/auth/**"));
    }

    #[test]
    fn test_file_matches_glob_extension() {
        assert!(file_matches(Path::new("schema.sql"), "**/*.sql"));
        assert!(!file_matches(Path::new("main.rs"), "**/*.sql"));
    }

    #[test]
    fn test_file_matches_glob_dotslash_stripped() {
        assert!(file_matches(Path::new("src/auth/login.rs"), "./src/**"));
    }

    #[test]
    fn test_message_matches_case_insensitive_substring() {
        let fuse = annotation::Fuse {
            file: PathBuf::from("src/auth.rs"),
            line: 1,
            tag: "TODO".to_string(),
            date: fixed_today(),
            owner: None,
            message: "Remove legacy OAuth after migration".to_string(),
            status: annotation::Status::Inert,
            blamed_owner: None,
        };

        assert!(message_matches(&fuse, "oauth"));
        assert!(message_matches(&fuse, "LEGACY"));
        assert!(!message_matches(&fuse, "billing"));
    }

    // ── canonicalize_path ─────────────────────────────────────────────────────

    #[test]
    fn test_canonicalize_path_valid() {
        let dir = tempfile::tempdir().unwrap();
        let result = canonicalize_path(dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_canonicalize_path_invalid() {
        let result = canonicalize_path(std::path::Path::new("/no/such/path"));
        assert!(result.is_err());
    }

    // ── merge_file_config ─────────────────────────────────────────────────────

    #[test]
    fn test_merge_file_config_basic() {
        let file_cfg = config::ConfigFile {
            triggers: Some(vec!["TODO".to_string()]),
            fuse_days: Some(7),
            exclude: None,
            extensions: None,
            max_detonated: None,
            max_ticking: None,
        };
        let overrides = CliOverrides::default();
        let cfg = merge_file_config(file_cfg, &overrides).unwrap();
        assert_eq!(cfg.triggers, vec!["TODO"]);
        assert_eq!(cfg.fuse_days, 7);
    }

    #[test]
    fn test_merge_file_config_cli_overrides_fuse() {
        let file_cfg = config::ConfigFile {
            triggers: None,
            fuse_days: Some(7),
            exclude: None,
            extensions: None,
            max_detonated: None,
            max_ticking: None,
        };
        let overrides = CliOverrides::new(Some("30d".to_string()), false);
        let cfg = merge_file_config(file_cfg, &overrides).unwrap();
        // CLI should win
        assert_eq!(cfg.fuse_days, 30);
    }

    // ── resolve_config ────────────────────────────────────────────────────────

    #[test]
    fn test_resolve_config_no_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let overrides = CliOverrides::default();
        let cfg = resolve_config(None, dir.path(), &overrides).unwrap();
        // No config file in temp dir and no CWD match (temp dir != cwd)
        // Should get defaults
        assert!(cfg.triggers.contains(&"TODO".to_string()));
    }

    #[test]
    fn test_resolve_config_reads_scan_dir_config() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        {
            let mut f = std::fs::File::create(dir.path().join(".timebomb.toml")).unwrap();
            writeln!(f, "fuse_days = 99").unwrap();
        }
        let overrides = CliOverrides::default();
        let cfg = resolve_config(None, dir.path(), &overrides).unwrap();
        assert_eq!(cfg.fuse_days, 99);
    }

    #[test]
    fn test_resolve_config_explicit_config_wins() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();

        // Config in the scan dir
        {
            let mut f = std::fs::File::create(dir.path().join(".timebomb.toml")).unwrap();
            writeln!(f, "fuse_days = 7").unwrap();
        }

        // Explicit config file
        let explicit_cfg = dir.path().join("explicit.toml");
        {
            let mut f = std::fs::File::create(&explicit_cfg).unwrap();
            writeln!(f, "fuse_days = 99").unwrap();
        }

        let overrides = CliOverrides::default();
        let cfg =
            resolve_config(Some(explicit_cfg.to_str().unwrap()), dir.path(), &overrides).unwrap();
        // Explicit config should win over scan-dir config
        assert_eq!(cfg.fuse_days, 99);
    }
}
