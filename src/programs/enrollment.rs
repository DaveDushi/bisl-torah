use std::collections::BTreeSet;

use anyhow::{anyhow, Result};
use chrono::{Local, NaiveDate, Utc};
use tracing::warn;

use crate::cache;
use crate::programs::catalog::{Catalog, CatalogEntry, CatalogKind};
use crate::sefaria::{CalendarItem, SefariaClient};
use crate::state::{Enrollment, EnrollmentKind, State};

/// Where to start a freshly enrolled cycle subscription.
#[derive(Debug, Clone, Copy)]
pub enum CycleStart {
    Today,
    /// Catch up from this past date forward.
    From(NaiveDate),
}

pub fn enroll_cycle(
    state: &mut State,
    catalog: &Catalog,
    program_id: &str,
    start: CycleStart,
) -> Result<()> {
    let entry = catalog
        .find(program_id)
        .ok_or_else(|| anyhow!("unknown program {}", program_id))?;
    require_kind(entry, CatalogKind::Cycle)?;
    let calendar_title = entry
        .calendar_title
        .clone()
        .ok_or_else(|| anyhow!("cycle program {} missing calendar_title", program_id))?;

    if state.enrollment(program_id).is_some() {
        return Err(anyhow!("already enrolled in {}", program_id));
    }

    let pointer_date = match start {
        CycleStart::Today => Local::now().date_naive(),
        CycleStart::From(d) => d,
    };

    let current_ref = lookup_calendar_ref_for_date(&calendar_title, pointer_date)?;

    state.enrollments.push(Enrollment {
        program_id: program_id.to_string(),
        display_name: entry.display_name.clone(),
        kind: EnrollmentKind::CycleSubscription {
            calendar_title,
            pointer_date,
        },
        current_ref,
        completed: BTreeSet::new(),
        last_completed_ref: None,
        unit: entry.unit,
        started_at: Utc::now(),
    });
    state.save()?;
    Ok(())
}

/// Options applied at book-enrollment time. `Default` means use the catalog's
/// `root_ref` and `unit` verbatim.
#[derive(Debug, Default, Clone)]
pub struct BookOptions {
    /// Override the starting ref. None = use catalog `root_ref`.
    pub starting_ref: Option<String>,
    /// Override the granularity. None = use catalog `unit`.
    pub unit_override: Option<crate::state::Unit>,
}

pub fn enroll_book_with_options(
    state: &mut State,
    catalog: &Catalog,
    program_id: &str,
    opts: BookOptions,
) -> Result<()> {
    let entry = catalog
        .find(program_id)
        .ok_or_else(|| anyhow!("unknown program {}", program_id))?;
    require_kind(entry, CatalogKind::Book)?;
    let catalog_root = entry
        .root_ref
        .clone()
        .ok_or_else(|| anyhow!("book program {} missing root_ref", program_id))?;

    if state.enrollment(program_id).is_some() {
        return Err(anyhow!("already enrolled in {}", program_id));
    }

    let unit = opts.unit_override.unwrap_or(entry.unit);
    let starting_ref = match opts.starting_ref {
        Some(r) => r,
        None => derive_root_for_unit(&catalog_root, entry.unit, unit),
    };

    state.enrollments.push(Enrollment {
        program_id: program_id.to_string(),
        display_name: entry.display_name.clone(),
        kind: EnrollmentKind::BookTrack {
            root_ref: starting_ref.clone(),
        },
        current_ref: starting_ref,
        completed: BTreeSet::new(),
        last_completed_ref: None,
        unit,
        started_at: Utc::now(),
    });
    state.save()?;
    Ok(())
}

/// Adjust the catalog's root ref to a different granularity. We use simple
/// heuristics on the trailing ":N" segment specifier; Sefaria treats refs of
/// different specificity as different depths automatically.
///
/// - Catalog at segment depth ("Mishnah Berurah 1:1"), user wants section: drop ":1".
/// - Catalog at section depth ("Mishneh Torah ... 1"), user wants segment: append ":1".
/// - Same depth: pass through.
fn derive_root_for_unit(
    catalog_root: &str,
    catalog_unit: crate::state::Unit,
    desired_unit: crate::state::Unit,
) -> String {
    use crate::state::Unit;
    if catalog_unit == desired_unit {
        return catalog_root.to_string();
    }
    match (catalog_unit, desired_unit) {
        (Unit::Segment, Unit::Section) => {
            // Drop the trailing ":N" if present.
            if let Some(idx) = catalog_root.rfind(':') {
                catalog_root[..idx].to_string()
            } else {
                catalog_root.to_string()
            }
        }
        (Unit::Section, Unit::Segment) => {
            // Append ":1" to step into the first segment of the first section.
            format!("{}:1", catalog_root)
        }
        _ => catalog_root.to_string(),
    }
}

/// Enroll in a custom Sefaria text picked via the library browser. The
/// `program_id` is namespaced as `custom:<title>` so it doesn't clash with
/// the curated catalog. `unit` defaults to Section for custom enrollments.
pub fn enroll_custom_book(
    state: &mut State,
    title: &str,
    starting_ref: &str,
    unit: crate::state::Unit,
) -> Result<()> {
    let program_id = format!("custom:{}", title);
    if state.enrollment(&program_id).is_some() {
        return Err(anyhow!("already enrolled in {}", program_id));
    }
    state.enrollments.push(Enrollment {
        program_id,
        display_name: title.to_string(),
        kind: EnrollmentKind::BookTrack {
            root_ref: starting_ref.to_string(),
        },
        current_ref: starting_ref.to_string(),
        completed: BTreeSet::new(),
        last_completed_ref: None,
        unit,
        started_at: Utc::now(),
    });
    state.save()?;
    Ok(())
}

fn require_kind(entry: &CatalogEntry, kind: CatalogKind) -> Result<()> {
    if entry.kind != kind {
        return Err(anyhow!(
            "program {} is not a {:?} program",
            entry.id,
            kind
        ));
    }
    Ok(())
}

/// Look up the ref for a cycle program on a specific date, hitting cache then network.
pub fn lookup_calendar_ref_for_date(
    calendar_title: &str,
    date: NaiveDate,
) -> Result<String> {
    let items = load_calendar_for_date(date)?;
    items
        .iter()
        .find(|i| i.title.en.eq_ignore_ascii_case(calendar_title))
        .map(|i| i.ref_.clone())
        .ok_or_else(|| {
            anyhow!(
                "calendar entry {} not found for {}",
                calendar_title,
                date
            )
        })
}

pub fn load_calendar_for_date(date: NaiveDate) -> Result<Vec<CalendarItem>> {
    let key = date.format("%Y-%m-%d").to_string();
    if let Some(items) = cache::load_calendar_for_key(&key)? {
        if !items.is_empty() {
            return Ok(items);
        }
    }
    let client = SefariaClient::default();
    match client.fetch_calendar_for_date(date) {
        Ok(items) => {
            let _ = cache::save_calendar_for_key(&key, &items);
            Ok(items)
        }
        Err(e) => {
            warn!(date = %date, error = %e, "calendar-by-date fetch failed");
            Err(e)
        }
    }
}
