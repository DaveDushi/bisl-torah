use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use sha1::{Digest, Sha1};

use crate::paths;
use crate::sefaria::{CalendarItem, RefText};

pub fn load_calendar_for_key(key: &str) -> Result<Option<Vec<CalendarItem>>> {
    let path = calendar_path(key)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let items: Vec<CalendarItem> = serde_json::from_str(&raw)?;
    Ok(Some(items))
}

pub fn save_calendar_for_key(key: &str, items: &[CalendarItem]) -> Result<()> {
    paths::ensure_dirs()?;
    let path = calendar_path(key)?;
    let json = serde_json::to_string_pretty(items)?;
    fs::write(&path, json).with_context(|| format!("writing calendar cache {}", path.display()))?;
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
