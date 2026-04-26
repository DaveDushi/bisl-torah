//! Safe-merge bitul-torah hooks into Claude Code's settings.json.

use std::fs;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project,
}

impl Scope {
    pub fn settings_path(self) -> Result<PathBuf> {
        match self {
            Scope::Global => global_settings_path(),
            Scope::Project => Ok(std::env::current_dir()?
                .join(".claude")
                .join("settings.json")),
        }
    }
}

fn global_settings_path() -> Result<PathBuf> {
    let home = directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("could not resolve home directory"))?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".claude").join("settings.json"))
}

const MARKER: &str = "bitul-torah";

pub fn install(scope: Scope) -> Result<InstallReport> {
    let path = scope.settings_path()?;
    info!(path = %path.display(), "installing hooks");

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating settings dir {}", parent.display()))?;
    }

    let mut root: Value = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        if raw.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing existing {}", path.display()))?
        }
    } else {
        json!({})
    };

    if !root.is_object() {
        return Err(anyhow!("settings.json root is not a JSON object"));
    }

    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks".to_string())
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        return Err(anyhow!("settings.json `hooks` is not an object"));
    }

    let report_prompt = ensure_event(hooks, "UserPromptSubmit", "hook-on-prompt")?;
    let report_stop = ensure_event(hooks, "Stop", "hook-on-stop")?;

    let pretty = serde_json::to_string_pretty(&root)?;
    fs::write(&path, pretty).with_context(|| format!("writing {}", path.display()))?;

    Ok(InstallReport {
        path,
        prompt: report_prompt,
        stop: report_stop,
    })
}

fn ensure_event(hooks: &mut Value, event: &str, sub: &str) -> Result<EventOutcome> {
    let arr = hooks
        .as_object_mut()
        .unwrap()
        .entry(event.to_string())
        .or_insert_with(|| json!([]));
    let arr = arr
        .as_array_mut()
        .ok_or_else(|| anyhow!("hooks.{} is not an array", event))?;

    // Already present?
    for entry in arr.iter() {
        if entry_managed_by_us(entry) {
            return Ok(EventOutcome::AlreadyPresent);
        }
    }

    arr.push(json!({
        "hooks": [
            {
                "type": "command",
                "command": format!("bitul-torah {}", sub),
                "_managed_by": MARKER,
            }
        ]
    }));
    Ok(EventOutcome::Added)
}

fn entry_managed_by_us(entry: &Value) -> bool {
    let Some(arr) = entry.get("hooks").and_then(|h| h.as_array()) else {
        return false;
    };
    arr.iter().any(|h| {
        h.get("_managed_by")
            .and_then(|v| v.as_str())
            .map(|s| s == MARKER)
            .unwrap_or(false)
    })
}

pub fn uninstall(scope: Scope) -> Result<UninstallReport> {
    let path = scope.settings_path()?;
    info!(path = %path.display(), "uninstalling hooks");

    if !path.exists() {
        return Ok(UninstallReport {
            path,
            removed_prompt: false,
            removed_stop: false,
        });
    }

    let raw = fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(UninstallReport {
            path,
            removed_prompt: false,
            removed_stop: false,
        });
    }

    let mut root: Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

    let removed_prompt = remove_event(&mut root, "UserPromptSubmit");
    let removed_stop = remove_event(&mut root, "Stop");

    let pretty = serde_json::to_string_pretty(&root)?;
    fs::write(&path, pretty)?;

    Ok(UninstallReport {
        path,
        removed_prompt,
        removed_stop,
    })
}

fn remove_event(root: &mut Value, event: &str) -> bool {
    let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return false;
    };
    let Some(arr) = hooks.get_mut(event).and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let before = arr.len();
    arr.retain(|entry| !entry_managed_by_us(entry));
    let after = arr.len();
    if arr.is_empty() {
        hooks.remove(event);
    }
    before != after
}

pub fn detect_status(scope: Scope) -> Result<HookStatus> {
    let path = scope.settings_path()?;
    if !path.exists() {
        return Ok(HookStatus {
            path,
            settings_exists: false,
            prompt_hook: false,
            stop_hook: false,
        });
    }
    let raw = fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(HookStatus {
            path,
            settings_exists: true,
            prompt_hook: false,
            stop_hook: false,
        });
    }
    let root: Value = serde_json::from_str(&raw)?;
    let prompt_hook = event_managed(&root, "UserPromptSubmit");
    let stop_hook = event_managed(&root, "Stop");
    Ok(HookStatus {
        path,
        settings_exists: true,
        prompt_hook,
        stop_hook,
    })
}

fn event_managed(root: &Value, event: &str) -> bool {
    let Some(arr) = root
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|v| v.as_array())
    else {
        return false;
    };
    arr.iter().any(entry_managed_by_us)
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub path: PathBuf,
    pub prompt: EventOutcome,
    pub stop: EventOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    Added,
    AlreadyPresent,
}

#[derive(Debug, Clone)]
pub struct UninstallReport {
    pub path: PathBuf,
    pub removed_prompt: bool,
    pub removed_stop: bool,
}

#[derive(Debug, Clone)]
pub struct HookStatus {
    pub path: PathBuf,
    pub settings_exists: bool,
    pub prompt_hook: bool,
    pub stop_hook: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, value: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn install_into_empty_creates_both_events() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        // Build a fake "Scope" by writing to known path then calling install_at.
        let mut root = json!({});
        let hooks = root
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert(json!({}));
        let p = ensure_event(hooks, "UserPromptSubmit", "hook-on-prompt").unwrap();
        let s = ensure_event(hooks, "Stop", "hook-on-stop").unwrap();
        write(&path, &root);

        assert_eq!(p, EventOutcome::Added);
        assert_eq!(s, EventOutcome::Added);

        let read_back = read(&path);
        assert!(event_managed(&read_back, "UserPromptSubmit"));
        assert!(event_managed(&read_back, "Stop"));
    }

    #[test]
    fn idempotent_install_doesnt_duplicate() {
        let mut root = json!({"hooks": {}});
        let hooks = root.get_mut("hooks").unwrap();
        ensure_event(hooks, "UserPromptSubmit", "hook-on-prompt").unwrap();
        let second = ensure_event(hooks, "UserPromptSubmit", "hook-on-prompt").unwrap();
        assert_eq!(second, EventOutcome::AlreadyPresent);
        let arr = hooks.get("UserPromptSubmit").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn install_preserves_unrelated_user_hooks() {
        let mut root = json!({
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [{ "type": "command", "command": "user-script.sh" }] }
                ]
            }
        });
        let hooks = root.get_mut("hooks").unwrap();
        ensure_event(hooks, "UserPromptSubmit", "hook-on-prompt").unwrap();
        let arr = hooks.get("UserPromptSubmit").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 2, "user hook preserved alongside ours");
    }

    #[test]
    fn uninstall_removes_only_managed_entries() {
        let mut root = json!({
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [{ "type": "command", "command": "user-script.sh" }] },
                    { "hooks": [{ "type": "command", "command": "bitul-torah hook-on-prompt", "_managed_by": "bitul-torah" }] }
                ]
            }
        });
        assert!(remove_event(&mut root, "UserPromptSubmit"));
        let arr = root
            .get("hooks")
            .unwrap()
            .get("UserPromptSubmit")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert!(!event_managed(&root, "UserPromptSubmit"));
    }

    #[test]
    fn uninstall_returns_false_when_nothing_managed() {
        let mut root = json!({"hooks": {"UserPromptSubmit": [
            { "hooks": [{ "type": "command", "command": "x.sh" }] }
        ]}});
        assert!(!remove_event(&mut root, "UserPromptSubmit"));
    }
}
