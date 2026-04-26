use std::process::{Command, Stdio};

#[cfg(unix)]
use anyhow::anyhow;
use anyhow::{Context, Result};
#[cfg(unix)]
use tracing::{debug, warn};

#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

#[cfg(windows)]
pub fn spawn(exe: &str, args: &[String]) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .creation_flags(CREATE_NEW_CONSOLE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().context("spawning detached console window")?;
    Ok(())
}

#[cfg(unix)]
pub fn spawn(exe: &str, args: &[String]) -> Result<()> {
    // Try common terminal emulators in priority order. Each takes the user's
    // command differently; we encode the variants here.
    let candidates: &[(&str, &[&str])] = &[
        ("alacritty", &["-e"]),
        ("kitty", &[]),
        ("wezterm", &["start", "--"]),
        ("foot", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xterm", &["-e"]),
        ("x-terminal-emulator", &["-e"]),
    ];

    for (term, head_args) in candidates {
        if which(term).is_some() {
            debug!(terminal = term, "spawning popup in terminal emulator");
            let mut cmd = Command::new(term);
            for a in head_args.iter() {
                cmd.arg(a);
            }
            cmd.arg(exe);
            for a in args {
                cmd.arg(a);
            }
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            match cmd.spawn() {
                Ok(_) => return Ok(()),
                Err(e) => {
                    warn!(terminal = term, error = %e, "spawn failed, trying next");
                    continue;
                }
            }
        }
    }

    // macOS fallback: `open -a Terminal <script>`. Building a script is fiddly;
    // for now we fall through to error.
    if cfg!(target_os = "macos") {
        if let Some(path) = which("osascript") {
            let cmdline = build_shell_line(exe, args);
            let script = format!(
                r#"tell application "Terminal" to do script "{}""#,
                cmdline.replace('"', "\\\"")
            );
            let mut cmd = Command::new(path);
            cmd.args(["-e", &script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            cmd.spawn().context("spawning Terminal via osascript")?;
            return Ok(());
        }
    }

    Err(anyhow!(
        "no supported terminal emulator found; install one of: alacritty, kitty, wezterm, foot, gnome-terminal, konsole, xterm"
    ))
}

#[cfg(unix)]
fn which(prog: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for entry in std::env::split_paths(&path) {
        let candidate = entry.join(prog);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn build_shell_line(exe: &str, args: &[String]) -> String {
    let mut parts = vec![exe.to_string()];
    parts.extend(args.iter().cloned());
    parts.join(" ")
}
