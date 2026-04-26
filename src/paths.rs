use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", "bisl-torah").context("could not resolve project directories")
}

pub fn data_dir() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    Ok(dirs.data_dir().to_path_buf())
}

pub fn config_path() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn cache_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("cache"))
}

pub fn calendar_cache_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("calendar"))
}

pub fn texts_cache_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("texts"))
}

pub fn sessions_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("sessions"))
}

pub fn state_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("state.json"))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("logs"))
}

pub fn ensure_dirs() -> Result<()> {
    for dir in [
        data_dir()?,
        cache_dir()?,
        calendar_cache_dir()?,
        texts_cache_dir()?,
        sessions_dir()?,
        logs_dir()?,
    ] {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating directory {}", dir.display()))?;
    }
    if let Some(parent) = config_path()?.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating config dir {}", parent.display()))?;
    }
    Ok(())
}
