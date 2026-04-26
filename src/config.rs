use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub categories: Vec<String>,
    pub layout: Layout,
    pub default_lang: Lang,
    pub nikud: bool,
    pub display: DisplayMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    Auto,
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Both,
    Hebrew,
    English,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayMode {
    Auto,
    WtSplit,
    TmuxPopup,
    NewConsole,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            categories: vec![
                "Halakhah".to_string(),
                "Mishnah".to_string(),
                "Chasidut".to_string(),
            ],
            layout: Layout::Auto,
            default_lang: Lang::Both,
            nikud: true,
            display: DisplayMode::Auto,
        }
    }
}

const STARTER_TOML: &str = include_str!("../assets/config_starter.toml");

impl Config {
    pub fn load() -> Result<Self> {
        let path = paths::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let cfg: Config = toml::from_str(&contents)
            .with_context(|| format!("parsing config {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        let contents = toml::to_string_pretty(self).context("serializing config")?;
        fs::write(&path, contents)
            .with_context(|| format!("writing config {}", path.display()))?;
        Ok(())
    }

    pub fn ensure_starter() -> Result<bool> {
        let path = paths::config_path()?;
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, STARTER_TOML)
            .with_context(|| format!("writing starter config {}", path.display()))?;
        Ok(true)
    }

    pub fn category_matches(&self, item_category: &str) -> bool {
        if self.categories.iter().any(|c| c == "*") {
            return true;
        }
        self.categories
            .iter()
            .any(|c| c.eq_ignore_ascii_case(item_category))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_recommendation() {
        let c = Config::default();
        assert_eq!(c.categories, vec!["Halakhah", "Mishnah", "Chasidut"]);
        assert_eq!(c.layout, Layout::Auto);
        assert_eq!(c.default_lang, Lang::Both);
        assert!(c.nikud);
        assert_eq!(c.display, DisplayMode::Auto);
    }

    #[test]
    fn parses_full_config() {
        let toml = r#"
            categories = ["Halakhah", "Tanakh"]
            layout = "side-by-side"
            default_lang = "english"
            nikud = false
            display = "wt-split"
        "#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(c.categories, vec!["Halakhah", "Tanakh"]);
        assert_eq!(c.layout, Layout::SideBySide);
        assert_eq!(c.default_lang, Lang::English);
        assert!(!c.nikud);
        assert_eq!(c.display, DisplayMode::WtSplit);
    }

    #[test]
    fn category_matching_handles_wildcard() {
        let c = Config {
            categories: vec!["*".into()],
            ..Default::default()
        };
        assert!(c.category_matches("Whatever"));
    }

    #[test]
    fn category_matching_is_case_insensitive() {
        let c = Config::default();
        assert!(c.category_matches("halakhah"));
        assert!(!c.category_matches("Tanakh"));
    }

    #[test]
    fn starter_toml_parses() {
        let _: Config = toml::from_str(STARTER_TOML).unwrap();
    }

    #[test]
    fn round_trips_through_toml() {
        let original = Config {
            categories: vec!["Halakhah".into(), "Tanakh".into()],
            layout: Layout::SideBySide,
            default_lang: Lang::English,
            nikud: false,
            display: DisplayMode::WtSplit,
        };
        let serialized = toml::to_string_pretty(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.categories, original.categories);
        assert_eq!(parsed.layout, original.layout);
        assert_eq!(parsed.default_lang, original.default_lang);
        assert_eq!(parsed.nikud, original.nikud);
        assert_eq!(parsed.display, original.display);
    }
}
