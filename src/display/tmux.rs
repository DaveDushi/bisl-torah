use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Spawn the popup as a tmux floating popup.
///
///   tmux display-popup -E -h 70% -w 80% '<exe> <args...>'
///
/// `-E` makes the popup close when the inner command exits. We pass arguments as a
/// single shell-escaped string because tmux invokes the command via `sh -c`.
pub fn spawn(exe: &str, args: &[String]) -> Result<()> {
    let cmdline = build_cmdline(exe, args);
    let mut cmd = Command::new("tmux");
    cmd.args(["display-popup", "-E", "-h", "70%", "-w", "80%", &cmdline])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().context("spawning tmux display-popup")?;
    Ok(())
}

fn build_cmdline(exe: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(shell_escape(exe));
    for a in args {
        parts.push(shell_escape(a));
    }
    parts.join(" ")
}

fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | ':' | '=' | ','));
    if safe {
        s.to_string()
    } else {
        let escaped = s.replace('\'', r"'\''");
        format!("'{}'", escaped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_paths_pass_through() {
        assert_eq!(
            shell_escape("/usr/local/bin/bitul-torah"),
            "/usr/local/bin/bitul-torah"
        );
    }

    #[test]
    fn paths_with_spaces_get_quoted() {
        assert_eq!(shell_escape("a b"), "'a b'");
    }

    #[test]
    fn single_quote_escaped() {
        assert_eq!(shell_escape("a'b"), r"'a'\''b'");
    }

    #[test]
    fn cmdline_combines_args() {
        let cmd = build_cmdline(
            "/bin/bt",
            &["show".into(), "--session".into(), "id 1".into()],
        );
        assert_eq!(cmd, "/bin/bt show --session 'id 1'");
    }
}
