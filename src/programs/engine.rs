use anyhow::{anyhow, Result};
use chrono::{Duration, Local};
use tracing::{info, warn};

use crate::cache;
use crate::fallback;
use crate::sefaria::{CalendarItem, RefText, SefariaClient, TitlePair};
use crate::state::{Enrollment, EnrollmentKind, State};

#[derive(Debug, Clone)]
pub struct ResolvedItem {
    pub item: CalendarItem,
    pub text: RefText,
    pub program_id: String,
    pub program_name: String,
    pub completed_count: usize,
    pub status: ItemStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Active,
    /// Cycle program at today's calendar entry — can't advance further.
    CaughtUp,
    /// A book just got its final ref marked complete on this `advance()` call.
    JustCompleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvanceOutcome {
    Advanced,
    CaughtUp,
    Completed,
}

/// Pick the program for a given invocation by rotation through enrolled programs.
pub fn pick_for_invocation(state: &State, count: u64) -> Option<&Enrollment> {
    if state.enrollments.is_empty() {
        return None;
    }
    let idx = (count as usize) % state.enrollments.len();
    state.enrollments.get(idx)
}

/// Index of the enrollment that follows `current_id` in rotation order.
pub fn next_program_index(state: &State, current_id: &str) -> Option<usize> {
    let n = state.enrollments.len();
    if n == 0 {
        return None;
    }
    let cur = state
        .enrollments
        .iter()
        .position(|e| e.program_id == current_id)?;
    Some((cur + 1) % n)
}

/// Resolve a program's current_ref into a fully populated ResolvedItem, going
/// through cache/network/fallback like the original content::resolve_for_invocation.
pub fn resolve(enrollment: &Enrollment) -> Result<ResolvedItem> {
    let text = load_text(&enrollment.current_ref)?;
    let item = build_calendar_item_for(enrollment);
    let status = if is_caught_up(enrollment) {
        ItemStatus::CaughtUp
    } else {
        ItemStatus::Active
    };
    Ok(ResolvedItem {
        item,
        text,
        program_id: enrollment.program_id.clone(),
        program_name: enrollment.display_name.clone(),
        completed_count: enrollment.completed.len(),
        status,
    })
}

/// Advance a program: mark current ref complete, move pointer to next ref.
/// Returns the post-advance ResolvedItem.
pub fn advance(state: &mut State, program_id: &str) -> Result<ResolvedItem> {
    let outcome = advance_pointer(state, program_id)?;
    match outcome {
        AdvanceOutcome::CaughtUp => {
            let enrollment = state
                .enrollment(program_id)
                .ok_or_else(|| anyhow!("enrollment {} disappeared", program_id))?;
            let mut resolved = resolve(enrollment)?;
            resolved.status = ItemStatus::CaughtUp;
            Ok(resolved)
        }
        AdvanceOutcome::Completed => {
            // Build a synthetic resolved item showing the just-completed ref before
            // archiving so the viewer can show a siyum banner once.
            let enrollment = state
                .enrollment(program_id)
                .ok_or_else(|| anyhow!("enrollment {} disappeared", program_id))?
                .clone();
            let mut resolved = resolve(&enrollment)?;
            resolved.status = ItemStatus::JustCompleted;
            state.archive_program(program_id);
            let _ = state.save();
            Ok(resolved)
        }
        AdvanceOutcome::Advanced => {
            let _ = state.save();
            let enrollment = state
                .enrollment(program_id)
                .ok_or_else(|| anyhow!("enrollment {} disappeared", program_id))?;
            resolve(enrollment)
        }
    }
}

fn advance_pointer(state: &mut State, program_id: &str) -> Result<AdvanceOutcome> {
    let (program_name, completed_ref, outcome) = {
        let enrollment = state
            .enrollment_mut(program_id)
            .ok_or_else(|| anyhow!("not enrolled in {}", program_id))?;

        match enrollment.kind.clone() {
            EnrollmentKind::CycleSubscription {
                calendar_title,
                pointer_date,
            } => {
                let today = Local::now().date_naive();
                if pointer_date >= today {
                    return Ok(AdvanceOutcome::CaughtUp);
                }
                let current_ref = enrollment.current_ref.clone();
                enrollment.completed.insert(current_ref.clone());
                enrollment.last_completed_ref = Some(current_ref.clone());
                let next_date = pointer_date + Duration::days(1);
                let next_ref = crate::programs::enrollment::lookup_calendar_ref_for_date(
                    &calendar_title,
                    next_date,
                )
                .map_err(|e| {
                    warn!(error = %e, date = %next_date, "calendar lookup failed");
                    e
                })?;
                enrollment.kind = EnrollmentKind::CycleSubscription {
                    calendar_title,
                    pointer_date: next_date,
                };
                enrollment.current_ref = next_ref;
                (
                    enrollment.display_name.clone(),
                    current_ref,
                    AdvanceOutcome::Advanced,
                )
            }
            EnrollmentKind::BookTrack { .. } => {
                let current_ref = enrollment.current_ref.clone();
                let text = load_text(&current_ref)?;
                enrollment.completed.insert(current_ref.clone());
                enrollment.last_completed_ref = Some(current_ref.clone());
                let outcome = if let Some(next) = text.next.filter(|s| !s.trim().is_empty()) {
                    enrollment.current_ref = next;
                    AdvanceOutcome::Advanced
                } else {
                    AdvanceOutcome::Completed
                };
                (enrollment.display_name.clone(), current_ref, outcome)
            }
        }
    };

    state.log_completion(program_id, &program_name, &completed_ref);
    Ok(outcome)
}

/// Undo the last completion: step back, un-mark, leave the user viewing the un-marked ref.
pub fn undo(state: &mut State, program_id: &str) -> Result<Option<ResolvedItem>> {
    {
        let enrollment = state
            .enrollment_mut(program_id)
            .ok_or_else(|| anyhow!("not enrolled in {}", program_id))?;
        let Some(last) = enrollment.last_completed_ref.clone() else {
            return Ok(None);
        };
        enrollment.completed.remove(&last);
        enrollment.current_ref = last;
        enrollment.last_completed_ref = None;

        if let EnrollmentKind::CycleSubscription {
            calendar_title,
            pointer_date,
        } = enrollment.kind.clone()
        {
            enrollment.kind = EnrollmentKind::CycleSubscription {
                calendar_title,
                pointer_date: pointer_date - Duration::days(1),
            };
        }
    }
    state.pop_log_for(program_id);

    let _ = state.save();
    let enrollment = state
        .enrollment(program_id)
        .ok_or_else(|| anyhow!("enrollment {} disappeared", program_id))?;
    Ok(Some(resolve(enrollment)?))
}

pub fn is_caught_up(enrollment: &Enrollment) -> bool {
    match &enrollment.kind {
        EnrollmentKind::CycleSubscription { pointer_date, .. } => {
            *pointer_date >= Local::now().date_naive()
        }
        EnrollmentKind::BookTrack { .. } => false,
    }
}

fn build_calendar_item_for(enrollment: &Enrollment) -> CalendarItem {
    let category = match &enrollment.kind {
        EnrollmentKind::BookTrack { .. } => "Program".to_string(),
        EnrollmentKind::CycleSubscription { calendar_title, .. } => calendar_title.clone(),
    };
    CalendarItem {
        title: TitlePair {
            en: enrollment.display_name.clone(),
            he: String::new(),
        },
        display_value: TitlePair {
            en: enrollment.current_ref.clone(),
            he: String::new(),
        },
        ref_: enrollment.current_ref.clone(),
        he_ref: None,
        category,
        order: Some(0),
        url: None,
        description: None,
        extra_details: None,
    }
}

fn load_text(ref_: &str) -> Result<RefText> {
    if let Some(t) = cache::load_text(ref_)? {
        info!(ref_ = %ref_, "text cache hit");
        return Ok(t);
    }
    let client = SefariaClient::default();
    match client.fetch_text(ref_) {
        Ok(text) => {
            let _ = cache::save_text(&text);
            Ok(text)
        }
        Err(e) => {
            warn!(ref_ = %ref_, error = %e, "text fetch failed; using fallback");
            fallback::text()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{EnrollmentKind, Unit};
    use chrono::{NaiveDate, Utc};
    use std::collections::BTreeSet;

    fn cycle_enrollment(pointer_date: NaiveDate) -> Enrollment {
        Enrollment {
            program_id: "daf-yomi".into(),
            display_name: "Daf Yomi".into(),
            kind: EnrollmentKind::CycleSubscription {
                calendar_title: "Daf Yomi".into(),
                pointer_date,
            },
            current_ref: "Sanhedrin 7".into(),
            completed: BTreeSet::new(),
            last_completed_ref: None,
            unit: Unit::Daf,
            started_at: Utc::now(),
        }
    }

    fn book_enrollment() -> Enrollment {
        Enrollment {
            program_id: "book:mishnah-berurah".into(),
            display_name: "Mishnah Berurah".into(),
            kind: EnrollmentKind::BookTrack {
                root_ref: "Mishnah Berurah 1:1".into(),
            },
            current_ref: "Mishnah Berurah 1:1".into(),
            completed: BTreeSet::new(),
            last_completed_ref: None,
            unit: Unit::Segment,
            started_at: Utc::now(),
        }
    }

    #[test]
    fn caught_up_detects_today() {
        let today = Local::now().date_naive();
        let e = cycle_enrollment(today);
        assert!(is_caught_up(&e));
        let e = cycle_enrollment(today - Duration::days(1));
        assert!(!is_caught_up(&e));
    }

    #[test]
    fn book_never_caught_up_by_date() {
        let e = book_enrollment();
        assert!(!is_caught_up(&e));
    }

    #[test]
    fn pick_for_invocation_rotates() {
        let mut state = State::default();
        state.enrollments.push(book_enrollment());
        let mut e2 = book_enrollment();
        e2.program_id = "other".into();
        state.enrollments.push(e2);

        let p0 = pick_for_invocation(&state, 0).unwrap();
        let p1 = pick_for_invocation(&state, 1).unwrap();
        let p2 = pick_for_invocation(&state, 2).unwrap();
        assert_eq!(p0.program_id, "book:mishnah-berurah");
        assert_eq!(p1.program_id, "other");
        assert_eq!(p2.program_id, "book:mishnah-berurah");
    }

    #[test]
    fn pick_for_invocation_empty_returns_none() {
        let state = State::default();
        assert!(pick_for_invocation(&state, 0).is_none());
    }

    #[test]
    fn next_program_index_wraps() {
        let mut state = State::default();
        state.enrollments.push(book_enrollment());
        let mut e2 = book_enrollment();
        e2.program_id = "b".into();
        state.enrollments.push(e2);

        assert_eq!(next_program_index(&state, "book:mishnah-berurah"), Some(1));
        assert_eq!(next_program_index(&state, "b"), Some(0));
    }
}
