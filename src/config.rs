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
    pub tags: Option<Vec<String>>,

    /// Warn (and optionally fail) if an annotation expires within this many days.
    #[serde(default)]
    pub warn_within_days: Option<u32>,

    /// Glob patterns to exclude from scanning.
    #[serde(default)]
    pub exclude: Option<Vec<String>>,

    /// File extensions to scan. If empty/absent, scan all text files.
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
}

/// Fully-resolved configuration after merging the config file with CLI overrides.
#[derive(Debug, Clone)]
pub struct Config {
    pub tags: Vec<String>,
    pub warn_within_days: u32,
    pub exclude_patterns: Vec<String>,
    pub extensions: Vec<String>,
    /// Whether to fail (exit 1) when warn-threshold items are found (--fail-on-warn).
    pub fail_on_warn: bool,
    /// If Some, only scan files whose relative path is in this set (--since git-diff mode).
    /// None means scan all files (normal mode).
    pub diff_files: Option<std::collections::HashSet<std::path::PathBuf>>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tags: default_tags(),
            warn_within_days: 0,
            exclude_patterns: default_excludes(),
            extensions: default_extensions(),
            fail_on_warn: false,
            diff_files: None,
        }
    }
}

fn default_tags() -> Vec<String> {
    vec![
        "TODO".to_string(),
        "FIXME".to_string(),
        "HACK".to_string(),
        "TEMP".to_string(),
        "REMOVEME".to_string(),
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
        "sql".to_string(),
        "tf".to_string(),
        "yaml".to_string(),
        "yml".to_string(),
    ]
}

/// CLI-level overrides that can be applied on top of a `Config`.
#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    /// `--warn-within 30d`
    pub warn_within: Option<String>,
    /// `--fail-on-warn`
    pub fail_on_warn: bool,
}

impl CliOverrides {
    pub fn new(warn_within: Option<String>, fail_on_warn: bool) -> Self {
        CliOverrides {
            warn_within,
            fail_on_warn,
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

    let tags = file_cfg
        .as_ref()
        .and_then(|c| c.tags.clone())
        .unwrap_or(defaults.tags);

    let mut warn_within_days = file_cfg
        .as_ref()
        .and_then(|c| c.warn_within_days)
        .unwrap_or(defaults.warn_within_days);

    let exclude_patterns = file_cfg
        .as_ref()
        .and_then(|c| c.exclude.clone())
        .unwrap_or(defaults.exclude_patterns);

    let extensions = file_cfg
        .as_ref()
        .and_then(|c| c.extensions.clone())
        .unwrap_or(defaults.extensions);

    // Apply CLI overrides
    if let Some(ref w) = overrides.warn_within {
        warn_within_days = parse_duration_days(w)?;
    }

    Ok(Config {
        tags,
        warn_within_days,
        exclude_patterns,
        extensions,
        fail_on_warn: overrides.fail_on_warn,
        diff_files: None,
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
        // Also try matching just the filename
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

    /// Build the annotation regex pattern from the configured tags.
    pub fn annotation_regex_pattern(&self) -> String {
        let tags_alternation = self.tags.join("|");
        format!(
            r"(?i)({tags})\[(\d{{4}}-\d{{2}}-\d{{2}})\](\[([^\]]+)\])?:\s*(.+)",
            tags = tags_alternation
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
        assert!(cfg.tags.contains(&"TODO".to_string()));
        assert!(cfg.tags.contains(&"FIXME".to_string()));
        assert_eq!(cfg.warn_within_days, 0);
        assert!(!cfg.extensions.is_empty());
        assert!(!cfg.fail_on_warn);
    }

    #[test]
    fn test_merge_no_file_no_overrides() {
        let cfg = merge_config(None, &CliOverrides::default()).unwrap();
        assert_eq!(cfg.tags, default_tags());
        assert_eq!(cfg.warn_within_days, 0);
    }

    #[test]
    fn test_merge_file_overrides_tags() {
        let file_cfg = ConfigFile {
            tags: Some(vec!["TODO".to_string(), "FIXME".to_string()]),
            warn_within_days: Some(7),
            exclude: None,
            extensions: None,
        };
        let cfg = merge_config(Some(file_cfg), &CliOverrides::default()).unwrap();
        assert_eq!(cfg.tags, vec!["TODO", "FIXME"]);
        assert_eq!(cfg.warn_within_days, 7);
        // Extensions should fall back to defaults
        assert!(cfg.extensions.contains(&"rs".to_string()));
    }

    #[test]
    fn test_cli_override_warn_within() {
        let overrides = CliOverrides::new(Some("30d".to_string()), false);
        let cfg = merge_config(None, &overrides).unwrap();
        assert_eq!(cfg.warn_within_days, 30);
    }

    #[test]
    fn test_cli_override_fail_on_warn() {
        let overrides = CliOverrides::new(None, true);
        let cfg = merge_config(None, &overrides).unwrap();
        assert!(cfg.fail_on_warn);
    }

    #[test]
    fn test_cli_warn_within_overrides_file() {
        let file_cfg = ConfigFile {
            tags: None,
            warn_within_days: Some(7),
            exclude: None,
            extensions: None,
        };
        let overrides = CliOverrides::new(Some("30d".to_string()), false);
        let cfg = merge_config(Some(file_cfg), &overrides).unwrap();
        // CLI should win
        assert_eq!(cfg.warn_within_days, 30);
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
    fn test_annotation_regex_pattern_contains_tags() {
        let cfg = Config::default();
        let pattern = cfg.annotation_regex_pattern();
        assert!(pattern.contains("TODO"));
        assert!(pattern.contains("FIXME"));
        assert!(pattern.contains("HACK"));
        assert!(pattern.contains("TEMP"));
        assert!(pattern.contains("REMOVEME"));
    }

    #[test]
    fn test_read_config_file_valid() {
        let toml_content = r#"
tags = ["TODO", "FIXME"]
warn_within_days = 14
exclude = ["vendor/**"]
extensions = ["rs", "go"]
"#;
        let f = write_toml(toml_content);
        let cfg_file = read_config_file(f.path()).unwrap();
        assert_eq!(cfg_file.tags.unwrap(), vec!["TODO", "FIXME"]);
        assert_eq!(cfg_file.warn_within_days.unwrap(), 14);
        assert_eq!(cfg_file.exclude.unwrap(), vec!["vendor/**"]);
        assert_eq!(cfg_file.extensions.unwrap(), vec!["rs", "go"]);
    }

    #[test]
    fn test_read_config_file_empty() {
        let f = write_toml("");
        let cfg_file = read_config_file(f.path()).unwrap();
        assert!(cfg_file.tags.is_none());
        assert!(cfg_file.warn_within_days.is_none());
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
