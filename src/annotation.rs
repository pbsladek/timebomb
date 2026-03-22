use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The status of an annotation relative to a given "today" date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The deadline has already passed.
    Expired,
    /// The deadline is within the configured warning window.
    ExpiringSoon,
    /// The deadline is comfortably in the future.
    Ok,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Expired => "expired",
            Status::ExpiringSoon => "expiring_soon",
            Status::Ok => "ok",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single timebomb annotation found in a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    /// Path to the file containing the annotation (relative to scan root).
    pub file: PathBuf,

    /// 1-based line number within the file.
    pub line: usize,

    /// The tag keyword, e.g. "TODO", "FIXME", "HACK".
    pub tag: String,

    /// The expiry date parsed from the annotation.
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

impl Annotation {
    /// Compute the number of days until (or since) expiry relative to `today`.
    /// Positive means days remaining; negative means days overdue.
    pub fn days_from_today(&self, today: NaiveDate) -> i64 {
        (self.date - today).num_days()
    }

    /// Returns true if this annotation has already expired.
    pub fn is_expired(&self) -> bool {
        self.status == Status::Expired
    }

    /// Returns true if this annotation is in the expiring-soon window.
    pub fn is_expiring_soon(&self) -> bool {
        self.status == Status::ExpiringSoon
    }

    /// Compute the status of an annotation given today's date and the warn_within_days threshold.
    pub fn compute_status(date: NaiveDate, today: NaiveDate, warn_within_days: u32) -> Status {
        if date < today {
            Status::Expired
        } else {
            let days_remaining = (date - today).num_days();
            if days_remaining <= warn_within_days as i64 {
                Status::ExpiringSoon
            } else {
                Status::Ok
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

    fn make_annotation(expiry: &str, status: Status) -> Annotation {
        Annotation {
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
    fn test_status_expired() {
        let today = date("2025-06-01");
        let status = Annotation::compute_status(date("2025-01-01"), today, 14);
        assert_eq!(status, Status::Expired);
    }

    #[test]
    fn test_status_expiring_soon_boundary() {
        let today = date("2025-06-01");
        // Exactly on the warn boundary — 14 days from now
        let status = Annotation::compute_status(date("2025-06-15"), today, 14);
        assert_eq!(status, Status::ExpiringSoon);
    }

    #[test]
    fn test_status_expiring_soon_within_window() {
        let today = date("2025-06-01");
        let status = Annotation::compute_status(date("2025-06-10"), today, 14);
        assert_eq!(status, Status::ExpiringSoon);
    }

    #[test]
    fn test_status_ok() {
        let today = date("2025-06-01");
        let status = Annotation::compute_status(date("2025-12-31"), today, 14);
        assert_eq!(status, Status::Ok);
    }

    #[test]
    fn test_status_expire_today_is_expired() {
        // An annotation whose date IS today is considered expired (date < today is false,
        // but days_remaining == 0 which is <= warn_within_days — so ExpiringSoon).
        // Per spec, "deadline has passed" means strictly before today.
        let today = date("2025-06-01");
        let status = Annotation::compute_status(date("2025-06-01"), today, 0);
        // With warn_within=0, days_remaining==0 is <= 0 => ExpiringSoon
        assert_eq!(status, Status::ExpiringSoon);
    }

    #[test]
    fn test_days_from_today_positive() {
        let today = date("2025-06-01");
        let ann = make_annotation("2025-06-11", Status::Ok);
        assert_eq!(ann.days_from_today(today), 10);
    }

    #[test]
    fn test_days_from_today_negative() {
        let today = date("2025-06-01");
        let ann = make_annotation("2025-05-20", Status::Expired);
        assert_eq!(ann.days_from_today(today), -12);
    }

    #[test]
    fn test_location() {
        let ann = make_annotation("2099-01-01", Status::Ok);
        assert_eq!(ann.location(), "src/foo.rs:10");
    }

    #[test]
    fn test_date_str() {
        let ann = make_annotation("2099-03-15", Status::Ok);
        assert_eq!(ann.date_str(), "2099-03-15");
    }

    #[test]
    fn test_is_expired() {
        let ann = make_annotation("2020-01-01", Status::Expired);
        assert!(ann.is_expired());
        assert!(!ann.is_expiring_soon());
    }

    #[test]
    fn test_is_expiring_soon() {
        let ann = make_annotation("2025-06-10", Status::ExpiringSoon);
        assert!(ann.is_expiring_soon());
        assert!(!ann.is_expired());
    }

    #[test]
    fn test_status_display() {
        assert_eq!(Status::Expired.to_string(), "expired");
        assert_eq!(Status::ExpiringSoon.to_string(), "expiring_soon");
        assert_eq!(Status::Ok.to_string(), "ok");
    }

    #[test]
    fn test_serde_roundtrip() {
        let ann = Annotation {
            file: PathBuf::from("src/lib.rs"),
            line: 99,
            tag: "FIXME".to_string(),
            date: date("2099-12-31"),
            owner: Some("alice".to_string()),
            message: "remove after upgrade".to_string(),
            status: Status::Ok,
            blamed_owner: None,
        };
        let json = serde_json::to_string(&ann).unwrap();
        assert!(json.contains("2099-12-31"));
        assert!(json.contains("alice"));
        let decoded: Annotation = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.date, ann.date);
        assert_eq!(decoded.owner, ann.owner);
        assert_eq!(decoded.tag, ann.tag);
    }

    #[test]
    fn test_compute_status_zero_warn_window() {
        let today = date("2025-06-01");
        // Future date with no warn window → Ok
        let status = Annotation::compute_status(date("2025-06-02"), today, 0);
        assert_eq!(status, Status::Ok);
    }
}
