//! Hook entry points called by Claude Code.
//!
//! Both subcommands read a single JSON object from stdin shaped like
//! `{ "session_id": "...", ... }` and exit 0. They never block Claude; even on
//! internal failure they swallow the error and log it.

use std::io::Read;

use anyhow::Result;
use serde::Deserialize;
use tracing::{info, warn};

use crate::config::Config;
use crate::display;
use crate::paths;
use crate::signals;

#[derive(Debug, Deserialize)]
struct HookEvent {
    #[serde(default)]
    session_id: Option<String>,
}

fn read_event() -> HookEvent {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return HookEvent { session_id: None };
    }
    serde_json::from_str(&raw).unwrap_or(HookEvent { session_id: None })
}

fn session_id_or_default(ev: &HookEvent) -> String {
    ev.session_id
        .clone()
        .unwrap_or_else(|| "default".to_string())
}

pub fn on_prompt() -> Result<()> {
    let ev = read_event();
    let session_id = session_id_or_default(&ev);
    info!(session = %session_id, "hook-on-prompt fired");

    let cfg = Config::load().unwrap_or_default();
    let signal_dir = paths::sessions_dir()?;
    paths::ensure_dirs()?;

    // Re-entrancy: if a popup is already running for this session, just touch
    // refresh-pending and exit. The running viewer will surface the hint.
    let pid_path = signals::pid_path(&signal_dir, &session_id);
    if let Some(pid) = signals::read_pid(&pid_path) {
        if process_alive(pid) {
            info!(pid, session = %session_id, "popup already running; signaling refresh");
            let _ = signals::touch(&signals::refresh_path(&signal_dir, &session_id));
            return Ok(());
        } else {
            signals::remove_quiet(&pid_path);
        }
    }

    // Clean up stale signal files from a prior session of the same id.
    signals::remove_quiet(&signals::done_path(&signal_dir, &session_id));
    signals::remove_quiet(&signals::refresh_path(&signal_dir, &session_id));

    let strategy = display::detect(cfg.display);
    if let Err(e) = display::spawn(strategy, &cfg.popup, &session_id, &signal_dir) {
        warn!(error = %e, "popup spawn failed");
    }
    Ok(())
}

pub fn on_stop() -> Result<()> {
    let ev = read_event();
    let session_id = session_id_or_default(&ev);
    info!(session = %session_id, "hook-on-stop fired");
    let signal_dir = paths::sessions_dir()?;
    paths::ensure_dirs()?;
    let _ = signals::touch(&signals::done_path(&signal_dir, &session_id));
    Ok(())
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // kill(pid, 0) returns 0 if the process exists.
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(handle) => {
                let mut code: u32 = 0;
                let result = GetExitCodeProcess(handle, &mut code as *mut u32);
                let _ = CloseHandle(handle);
                result.is_ok() && code == STILL_ACTIVE
            }
            Err(_) => false,
        }
    }
}
