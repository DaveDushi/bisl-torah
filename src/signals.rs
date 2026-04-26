use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn pid_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{}.pid", sanitize(session_id)))
}

pub fn done_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{}.done", sanitize(session_id)))
}

pub fn refresh_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{}.refresh-pending", sanitize(session_id)))
}

pub fn touch(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, b"").with_context(|| format!("touching signal {}", path.display()))?;
    Ok(())
}

pub fn write_pid(path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, pid.to_string().as_bytes())
        .with_context(|| format!("writing pid {}", path.display()))?;
    Ok(())
}

pub fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse::<u32>().ok()
}

pub fn remove_quiet(path: &Path) {
    let _ = fs::remove_file(path);
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_safe_chars() {
        assert_eq!(sanitize("abc-123_x"), "abc-123_x");
    }

    #[test]
    fn sanitize_replaces_unsafe() {
        assert_eq!(sanitize("a/b\\c d"), "a_b_c_d");
    }

    #[test]
    fn paths_are_well_formed() {
        let dir = std::env::temp_dir();
        let pid = pid_path(&dir, "sess-1");
        let done = done_path(&dir, "sess-1");
        let refr = refresh_path(&dir, "sess-1");
        assert!(pid.to_string_lossy().ends_with("sess-1.pid"));
        assert!(done.to_string_lossy().ends_with("sess-1.done"));
        assert!(refr.to_string_lossy().ends_with("sess-1.refresh-pending"));
    }
}
