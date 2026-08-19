//! What the application has done this session, in the order it did it.
//!
//! An interface answers one thing at a time and then moves on: the error of an
//! analysis that failed is replaced by the next one, a scan reports its
//! matches in the view that ran it, an export says where it wrote and is gone
//! by the time the reader has changed view. None of it survives the moment it
//! happened, and someone who looked away comes back to a window that no longer
//! remembers.
//!
//! This is that memory — one line per thing the application did — and it is
//! kept in this process and nowhere else. A record of a session is a record of
//! which files someone opened, so it is never written to disk and never leaves
//! the machine; closing Desdec is what clears it.

use std::sync::OnceLock;

use time::{OffsetDateTime, UtcOffset};

/// How much a line matters, which is the only thing that colours it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    /// Something the application did, and did successfully.
    Note,
    /// Something it could not do, that it carried on without.
    Warning,
    /// Something it was asked for and could not deliver.
    Failure,
}

/// One line of the journal.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// When it happened, as an instant rather than as a reading of a clock.
    ///
    /// Kept in UTC and turned into the reader's own time only when it is
    /// drawn: an offset captured once at startup describes the machine, not
    /// the moment, and storing what a clock said would make every line depend
    /// on that conversion having already happened.
    pub at: OffsetDateTime,
    pub level: Level,
    pub text: String,
}

/// The machine's offset from UTC, read once before any thread exists.
static LOCAL_OFFSET: OnceLock<UtcOffset> = OnceLock::new();

/// Reads the local time offset, and keeps it for the rest of the run.
///
/// Called from `main` before anything else: on Unix the answer comes from an
/// environment the process can change under itself, so the library refuses to
/// answer once a second thread is running — and this application starts
/// several. Asking at the one moment it can be answered is what lets the
/// account be stamped with the reader's own time rather than with UTC.
///
/// A machine that will not say keeps the account in UTC: two hours wrong for
/// some readers, but never a made-up offset.
pub fn capture_local_offset() {
    let _ = LOCAL_OFFSET.set(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC));
}

/// The offset the stamps are drawn in.
#[must_use]
pub fn local_offset() -> UtcOffset {
    *LOCAL_OFFSET.get_or_init(|| UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
}

/// How long a session's account can grow before its oldest lines are dropped.
///
/// A long session with a decompiler that is retried, or an assistant asked
/// again and again, would otherwise keep every line of it for the rest of the
/// run. The end is what a reader reads.
const LIMIT: usize = 1000;

/// The account of one session, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Journal {
    entries: Vec<Entry>,
}

impl Journal {
    /// Records what just happened, stamped with the moment it did.
    pub fn record(&mut self, level: Level, text: impl Into<String>) {
        self.record_at(OffsetDateTime::now_utc(), level, text);
    }

    /// The same, at a given instant, so a test can say when.
    pub fn record_at(&mut self, at: OffsetDateTime, level: Level, text: impl Into<String>) {
        self.entries.push(Entry {
            at,
            level,
            text: text.into(),
        });
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The most recent line, which is the one the status bar shows.
    #[must_use]
    pub fn last(&self) -> Option<&Entry> {
        self.entries.last()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The whole account as text, for the clipboard.
    ///
    /// Stamped exactly as it is on screen: what is copied has to be what was
    /// read, or a line quoted in a bug report says something else.
    #[must_use]
    pub fn as_text(&self) -> String {
        self.entries
            .iter()
            .map(|entry| format!("{}  {}", stamp(entry.at), entry.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A moment, as the clock on the reader's own wall showed it.
#[must_use]
pub fn stamp(at: OffsetDateTime) -> String {
    stamp_in(at, local_offset())
}

/// The same, in a given offset, so a test does not depend on the machine it
/// runs on.
#[must_use]
pub fn stamp_in(at: OffsetDateTime, offset: UtcOffset) -> String {
    let local = at.to_offset(offset);
    format!(
        "{:02}:{:02}:{:02}",
        local.hour(),
        local.minute(),
        local.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// One instant, read on two different walls: the account is stamped in
    /// the reader's own time, not in UTC.
    #[test]
    fn a_moment_reads_as_the_local_clock() {
        let at = OffsetDateTime::from_unix_timestamp(1_755_555_555).expect("a real instant");

        assert_eq!(stamp_in(at, UtcOffset::UTC), "22:19:15");
        // Two hours east, which is the next day where this instant falls.
        assert_eq!(
            stamp_in(at, UtcOffset::from_hms(2, 0, 0).expect("a real offset")),
            "00:19:15"
        );
        assert_eq!(
            stamp_in(at, UtcOffset::from_hms(-5, 0, 0).expect("a real offset")),
            "17:19:15"
        );
    }

    /// A session that runs all day must not grow without end, and what a
    /// reader reads is the end of the account.
    #[test]
    fn the_oldest_lines_are_dropped_once_the_account_is_full() {
        let mut journal = Journal::default();
        for index in 0..(LIMIT + 10) {
            journal.record(Level::Note, format!("line {index}"));
        }

        assert_eq!(journal.len(), LIMIT);
        assert_eq!(
            journal.last().map(|entry| entry.text.as_str()),
            Some(format!("line {}", LIMIT + 9).as_str())
        );
        assert_eq!(
            journal.entries().first().map(|entry| entry.text.as_str()),
            Some("line 10")
        );
    }

    /// What is copied has to be what was read.
    #[test]
    fn the_copied_account_carries_the_stamps_it_showed() {
        let at = OffsetDateTime::from_unix_timestamp(1_755_555_555).expect("a real instant");
        let mut journal = Journal::default();
        journal.record_at(at, Level::Note, "opened");
        journal.record_at(
            at + Duration::from_secs(62),
            Level::Failure,
            "could not read it",
        );

        let stamps: Vec<String> = journal
            .entries()
            .iter()
            .map(|entry| stamp(entry.at))
            .collect();
        assert_eq!(
            journal.as_text(),
            format!("{}  opened\n{}  could not read it", stamps[0], stamps[1])
        );
    }

    /// Every line of an account is drawn in the same offset, whatever this
    /// host answered — including a host that would not answer at all, whose
    /// account is kept in UTC rather than in an invented offset.
    #[test]
    fn one_offset_stamps_the_whole_account() {
        assert_eq!(local_offset(), local_offset());
    }
}
