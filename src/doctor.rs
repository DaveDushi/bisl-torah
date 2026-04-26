use std::io::Read;

use anyhow::Result;
use tracing::error;

use crate::install::{detect_status, Scope};
use crate::paths;
use crate::sefaria::SefariaClient;

pub struct Report {
    pub binary_path: String,
    pub binary_on_path: bool,
    pub global_hooks: HookCheck,
    pub project_hooks: HookCheck,
    pub sefaria_reachable: bool,
    pub config_path: String,
    pub config_exists: bool,
    pub log_path: String,
    pub recent_log_lines: Vec<String>,
}

pub struct HookCheck {
    pub path: String,
    pub settings_exists: bool,
    pub prompt_hook: bool,
    pub stop_hook: bool,
}

pub fn run() -> Result<Report> {
    let exe = std::env::current_exe()?;
    let binary_path = exe.to_string_lossy().to_string();
    let binary_on_path = is_on_path("bisl-torah");

    let global = match detect_status(Scope::Global) {
        Ok(s) => HookCheck {
            path: s.path.to_string_lossy().to_string(),
            settings_exists: s.settings_exists,
            prompt_hook: s.prompt_hook,
            stop_hook: s.stop_hook,
        },
        Err(e) => {
            error!(error = %e, "global status check failed");
            HookCheck {
                path: String::new(),
                settings_exists: false,
                prompt_hook: false,
                stop_hook: false,
            }
        }
    };
    let project = match detect_status(Scope::Project) {
        Ok(s) => HookCheck {
            path: s.path.to_string_lossy().to_string(),
            settings_exists: s.settings_exists,
            prompt_hook: s.prompt_hook,
            stop_hook: s.stop_hook,
        },
        Err(_) => HookCheck {
            path: String::new(),
            settings_exists: false,
            prompt_hook: false,
            stop_hook: false,
        },
    };

    let sefaria_reachable = match SefariaClient::default().fetch_calendar() {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %e, "sefaria fetch failed in doctor");
            false
        }
    };

    let config_path = paths::config_path()?.to_string_lossy().to_string();
    let config_exists = std::path::Path::new(&config_path).exists();

    let log_path = paths::logs_dir()?.to_string_lossy().to_string();
    let recent_log_lines = recent_log_lines(50);

    Ok(Report {
        binary_path,
        binary_on_path,
        global_hooks: global,
        project_hooks: project,
        sefaria_reachable,
        config_path,
        config_exists,
        log_path,
        recent_log_lines,
    })
}

fn is_on_path(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        for ext in candidate_exts() {
            let candidate = dir.join(format!("{}{}", prog, ext));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(windows)]
fn candidate_exts() -> &'static [&'static str] {
    &["", ".exe", ".cmd", ".bat"]
}

#[cfg(unix)]
fn candidate_exts() -> &'static [&'static str] {
    &[""]
}

fn recent_log_lines(limit: usize) -> Vec<String> {
    let Ok(dir) = paths::logs_dir() else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime)> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), m))
        })
        .collect();
    entries.sort_by_key(|(_, m)| std::cmp::Reverse(*m));
    let Some((path, _)) = entries.first() else {
        return Vec::new();
    };
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    buf.lines()
        .rev()
        .take(limit)
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn print_report(r: &Report) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    writeln!(out, "bisl-torah doctor").ok();
    writeln!(out, "==================\n").ok();
    line(&mut out, "binary path", &r.binary_path);
    line(
        &mut out,
        "on PATH",
        &if r.binary_on_path {
            "yes".to_string()
        } else {
            "NO".to_string()
        },
    );
    writeln!(out).ok();

    writeln!(out, "config file: {}", r.config_path).ok();
    line(
        &mut out,
        "  exists",
        &if r.config_exists {
            "yes".to_string()
        } else {
            "no (defaults in use)".to_string()
        },
    );
    writeln!(out).ok();

    writeln!(out, "global settings: {}", r.global_hooks.path).ok();
    line(
        &mut out,
        "  file exists",
        &yes_no(r.global_hooks.settings_exists),
    );
    line(
        &mut out,
        "  UserPromptSubmit hook",
        &yes_no(r.global_hooks.prompt_hook),
    );
    line(&mut out, "  Stop hook", &yes_no(r.global_hooks.stop_hook));
    writeln!(out).ok();

    writeln!(out, "project settings: {}", r.project_hooks.path).ok();
    line(
        &mut out,
        "  file exists",
        &yes_no(r.project_hooks.settings_exists),
    );
    line(
        &mut out,
        "  UserPromptSubmit hook",
        &yes_no(r.project_hooks.prompt_hook),
    );
    line(&mut out, "  Stop hook", &yes_no(r.project_hooks.stop_hook));
    writeln!(out).ok();

    writeln!(
        out,
        "Sefaria API: {}",
        if r.sefaria_reachable {
            "reachable"
        } else {
            "UNREACHABLE"
        }
    )
    .ok();
    writeln!(out).ok();

    writeln!(out, "log dir: {}", r.log_path).ok();
    if r.recent_log_lines.is_empty() {
        writeln!(out, "  (no log entries yet)").ok();
    } else {
        writeln!(out, "  recent entries:").ok();
        for line in &r.recent_log_lines {
            writeln!(out, "  | {}", line).ok();
        }
    }
}

fn line<W: std::io::Write>(w: &mut W, k: &str, v: &str) {
    writeln!(w, "  {:<22}: {}", k, v).ok();
}

fn yes_no(b: bool) -> String {
    if b {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}
