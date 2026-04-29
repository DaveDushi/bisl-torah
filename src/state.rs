use std::collections::BTreeSet;
use std::fs;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::paths;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub schema_version: u32,
    pub invocation_count: u64,
    pub enrollments: Vec<Enrollment>,
    pub archive: Vec<ArchivedEnrollment>,
    /// Reverse-chronological log of every completion (`n` press), used for the history screen.
    pub completion_log: Vec<CompletionLogEntry>,
    /// Set to true once the v1->v2 migration prompt has been shown (or skipped) at least once.
    pub migration_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionLogEntry {
    pub program_id: String,
    pub program_name: String,
    pub ref_: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Enrollment {
    pub program_id: String,
    pub display_name: String,
    pub kind: EnrollmentKind,
    pub current_ref: String,
    pub completed: BTreeSet<String>,
    pub last_completed_ref: Option<String>,
    pub unit: Unit,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum EnrollmentKind {
    BookTrack {
        root_ref: String,
    },
    CycleSubscription {
        calendar_title: String,
        pointer_date: NaiveDate,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Unit {
    Segment,
    Section,
    Daf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchivedEnrollment {
    pub program_id: String,
    pub display_name: String,
    pub completed_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub item_count: usize,
}

impl State {
    pub fn load() -> Result<Self> {
        let path = paths::state_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("reading state {}", path.display()))?;
        let s: State = serde_json::from_str(&contents).unwrap_or_default();
        Ok(s)
    }

    pub fn save(&self) -> Result<()> {
        paths::ensure_dirs()?;
        let path = paths::state_path()?;
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents).with_context(|| format!("writing state {}", path.display()))?;
        Ok(())
    }

    pub fn bump(&mut self) {
        self.invocation_count = self.invocation_count.wrapping_add(1);
    }

    pub fn needs_migration(&self) -> bool {
        !self.migration_done && self.schema_version < CURRENT_SCHEMA_VERSION
    }

    pub fn enrollment(&self, program_id: &str) -> Option<&Enrollment> {
        self.enrollments.iter().find(|e| e.program_id == program_id)
    }

    pub fn enrollment_mut(&mut self, program_id: &str) -> Option<&mut Enrollment> {
        self.enrollments
            .iter_mut()
            .find(|e| e.program_id == program_id)
    }

    pub fn unenroll(&mut self, program_id: &str) -> Option<Enrollment> {
        let pos = self.enrollments.iter().position(|e| e.program_id == program_id)?;
        Some(self.enrollments.remove(pos))
    }

    pub fn archive_program(&mut self, program_id: &str) {
        if let Some(e) = self.unenroll(program_id) {
            self.archive.push(ArchivedEnrollment {
                program_id: e.program_id,
                display_name: e.display_name,
                completed_at: Utc::now(),
                started_at: e.started_at,
                item_count: e.completed.len(),
            });
        }
    }

    pub fn log_completion(&mut self, program_id: &str, program_name: &str, ref_: &str) {
        // Front-prepended so the vec is naturally reverse-chronological.
        self.completion_log.insert(
            0,
            CompletionLogEntry {
                program_id: program_id.to_string(),
                program_name: program_name.to_string(),
                ref_: ref_.to_string(),
                at: Utc::now(),
            },
        );
        // Cap to keep the file bounded.
        const MAX_LOG: usize = 5000;
        if self.completion_log.len() > MAX_LOG {
            self.completion_log.truncate(MAX_LOG);
        }
    }

    /// Drop the most recent log entry for this program (used by undo).
    pub fn pop_log_for(&mut self, program_id: &str) {
        if let Some(idx) = self
            .completion_log
            .iter()
            .position(|e| e.program_id == program_id)
        {
            self.completion_log.remove(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_state_needs_migration() {
        let s = State::default();
        assert!(s.needs_migration());
    }

    #[test]
    fn state_with_v2_does_not_need_migration() {
        let s = State {
            schema_version: CURRENT_SCHEMA_VERSION,
            migration_done: true,
            ..Default::default()
        };
        assert!(!s.needs_migration());
    }

    #[test]
    fn enrollment_kind_book_round_trips() {
        let k = EnrollmentKind::BookTrack {
            root_ref: "Mishnah Berurah".into(),
        };
        let s = serde_json::to_string(&k).unwrap();
        let parsed: EnrollmentKind = serde_json::from_str(&s).unwrap();
        assert_eq!(k, parsed);
    }

    #[test]
    fn enrollment_kind_cycle_round_trips() {
        let k = EnrollmentKind::CycleSubscription {
            calendar_title: "Daf Yomi".into(),
            pointer_date: NaiveDate::from_ymd_opt(2026, 4, 27).unwrap(),
        };
        let s = serde_json::to_string(&k).unwrap();
        let parsed: EnrollmentKind = serde_json::from_str(&s).unwrap();
        assert_eq!(k, parsed);
    }

    #[test]
    fn legacy_state_with_only_invocation_count_loads() {
        let raw = r#"{ "invocation_count": 42 }"#;
        let s: State = serde_json::from_str(raw).unwrap();
        assert_eq!(s.invocation_count, 42);
        assert_eq!(s.schema_version, 0);
        assert!(s.enrollments.is_empty());
        assert!(s.needs_migration());
    }

    #[test]
    fn unenroll_removes() {
        let mut s = State::default();
        s.enrollments.push(Enrollment {
            program_id: "x".into(),
            display_name: "x".into(),
            kind: EnrollmentKind::BookTrack {
                root_ref: "x".into(),
            },
            current_ref: "x".into(),
            completed: BTreeSet::new(),
            last_completed_ref: None,
            unit: Unit::Segment,
            started_at: Utc::now(),
        });
        assert_eq!(s.enrollments.len(), 1);
        let removed = s.unenroll("x");
        assert!(removed.is_some());
        assert!(s.enrollments.is_empty());
    }

    #[test]
    fn archive_moves_enrollment() {
        let mut s = State::default();
        s.enrollments.push(Enrollment {
            program_id: "x".into(),
            display_name: "X".into(),
            kind: EnrollmentKind::BookTrack {
                root_ref: "x".into(),
            },
            current_ref: "x".into(),
            completed: {
                let mut c = BTreeSet::new();
                c.insert("a".into());
                c.insert("b".into());
                c
            },
            last_completed_ref: None,
            unit: Unit::Segment,
            started_at: Utc::now(),
        });
        s.archive_program("x");
        assert!(s.enrollments.is_empty());
        assert_eq!(s.archive.len(), 1);
        assert_eq!(s.archive[0].item_count, 2);
    }
}
