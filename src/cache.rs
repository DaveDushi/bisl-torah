use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Local;
use sha1::{Digest, Sha1};

use crate::paths;
use crate::sefaria::{CalendarItem, RefText};

pub fn today_key() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn load_calendar_today() -> Result<Option<Vec<CalendarItem>>> {
    let path = calendar_path(&today_key())?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let items: Vec<CalendarItem> = serde_json::from_str(&raw)?;
    Ok(Some(items))
}

pub fn save_calendar_today(items: &[CalendarItem]) -> Result<()> {
    paths::ensure_dirs()?;
    let path = calendar_path(&today_key())?;
    let json = serde_json::to_string_pretty(items)?;
    fs::write(&path, json).with_context(|| format!("writing calendar cache {}", path.display()))?;
    Ok(())
}

/// Returns yesterday's (or older) calendar if any exists; used for offline fallback.
pub fn load_any_recent_calendar() -> Result<Option<Vec<CalendarItem>>> {
    let dir = paths::calendar_cache_dir()?;
    if !dir.exists() {
        return Ok(None);
    }
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();
    entries.sort_by_key(|(_, m)| std::cmp::Reverse(*m));
    for (path, _) in entries {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(items) = serde_json::from_str::<Vec<CalendarItem>>(&raw) {
                return Ok(Some(items));
            }
        }
    }
    Ok(None)
}

pub fn prune_old_calendars(keep_days: u64) -> Result<()> {
    let dir = paths::calendar_cache_dir()?;
    if !dir.exists() {
        return Ok(());
    }
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(keep_days * 86400))
        .unwrap_or(std::time::UNIX_EPOCH);
    for entry in fs::read_dir(&dir)?.flatten() {
        if let Ok(metadata) = entry.metadata() {
            if let Ok(modified) = metadata.modified() {
                if modified < cutoff {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
    Ok(())
}

pub fn load_text(ref_: &str) -> Result<Option<RefText>> {
    let path = text_path(ref_)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let text: RefText = serde_json::from_str(&raw)?;
    Ok(Some(text))
}

pub fn save_text(text: &RefText) -> Result<()> {
    paths::ensure_dirs()?;
    let path = text_path(&text.ref_)?;
    let json = serde_json::to_string_pretty(text)?;
    fs::write(&path, json).with_context(|| format!("writing text cache {}", path.display()))?;
    Ok(())
}

fn calendar_path(date_key: &str) -> Result<PathBuf> {
    Ok(paths::calendar_cache_dir()?.join(format!("{}.json", date_key)))
}

fn text_path(ref_: &str) -> Result<PathBuf> {
    let mut hasher = Sha1::new();
    hasher.update(ref_.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    Ok(paths::texts_cache_dir()?.join(format!("{}.json", hex)))
}
