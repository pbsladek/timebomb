use std::fmt;
use std::io;
use std::path::PathBuf;

/// Top-level error type for timebomb
#[derive(Debug)]
pub enum Error {
    /// I/O error with optional path context
    Io {
        source: io::Error,
        path: Option<PathBuf>,
    },

    /// Failed to parse the config file
    ConfigParse {
        source: toml::de::Error,
        path: PathBuf,
    },

    /// Config file could not be read
    ConfigRead { source: io::Error, path: PathBuf },

    /// Regex compilation failed (should never happen at runtime if patterns are constant)
    RegexCompile(regex::Error),

    /// A date string in an annotation was not a valid calendar date
    InvalidDate {
        date_str: String,
        file: PathBuf,
        line: usize,
    },

    /// A glob pattern in the config was invalid
    InvalidGlob {
        pattern: String,
        source: globset::Error,
    },

    /// A CLI argument was semantically invalid (e.g. bad duration string)
    InvalidArgument(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io {
                source,
                path: Some(p),
            } => {
                write!(f, "I/O error reading '{}': {}", p.display(), source)
            }
            Error::Io { source, path: None } => {
                write!(f, "I/O error: {}", source)
            }
            Error::ConfigParse { source, path } => {
                write!(f, "Failed to parse config '{}': {}", path.display(), source)
            }
            Error::ConfigRead { source, path } => {
                write!(f, "Failed to read config '{}': {}", path.display(), source)
            }
            Error::RegexCompile(e) => {
                write!(f, "Regex compilation error: {}", e)
            }
            Error::InvalidDate {
                date_str,
                file,
                line,
            } => {
                write!(
                    f,
                    "Invalid date '{}' at {}:{} (expected YYYY-MM-DD)",
                    date_str,
                    file.display(),
                    line
                )
            }
            Error::InvalidGlob { pattern, source } => {
                write!(f, "Invalid glob pattern '{}': {}", pattern, source)
            }
            Error::InvalidArgument(msg) => {
                write!(f, "Invalid argument: {}", msg)
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::ConfigParse { source, .. } => Some(source),
            Error::ConfigRead { source, .. } => Some(source),
            Error::RegexCompile(e) => Some(e),
            Error::InvalidGlob { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<regex::Error> for Error {
    fn from(e: regex::Error) -> Self {
        Error::RegexCompile(e)
    }
}

/// Convenience result alias
pub type Result<T> = std::result::Result<T, Error>;

/// Maximum allowed value for `--fuse` (10 years).
///
/// Values beyond this are almost certainly mistakes (e.g. off-by-one on unit
/// conversion) and would suppress all warnings across any realistic codebase.
const MAX_FUSE_DAYS: u32 = 3_650;

/// Parse a duration string like "30d", "14d", "7d" into a number of days.
/// Only day-based durations are supported.
pub fn parse_duration_days(s: &str) -> Result<u32> {
    let s = s.trim();
    if let Some(num_str) = s.strip_suffix('d') {
        let days = num_str.parse::<u32>().map_err(|_| {
            Error::InvalidArgument(format!(
                "'{}' is not a valid duration — expected a format like '30d'",
                s
            ))
        })?;
        if days > MAX_FUSE_DAYS {
            return Err(Error::InvalidArgument(format!(
                "'{}' exceeds the maximum allowed fuse window of {}d (10 years)",
                s, MAX_FUSE_DAYS
            )));
        }
        Ok(days)
    } else {
        Err(Error::InvalidArgument(format!(
            "'{}' is not a valid duration — expected a format like '30d'",
            s
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_days_valid() {
        assert_eq!(parse_duration_days("30d").unwrap(), 30);
        assert_eq!(parse_duration_days("0d").unwrap(), 0);
        assert_eq!(parse_duration_days("365d").unwrap(), 365);
        assert_eq!(parse_duration_days("  14d  ").unwrap(), 14);
    }

    #[test]
    fn test_parse_duration_days_cap() {
        assert!(parse_duration_days("3650d").is_ok());
        assert!(parse_duration_days("3651d").is_err());
        assert!(parse_duration_days("9999999d").is_err());
    }

    #[test]
    fn test_parse_duration_days_invalid() {
        assert!(parse_duration_days("30").is_err());
        assert!(parse_duration_days("abc").is_err());
        assert!(parse_duration_days("30h").is_err());
        assert!(parse_duration_days("").is_err());
        assert!(parse_duration_days("-5d").is_err());
    }

    #[test]
    fn test_error_display_io_with_path() {
        let err = Error::Io {
            source: io::Error::new(io::ErrorKind::NotFound, "not found"),
            path: Some(PathBuf::from("/some/file.rs")),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("/some/file.rs"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn test_error_display_io_without_path() {
        let err = Error::Io {
            source: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
            path: None,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("denied"));
    }

    #[test]
    fn test_error_display_invalid_date() {
        let err = Error::InvalidDate {
            date_str: "2026-13-45".to_string(),
            file: PathBuf::from("src/main.rs"),
            line: 42,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("2026-13-45"));
        assert!(msg.contains("src/main.rs"));
        assert!(msg.contains("42"));
    }

    #[test]
    fn test_error_display_invalid_argument() {
        let err = Error::InvalidArgument("bad value".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("bad value"));
    }
}
