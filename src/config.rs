use crate::error::{parse_duration_days, Error, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The structure of `.timebomb.toml` on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// Tags to scan for.
    #[serde(default)]
    pub triggers: Option<Vec<String>>,

    /// Warn (and optionally fail) if a fuse expires within this many days.
    #[serde(default)]
    pub fuse_days: Option<u32>,

    /// Glob patterns to exclude from scanning.
    #[serde(default)]
    pub exclude: Option<Vec<String>>,

    /// File extensions to scan. If empty/absent, scan all text files.
    #[serde(default)]
    pub extensions: Option<Vec<String>>,

    /// Ratchet: fail if the number of detonated fuses exceeds this limit.
    #[serde(default)]
    pub max_detonated: Option<usize>,

    /// Ratchet: fail if the number of ticking fuses exceeds this limit.
    #[serde(default)]
    pub max_ticking: Option<usize>,
}

/// Fully-resolved configuration after merging the config file with CLI overrides.
#[derive(Debug, Clone)]
pub struct Config {
    pub triggers: Vec<String>,
    pub fuse_days: u32,
    pub exclude_patterns: Vec<String>,
    pub extensions: Vec<String>,
    /// Whether to fail (exit 1) when ticking items are found (--fail-on-ticking).
    pub fail_on_ticking: bool,
    /// If Some, only scan files whose relative path is in this set (--since git-diff mode).
    /// None means scan all files (normal mode).
    pub diff_files: Option<std::collections::HashSet<std::path::PathBuf>>,
    /// Ratchet: fail if the number of detonated fuses exceeds this limit.
    pub max_detonated: Option<usize>,
    /// Ratchet: fail if the number of ticking fuses exceeds this limit.
    pub max_ticking: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            triggers: default_triggers(),
            fuse_days: 0,
            exclude_patterns: default_excludes(),
            extensions: default_extensions(),
            fail_on_ticking: false,
            diff_files: None,
            max_detonated: None,
            max_ticking: None,
        }
    }
}

fn default_triggers() -> Vec<String> {
    vec![
        "TODO".to_string(),
        "FIXME".to_string(),
        "HACK".to_string(),
        "TEMP".to_string(),
        "REMOVEME".to_string(),
        "DEBT".to_string(),
        "STOPSHIP".to_string(),
        "WORKAROUND".to_string(),
        "DEPRECATED".to_string(),
        "BUG".to_string(),
    ]
}

fn default_excludes() -> Vec<String> {
    vec![
        "vendor/**".to_string(),
        "node_modules/**".to_string(),
        "*.min.js".to_string(),
        ".git/**".to_string(),
    ]
}

fn default_extensions() -> Vec<String> {
    vec![
        "rs".to_string(),
        "go".to_string(),
        "ts".to_string(),
        "js".to_string(),
        "py".to_string(),
        "rb".to_string(),
        "java".to_string(),
        "cs".to_string(),
        "fs".to_string(),
        "hs".to_string(),
        "php".to_string(),
        "clj".to_string(),
        "lisp".to_string(),
        "rkt".to_string(),
        "ex".to_string(),
        "erl".to_string(),
        "c".to_string(),
        "cpp".to_string(),
        "d".to_string(),
        "swift".to_string(),
        "ml".to_string(),
        "lua".to_string(),
        "dart".to_string(),
        "kt".to_string(),
        "sql".to_string(),
        "tf".to_string(),
        "yaml".to_string(),
        "yml".to_string(),
    ]
}

/// CLI-level overrides that can be applied on top of a `Config`.
#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    /// `--fuse 30d`
    pub fuse: Option<String>,
    /// `--fail-on-ticking`
    pub fail_on_ticking: bool,
}

impl CliOverrides {
    pub fn new(fuse: Option<String>, fail_on_ticking: bool) -> Self {
        CliOverrides {
            fuse,
            fail_on_ticking,
        }
    }
}

/// Load `.timebomb.toml` from `root_dir` if it exists, merge with defaults and CLI overrides.
///
/// Only `root_dir/.timebomb.toml` is checked. CWD fallback is the caller's responsibility
/// (handled in `main.rs`) so that tests using temp directories are not affected by a
/// `.timebomb.toml` that happens to exist in the current working directory.
///
/// Returns a fully-resolved `Config`.
pub fn load_config(root_dir: &Path, overrides: &CliOverrides) -> Result<Config> {
    let config_path = root_dir.join(".timebomb.toml");
    let file_cfg = if config_path.exists() {
        Some(read_config_file(&config_path)?)
    } else {
        None
    };

    merge_config(file_cfg, overrides)
}

fn read_config_file(path: &Path) -> Result<ConfigFile> {
    let content = std::fs::read_to_string(path).map_err(|e| Error::ConfigRead {
        source: e,
        path: path.to_path_buf(),
    })?;
    toml::from_str(&content).map_err(|e| Error::ConfigParse {
        source: e,
        path: path.to_path_buf(),
    })
}

fn merge_config(file_cfg: Option<ConfigFile>, overrides: &CliOverrides) -> Result<Config> {
    let defaults = Config::default();

    let triggers = file_cfg
        .as_ref()
        .and_then(|c| c.triggers.clone())
        .unwrap_or(defaults.triggers);

    let mut fuse_days = file_cfg
        .as_ref()
        .and_then(|c| c.fuse_days)
        .unwrap_or(defaults.fuse_days);

    let exclude_patterns = file_cfg
        .as_ref()
        .and_then(|c| c.exclude.clone())
        .unwrap_or(defaults.exclude_patterns);

    let extensions = file_cfg
        .as_ref()
        .and_then(|c| c.extensions.clone())
        .unwrap_or(defaults.extensions);

    let max_detonated = file_cfg.as_ref().and_then(|c| c.max_detonated);
    let max_ticking = file_cfg.as_ref().and_then(|c| c.max_ticking);

    // Apply CLI overrides
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
        max_detonated,
        max_ticking,
    })
}

impl Config {
    /// Build a `GlobSet` from the exclude patterns for fast path matching.
    pub fn build_exclude_globset(&self) -> Result<GlobSet> {
        let mut builder = GlobSetBuilder::new();
        for pattern in &self.exclude_patterns {
            let glob = Glob::new(pattern).map_err(|e| Error::InvalidGlob {
                pattern: pattern.clone(),
                source: e,
            })?;
            builder.add(glob);
        }
        builder.build().map_err(|e| Error::InvalidGlob {
            pattern: "(combined)".to_string(),
            source: e,
        })
    }

    /// Return true if the given path should be excluded per exclude globs.
    /// `path` should be relative to the scan root for glob matching to work correctly.
    pub fn is_excluded(&self, path: &Path, globset: &GlobSet) -> bool {
        // Match against the full relative path string and each component
        if globset.is_match(path) {
            return true;
        }
        // Also try matching just the filename so that patterns like `*.min.js`
        // correctly exclude nested files (e.g. `dist/app.min.js`).
        if let Some(fname) = path.file_name() {
            if globset.is_match(Path::new(fname)) {
                return true;
            }
        }
        false
    }

    /// Return true if the file extension is in the allowed list.
    /// If the extensions list is empty, all files are considered eligible.
    pub fn extension_allowed(&self, path: &Path) -> bool {
        if self.extensions.is_empty() {
            return true;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => self
                .extensions
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(ext)),
            None => false,
        }
    }

    /// Build the fuse regex pattern from the configured triggers.
    pub fn fuse_regex_pattern(&self) -> String {
        let triggers_alternation = self.triggers.join("|");
        format!(
            r"(?i)({tags})\[(\d{{4}}-\d{{2}}-\d{{2}})\](\[([^\]]+)\])?:\s*(.+)",
            tags = triggers_alternation
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_toml(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert!(cfg.triggers.contains(&"TODO".to_string()));
        assert!(cfg.triggers.contains(&"FIXME".to_string()));
        assert_eq!(cfg.fuse_days, 0);
        assert!(!cfg.extensions.is_empty());
        assert!(!cfg.fail_on_ticking);
    }

    #[test]
    fn test_merge_no_file_no_overrides() {
        let cfg = merge_config(None, &CliOverrides::default()).unwrap();
        assert_eq!(cfg.triggers, default_triggers());
        assert_eq!(cfg.fuse_days, 0);
    }

    #[test]
    fn test_merge_file_overrides_triggers() {
        let file_cfg = ConfigFile {
            triggers: Some(vec!["TODO".to_string(), "FIXME".to_string()]),
            fuse_days: Some(7),
            exclude: None,
            extensions: None,
            max_detonated: None,
            max_ticking: None,
        };
        let cfg = merge_config(Some(file_cfg), &CliOverrides::default()).unwrap();
        assert_eq!(cfg.triggers, vec!["TODO", "FIXME"]);
        assert_eq!(cfg.fuse_days, 7);
        // Extensions should fall back to defaults
        assert!(cfg.extensions.contains(&"rs".to_string()));
    }

    #[test]
    fn test_cli_override_fuse() {
        let overrides = CliOverrides::new(Some("30d".to_string()), false);
        let cfg = merge_config(None, &overrides).unwrap();
        assert_eq!(cfg.fuse_days, 30);
    }

    #[test]
    fn test_cli_override_fail_on_ticking() {
        let overrides = CliOverrides::new(None, true);
        let cfg = merge_config(None, &overrides).unwrap();
        assert!(cfg.fail_on_ticking);
    }

    #[test]
    fn test_cli_fuse_overrides_file() {
        let file_cfg = ConfigFile {
            triggers: None,
            fuse_days: Some(7),
            exclude: None,
            extensions: None,
            max_detonated: None,
            max_ticking: None,
        };
        let overrides = CliOverrides::new(Some("30d".to_string()), false);
        let cfg = merge_config(Some(file_cfg), &overrides).unwrap();
        // CLI should win
        assert_eq!(cfg.fuse_days, 30);
    }

    #[test]
    fn test_cli_invalid_duration() {
        let overrides = CliOverrides::new(Some("notadate".to_string()), false);
        let result = merge_config(None, &overrides);
        assert!(result.is_err());
    }

    #[test]
    fn test_extension_allowed_rs() {
        let cfg = Config::default();
        assert!(cfg.extension_allowed(Path::new("src/main.rs")));
        assert!(cfg.extension_allowed(Path::new("src/lib.go")));
    }

    #[test]
    fn test_extension_allowed_unknown() {
        let cfg = Config::default();
        assert!(!cfg.extension_allowed(Path::new("file.xyz")));
        assert!(!cfg.extension_allowed(Path::new("Makefile")));
    }

    #[test]
    fn test_extension_empty_allows_all() {
        let cfg = Config {
            extensions: vec![],
            ..Config::default()
        };
        assert!(cfg.extension_allowed(Path::new("anything.xyz")));
        assert!(cfg.extension_allowed(Path::new("Makefile")));
    }

    #[test]
    fn test_is_excluded_git() {
        let cfg = Config::default();
        let gs = cfg.build_exclude_globset().unwrap();
        assert!(cfg.is_excluded(Path::new(".git/config"), &gs));
        assert!(cfg.is_excluded(Path::new(".git/HEAD"), &gs));
    }

    #[test]
    fn test_is_excluded_node_modules() {
        let cfg = Config::default();
        let gs = cfg.build_exclude_globset().unwrap();
        assert!(cfg.is_excluded(Path::new("node_modules/lodash/index.js"), &gs));
    }

    #[test]
    fn test_is_not_excluded_src() {
        let cfg = Config::default();
        let gs = cfg.build_exclude_globset().unwrap();
        assert!(!cfg.is_excluded(Path::new("src/main.rs"), &gs));
    }

    #[test]
    fn test_fuse_regex_pattern_contains_triggers() {
        let cfg = Config::default();
        let pattern = cfg.fuse_regex_pattern();
        assert!(pattern.contains("TODO"));
        assert!(pattern.contains("FIXME"));
        assert!(pattern.contains("HACK"));
        assert!(pattern.contains("TEMP"));
        assert!(pattern.contains("REMOVEME"));
        assert!(pattern.contains("DEBT"));
        assert!(pattern.contains("STOPSHIP"));
        assert!(pattern.contains("WORKAROUND"));
        assert!(pattern.contains("DEPRECATED"));
        assert!(pattern.contains("BUG"));
    }

    #[test]
    fn test_read_config_file_valid() {
        let toml_content = r#"
triggers = ["TODO", "FIXME"]
fuse_days = 14
exclude = ["vendor/**"]
extensions = ["rs", "go"]
"#;
        let f = write_toml(toml_content);
        let cfg_file = read_config_file(f.path()).unwrap();
        assert_eq!(cfg_file.triggers.unwrap(), vec!["TODO", "FIXME"]);
        assert_eq!(cfg_file.fuse_days.unwrap(), 14);
        assert_eq!(cfg_file.exclude.unwrap(), vec!["vendor/**"]);
        assert_eq!(cfg_file.extensions.unwrap(), vec!["rs", "go"]);
    }

    #[test]
    fn test_read_config_file_empty() {
        let f = write_toml("");
        let cfg_file = read_config_file(f.path()).unwrap();
        assert!(cfg_file.triggers.is_none());
        assert!(cfg_file.fuse_days.is_none());
    }

    #[test]
    fn test_read_config_file_invalid_toml() {
        let f = write_toml("this is not valid toml ][[[");
        let result = read_config_file(f.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_read_config_file_not_found() {
        let result = read_config_file(Path::new("/nonexistent/path/.timebomb.toml"));
        assert!(result.is_err());
    }
}
