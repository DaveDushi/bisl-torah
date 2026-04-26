use std::path::Path;

use anyhow::Result;
use tracing::info;

use crate::config::DisplayMode;

pub mod new_console;
pub mod tmux;
pub mod wt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    WtSplit,
    TmuxPopup,
    NewConsole,
}

pub fn detect(mode: DisplayMode) -> Strategy {
    match mode {
        DisplayMode::WtSplit => Strategy::WtSplit,
        DisplayMode::TmuxPopup => Strategy::TmuxPopup,
        DisplayMode::NewConsole => Strategy::NewConsole,
        DisplayMode::Auto => auto_detect(),
    }
}

fn auto_detect() -> Strategy {
    if std::env::var("WT_SESSION").is_ok() {
        return Strategy::WtSplit;
    }
    if std::env::var("TMUX").is_ok() {
        return Strategy::TmuxPopup;
    }
    Strategy::NewConsole
}

pub fn spawn(strategy: Strategy, session_id: &str, signal_dir: &Path) -> Result<()> {
    info!(strategy = ?strategy, session = %session_id, "spawning popup");
    let exe = current_exe_string()?;
    let signal_dir_str = signal_dir.to_string_lossy().to_string();
    let show_args = vec![
        "show".to_string(),
        "--session".to_string(),
        session_id.to_string(),
        "--signal-dir".to_string(),
        signal_dir_str,
    ];
    match strategy {
        Strategy::WtSplit => wt::spawn(&exe, &show_args),
        Strategy::TmuxPopup => tmux::spawn(&exe, &show_args),
        Strategy::NewConsole => new_console::spawn(&exe, &show_args),
    }
}

fn current_exe_string() -> Result<String> {
    let path = std::env::current_exe()?;
    Ok(path.to_string_lossy().to_string())
}
