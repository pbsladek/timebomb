//! Priority view for fuses that need attention soon.

use crate::annotation::{Fuse, Status};
use chrono::NaiveDate;
use std::io::{self, Write};

/// Return detonated and ticking fuses ordered by urgency.
pub fn select_armory_fuses(fuses: &[Fuse], today: NaiveDate, limit: usize) -> Vec<&Fuse> {
    let mut selected: Vec<&Fuse> = fuses
        .iter()
        .filter(|fuse| matches!(fuse.status, Status::Detonated | Status::Ticking))
        .collect();

    selected.sort_unstable_by(|a, b| {
        armory_status_order(&a.status)
            .cmp(&armory_status_order(&b.status))
            .then(a.days_from_today(today).cmp(&b.days_from_today(today)))
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });
    selected.truncate(limit);
    selected
}

/// Write the armory view to the supplied writer.
pub fn print_armory_to_writer<W: Write>(
    fuses: &[&Fuse],
    today: NaiveDate,
    oldest: bool,
    mut writer: W,
) -> io::Result<()> {
    let heading = if oldest {
        "Most volatile fuse"
    } else {
        "Most volatile fuses"
    };
    writeln!(writer, "{heading}")?;

    if fuses.is_empty() {
        writeln!(writer)?;
        writeln!(writer, "Magazine is quiet.")?;
        return Ok(());
    }

    for (idx, fuse) in fuses.iter().enumerate() {
        let days = fuse.days_from_today(today);
        let status = match fuse.status {
            Status::Detonated => "DETONATED",
            Status::Ticking => "TICKING",
            Status::Inert => "INERT",
        };
        let delta = if days < 0 {
            format!("{}d overdue", days.abs())
        } else {
            format!("{}d left", days)
        };

        writeln!(
            writer,
            "{}. {:<9} {:>11}  {}:{}",
            idx + 1,
            status,
            delta,
            fuse.file.display(),
            fuse.line
        )?;
        writeln!(writer, "   {}", fuse.annotation_text())?;

        if idx + 1 < fuses.len() {
            writeln!(writer)?;
        }
    }

    Ok(())
}

/// Print the armory view to stdout.
pub fn print_armory(fuses: &[&Fuse], today: NaiveDate, oldest: bool) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    // stdout write failures are not expected in normal CLI use; keep the public
    // command path simple like the other terminal renderers in this crate.
    let _ = print_armory_to_writer(fuses, today, oldest, &mut handle);
}

fn armory_status_order(status: &Status) -> u8 {
    match status {
        Status::Detonated => 0,
        Status::Ticking => 1,
        Status::Inert => 2,
    }
}

trait FuseAnnotationText {
    fn annotation_text(&self) -> String;
}

impl FuseAnnotationText for Fuse {
    fn annotation_text(&self) -> String {
        match self.owner.as_deref() {
            Some(owner) => format!(
                "{}[{}][{}]: {}",
                self.tag,
                self.date_str(),
                owner,
                self.message
            ),
            None => format!("{}[{}]: {}", self.tag, self.date_str(), self.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn fuse(file: &str, line: usize, expiry: &str, status: Status, message: &str) -> Fuse {
        Fuse {
            file: PathBuf::from(file),
            line,
            tag: "TODO".to_string(),
            date: date(expiry),
            owner: None,
            message: message.to_string(),
            status,
            blamed_owner: None,
        }
    }

    #[test]
    fn test_select_armory_fuses_ranks_detonated_then_ticking() {
        let today = date("2026-04-18");
        let fuses = vec![
            fuse("soon.rs", 1, "2026-04-19", Status::Ticking, "one day"),
            fuse("old.rs", 1, "2026-04-01", Status::Detonated, "old"),
            fuse("older.rs", 1, "2026-03-01", Status::Detonated, "older"),
            fuse("later.rs", 1, "2026-04-25", Status::Ticking, "later"),
            fuse("safe.rs", 1, "2026-12-01", Status::Inert, "safe"),
        ];

        let selected = select_armory_fuses(&fuses, today, 10);

        assert_eq!(selected.len(), 4);
        assert_eq!(selected[0].file, PathBuf::from("older.rs"));
        assert_eq!(selected[1].file, PathBuf::from("old.rs"));
        assert_eq!(selected[2].file, PathBuf::from("soon.rs"));
        assert_eq!(selected[3].file, PathBuf::from("later.rs"));
    }

    #[test]
    fn test_select_armory_fuses_honors_limit() {
        let today = date("2026-04-18");
        let fuses = vec![
            fuse("a.rs", 1, "2026-04-01", Status::Detonated, "a"),
            fuse("b.rs", 1, "2026-04-02", Status::Detonated, "b"),
            fuse("c.rs", 1, "2026-04-03", Status::Detonated, "c"),
        ];

        let selected = select_armory_fuses(&fuses, today, 2);

        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_print_armory_to_writer_empty() {
        let today = date("2026-04-18");
        let mut out = Vec::new();

        print_armory_to_writer(&[], today, false, &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Most volatile fuses"));
        assert!(text.contains("Magazine is quiet."));
    }

    #[test]
    fn test_print_armory_to_writer_includes_annotation() {
        let today = date("2026-04-18");
        let mut item = fuse(
            "src/auth.rs",
            42,
            "2026-04-01",
            Status::Detonated,
            "remove fallback",
        );
        item.owner = Some("alice".to_string());
        let selected = vec![&item];
        let mut out = Vec::new();

        print_armory_to_writer(&selected, today, false, &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("DETONATED"));
        assert!(text.contains("17d overdue"));
        assert!(text.contains("src/auth.rs:42"));
        assert!(text.contains("TODO[2026-04-01][alice]: remove fallback"));
    }

    #[test]
    fn test_print_armory_to_writer_oldest_heading() {
        let today = date("2026-04-18");
        let item = fuse("src/auth.rs", 42, "2026-04-01", Status::Detonated, "old");
        let selected = vec![&item];
        let mut out = Vec::new();

        print_armory_to_writer(&selected, today, true, &mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("Most volatile fuse\n"));
    }
}
