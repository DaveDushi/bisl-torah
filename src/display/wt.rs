use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Spawn the popup as a Windows Terminal split-pane.
///
///   wt -w 0 split-pane -V -s 0.4 <exe> <args...>
///
/// `-w 0` targets the most recent WT window. `-V` splits vertically (panes side by side).
/// `-s 0.4` allocates 40% of the parent pane to the new pane.
pub fn spawn(exe: &str, args: &[String]) -> Result<()> {
    let mut cmd = Command::new("wt.exe");
    cmd.args(["-w", "0", "split-pane", "-V", "-s", "0.4", exe]);
    for a in args {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().context("spawning wt.exe")?;
    Ok(())
}
