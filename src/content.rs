//! Glue that turns a Config into (CalendarItem, RefText) for the viewer:
//! cache → network → fallback.

use anyhow::Result;
use tracing::{info, warn};

use crate::cache;
use crate::config::Config;
use crate::fallback;
use crate::sefaria::{select, CalendarItem, RefText, SefariaClient};
use crate::state::State;

pub struct ResolvedItem {
    pub item: CalendarItem,
    pub text: RefText,
}

pub fn resolve_for_invocation(cfg: &Config, count: u64) -> Result<ResolvedItem> {
    let items = load_calendar(cfg)?;
    let item = match select::pick(&items, cfg, count) {
        Some(i) => i.clone(),
        None => {
            warn!("no calendar items at all; using fallback");
            return Ok(ResolvedItem {
                item: fallback::item()?,
                text: fallback::text()?,
            });
        }
    };
    let text = load_text(&item.ref_)?;
    Ok(ResolvedItem { item, text })
}

pub fn next_for(state: &mut State, cfg: &Config) -> Option<(CalendarItem, RefText)> {
    state.bump();
    let _ = state.save();
    let items = load_calendar(cfg).ok()?;
    let item = select::pick(&items, cfg, state.invocation_count)?.clone();
    let text = load_text(&item.ref_).ok()?;
    Some((item, text))
}

fn load_calendar(cfg: &Config) -> Result<Vec<CalendarItem>> {
    let _ = cfg;
    if let Some(items) = cache::load_calendar_today()? {
        info!("calendar cache hit (today)");
        if !items.is_empty() {
            return Ok(items);
        }
    }
    let client = SefariaClient::default();
    let items = match client.fetch_calendar() {
        Ok(items) => {
            let _ = cache::save_calendar_today(&items);
            let _ = cache::prune_old_calendars(7);
            items
        }
        Err(e) => {
            warn!(error = %e, "calendar fetch failed; trying recent cache");
            cache::load_any_recent_calendar()?.unwrap_or_default()
        }
    };
    if items.is_empty() {
        warn!("no calendar items available; returning fallback-only list");
        return Ok(vec![fallback::item()?]);
    }
    Ok(items)
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

    #[test]
    fn fallback_text_loads() {
        let t = fallback::text().unwrap();
        assert!(!t.he.is_empty());
        assert!(!t.en.is_empty());
    }

    #[test]
    fn fallback_item_loads() {
        let i = fallback::item().unwrap();
        assert_eq!(i.ref_, "Pirkei Avot 1:1");
    }
}
