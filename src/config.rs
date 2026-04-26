use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub categories: Vec<String>,
    pub layout: Layout,
    pub default_lang: Lang,
    pub nikud: bool,
    pub display: DisplayMode,
    pub popup: Popup,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Popup {
    pub wt_width: String,
    pub tmux_width: String,
    pub tmux_height: String,
}

impl Default for Popup {
    fn default() -> Self {
        Self {
            wt_width: "40%".into(),
            tmux_width: "80%".into(),
            tmux_height: "70%".into(),
        }
    }
}

pub const POPUP_MIN_PCT: u32 = 10;
pub const POPUP_MAX_PCT: u32 = 90;

/// Parse a percentage value like "40%", "40", or " 40 % " into a fraction (0.40).
/// Returns `None` on parse failure or out-of-range [POPUP_MIN_PCT, POPUP_MAX_PCT].
pub fn parse_percent(s: &str) -> Option<f32> {
    let trimmed = s.trim().trim_end_matches('%').trim();
    let n: f32 = trimmed.parse().ok()?;
    if n < POPUP_MIN_PCT as f32 || n > POPUP_MAX_PCT as f32 {
        return None;
    }
    Some(n / 100.0)
}

/// Parse a percentage and fall back to default with a warning if invalid.
fn percent_or_default(field: &str, value: &str, default: &str) -> f32 {
    if let Some(v) = parse_percent(value) {
        return v;
    }
    warn!(
        "popup.{} invalid value {:?}, falling back to default {:?}",
        field, value, default
    );
    parse_percent(default).expect("default percent must be valid")
}

impl Popup {
    /// Resolved WT split fraction (0.10..=0.90).
    pub fn wt_width_fraction(&self) -> f32 {
        percent_or_default("wt_width", &self.wt_width, "40%")
    }

    /// Resolved tmux popup width as a clean "NN%" string.
    pub fn tmux_width_pct(&self) -> String {
        let f = percent_or_default("tmux_width", &self.tmux_width, "80%");
        format!("{}%", (f * 100.0).round() as u32)
    }

    /// Resolved tmux popup height as a clean "NN%" string.
    pub fn tmux_height_pct(&self) -> String {
        let f = percent_or_default("tmux_height", &self.tmux_height, "70%");
        format!("{}%", (f * 100.0).round() as u32)
    }
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
            popup: Popup::default(),
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
        assert_eq!(c.popup, Popup::default());
        assert_eq!(c.popup.wt_width, "40%");
        assert_eq!(c.popup.tmux_width, "80%");
        assert_eq!(c.popup.tmux_height, "70%");
    }

    #[test]
    fn parse_percent_accepts_with_and_without_sign() {
        assert!((parse_percent("40%").unwrap() - 0.40).abs() < 1e-6);
        assert!((parse_percent("40").unwrap() - 0.40).abs() < 1e-6);
        assert!((parse_percent(" 65 % ").unwrap() - 0.65).abs() < 1e-6);
    }

    #[test]
    fn parse_percent_rejects_garbage() {
        assert!(parse_percent("abc").is_none());
        assert!(parse_percent("").is_none());
        assert!(parse_percent("%").is_none());
    }

    #[test]
    fn parse_percent_clamps_out_of_range() {
        assert!(parse_percent("0%").is_none());
        assert!(parse_percent("5%").is_none());
        assert!(parse_percent("150%").is_none());
        assert!(parse_percent("-10%").is_none());
    }

    #[test]
    fn popup_resolved_falls_back_on_invalid() {
        let p = Popup {
            wt_width: "150%".into(),
            tmux_width: "garbage".into(),
            tmux_height: "70%".into(),
        };
        assert!((p.wt_width_fraction() - 0.40).abs() < 1e-6);
        assert_eq!(p.tmux_width_pct(), "80%");
        assert_eq!(p.tmux_height_pct(), "70%");
    }

    #[test]
    fn popup_round_trips_through_toml() {
        let p = Popup {
            wt_width: "50%".into(),
            tmux_width: "75%".into(),
            tmux_height: "65%".into(),
        };
        let toml = toml::to_string_pretty(&p).unwrap();
        let parsed: Popup = toml::from_str(&toml).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn config_with_popup_section_parses() {
        let toml = r#"
            categories = ["Halakhah"]
            layout = "auto"
            default_lang = "both"
            nikud = true
            display = "auto"

            [popup]
            wt_width = "55%"
            tmux_width = "85%"
            tmux_height = "75%"
        "#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(c.popup.wt_width, "55%");
        assert_eq!(c.popup.tmux_width, "85%");
        assert_eq!(c.popup.tmux_height, "75%");
    }

    #[test]
    fn config_without_popup_section_uses_defaults() {
        let toml = r#"
            categories = ["Halakhah"]
        "#;
        let c: Config = toml::from_str(toml).unwrap();
        assert_eq!(c.popup, Popup::default());
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
            popup: Popup {
                wt_width: "55%".into(),
                tmux_width: "85%".into(),
                tmux_height: "65%".into(),
            },
        };
        let serialized = toml::to_string_pretty(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.categories, original.categories);
        assert_eq!(parsed.layout, original.layout);
        assert_eq!(parsed.default_lang, original.default_lang);
        assert_eq!(parsed.nikud, original.nikud);
        assert_eq!(parsed.display, original.display);
        assert_eq!(parsed.popup, original.popup);
    }
}
