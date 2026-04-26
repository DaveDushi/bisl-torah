use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Spawn the popup as a Windows Terminal split-pane.
///
///   wt -w 0 split-pane -V -s <fraction> <exe> <args...>
///
/// `-w 0` targets the most recent WT window. `-V` splits vertically (panes side by side).
/// `-s <fraction>` allocates that fraction of the parent pane to the new pane.
pub fn spawn(exe: &str, args: &[String], width_fraction: f32) -> Result<()> {
    let frac = format!("{:.2}", width_fraction);
    let mut cmd = Command::new("wt.exe");
    cmd.args(["-w", "0", "split-pane", "-V", "-s", &frac, exe]);
    for a in args {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().context("spawning wt.exe")?;
    Ok(())
}
