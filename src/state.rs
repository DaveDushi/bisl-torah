use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct State {
    pub invocation_count: u64,
}

impl State {
    pub fn load() -> Result<Self> {
        let path = paths::state_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("reading state {}", path.display()))?;
        let s: State = serde_json::from_str(&contents).unwrap_or_default();
        Ok(s)
    }

    pub fn save(&self) -> Result<()> {
        paths::ensure_dirs()?;
        let path = paths::state_path()?;
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents).with_context(|| format!("writing state {}", path.display()))?;
        Ok(())
    }

    pub fn bump(&mut self) {
        self.invocation_count = self.invocation_count.wrapping_add(1);
    }
}
