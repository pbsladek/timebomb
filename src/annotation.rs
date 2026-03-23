use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The status of a fuse relative to a given "today" date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The deadline has already passed.
    Detonated,
    /// The deadline is within the configured warning window.
    Ticking,
    /// The deadline is comfortably in the future.
    Inert,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Detonated => "detonated",
            Status::Ticking => "ticking",
            Status::Inert => "inert",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single timebomb fuse found in a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fuse {
    /// Path to the file containing the fuse (relative to scan root).
    pub file: PathBuf,

    /// 1-based line number within the file.
    pub line: usize,

    /// The tag keyword, e.g. "TODO", "FIXME", "HACK".
    pub tag: String,

    /// The expiry date parsed from the fuse.
    #[serde(serialize_with = "serialize_naive_date")]
    #[serde(deserialize_with = "deserialize_naive_date")]
    pub date: NaiveDate,

    /// Optional owner extracted from the second bracket group, e.g. `[alice]`.
    pub owner: Option<String>,

    /// The descriptive message after the colon.
    pub message: String,

    /// Computed status relative to the "today" date used during scanning.
    pub status: Status,

    /// Owner inferred from git blame when no explicit `[owner]` bracket is present.
    /// Populated only when `--blame` is passed; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub blamed_owner: Option<String>,
}

impl Fuse {
    /// Compute the number of days until (or since) expiry relative to `today`.
    /// Positive means days remaining; negative means days overdue.
    pub fn days_from_today(&self, today: NaiveDate) -> i64 {
        (self.date - today).num_days()
    }

    /// Returns true if this fuse has already detonated.
    pub fn is_detonated(&self) -> bool {
        self.status == Status::Detonated
    }

    /// Returns true if this fuse is in the ticking window.
    pub fn is_ticking(&self) -> bool {
        self.status == Status::Ticking
    }

    /// Compute the status of a fuse given today's date and the fuse_days threshold.
    pub fn compute_status(date: NaiveDate, today: NaiveDate, fuse_days: u32) -> Status {
        if date < today {
            Status::Detonated
        } else {
            let days_remaining = (date - today).num_days();
            if days_remaining <= fuse_days as i64 {
                Status::Ticking
            } else {
                Status::Inert
            }
        }
    }

    /// Return a short location string like `src/main.rs:42`.
    pub fn location(&self) -> String {
        format!("{}:{}", self.file.display(), self.line)
    }

    /// Return the date formatted as YYYY-MM-DD.
    pub fn date_str(&self) -> String {
        self.date.format("%Y-%m-%d").to_string()
    }
}

// --- Serde helpers for NaiveDate as "YYYY-MM-DD" string ---

fn serialize_naive_date<S>(date: &NaiveDate, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&date.format("%Y-%m-%d").to_string())
}

fn deserialize_naive_date<'de, D>(deserializer: D) -> Result<NaiveDate, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn make_fuse(expiry: &str, status: Status) -> Fuse {
        Fuse {
            file: PathBuf::from("src/foo.rs"),
            line: 10,
            tag: "TODO".to_string(),
            date: date(expiry),
            owner: None,
            message: "some message".to_string(),
            status,
            blamed_owner: None,
        }
    }

    #[test]
    fn test_status_detonated() {
        let today = date("2025-06-01");
        let status = Fuse::compute_status(date("2025-01-01"), today, 14);
        assert_eq!(status, Status::Detonated);
    }

    #[test]
    fn test_status_ticking_boundary() {
        let today = date("2025-06-01");
        // Exactly on the warn boundary — 14 days from now
        let status = Fuse::compute_status(date("2025-06-15"), today, 14);
        assert_eq!(status, Status::Ticking);
    }

    #[test]
    fn test_status_ticking_within_window() {
        let today = date("2025-06-01");
        let status = Fuse::compute_status(date("2025-06-10"), today, 14);
        assert_eq!(status, Status::Ticking);
    }

    #[test]
    fn test_status_inert() {
        let today = date("2025-06-01");
        let status = Fuse::compute_status(date("2025-12-31"), today, 14);
        assert_eq!(status, Status::Inert);
    }

    #[test]
    fn test_status_expire_today_is_ticking() {
        // A fuse whose date IS today is considered ticking (date < today is false,
        // but days_remaining == 0 which is <= fuse_days — so Ticking).
        // Per spec, "deadline has passed" means strictly before today.
        let today = date("2025-06-01");
        let status = Fuse::compute_status(date("2025-06-01"), today, 0);
        // With fuse_days=0, days_remaining==0 is <= 0 => Ticking
        assert_eq!(status, Status::Ticking);
    }

    #[test]
    fn test_days_from_today_positive() {
        let today = date("2025-06-01");
        let fuse = make_fuse("2025-06-11", Status::Inert);
        assert_eq!(fuse.days_from_today(today), 10);
    }

    #[test]
    fn test_days_from_today_negative() {
        let today = date("2025-06-01");
        let fuse = make_fuse("2025-05-20", Status::Detonated);
        assert_eq!(fuse.days_from_today(today), -12);
    }

    #[test]
    fn test_location() {
        let fuse = make_fuse("2099-01-01", Status::Inert);
        assert_eq!(fuse.location(), "src/foo.rs:10");
    }

    #[test]
    fn test_date_str() {
        let fuse = make_fuse("2099-03-15", Status::Inert);
        assert_eq!(fuse.date_str(), "2099-03-15");
    }

    #[test]
    fn test_is_detonated() {
        let fuse = make_fuse("2020-01-01", Status::Detonated);
        assert!(fuse.is_detonated());
        assert!(!fuse.is_ticking());
    }

    #[test]
    fn test_is_ticking() {
        let fuse = make_fuse("2025-06-10", Status::Ticking);
        assert!(fuse.is_ticking());
        assert!(!fuse.is_detonated());
    }

    #[test]
    fn test_status_display() {
        assert_eq!(Status::Detonated.to_string(), "detonated");
        assert_eq!(Status::Ticking.to_string(), "ticking");
        assert_eq!(Status::Inert.to_string(), "inert");
    }

    #[test]
    fn test_serde_roundtrip() {
        let fuse = Fuse {
            file: PathBuf::from("src/lib.rs"),
            line: 99,
            tag: "FIXME".to_string(),
            date: date("2099-12-31"),
            owner: Some("alice".to_string()),
            message: "remove after upgrade".to_string(),
            status: Status::Inert,
            blamed_owner: None,
        };
        let json = serde_json::to_string(&fuse).unwrap();
        assert!(json.contains("2099-12-31"));
        assert!(json.contains("alice"));
        let decoded: Fuse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.date, fuse.date);
        assert_eq!(decoded.owner, fuse.owner);
        assert_eq!(decoded.tag, fuse.tag);
    }

    #[test]
    fn test_compute_status_zero_fuse_window() {
        let today = date("2025-06-01");
        // Future date with no fuse window → Inert
        let status = Fuse::compute_status(date("2025-06-02"), today, 0);
        assert_eq!(status, Status::Inert);
    }
}
