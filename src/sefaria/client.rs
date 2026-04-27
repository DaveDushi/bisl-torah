use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_BASE: &str = "https://www.sefaria.org";

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitlePair {
    #[serde(default)]
    pub en: String,
    #[serde(default)]
    pub he: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarItem {
    #[serde(default)]
    pub title: TitlePair,
    #[serde(rename = "displayValue", default)]
    pub display_value: TitlePair,
    #[serde(rename = "ref", default)]
    pub ref_: String,
    #[serde(rename = "heRef", default)]
    pub he_ref: Option<String>,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub order: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub description: Option<TitlePair>,
    #[serde(rename = "extraDetails", default)]
    pub extra_details: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CalendarResponse {
    #[serde(rename = "calendar_items", default)]
    pub calendar_items: Vec<CalendarItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefText {
    #[serde(rename = "ref", default)]
    pub ref_: String,
    #[serde(default)]
    pub he: Vec<String>,
    #[serde(default)]
    pub en: Vec<String>,
}

/// Sefaria's `/api/v3/texts/{ref}` returns multiple `versions`. Each version has a `text`
/// field that is either a string, list of strings, or nested list. We flatten to a flat
/// list of segment strings for simplicity.
#[derive(Debug, Deserialize)]
struct V3TextResponse {
    #[serde(default)]
    versions: Vec<V3Version>,
    #[serde(default, rename = "ref")]
    ref_: Option<String>,
}

#[derive(Debug, Deserialize)]
struct V3Version {
    #[serde(default)]
    language: String,
    #[serde(default)]
    text: serde_json::Value,
}

pub struct SefariaClient {
    base: String,
    http: reqwest::blocking::Client,
}

impl Default for SefariaClient {
    fn default() -> Self {
        Self::new(DEFAULT_BASE.to_string())
    }
}

impl SefariaClient {
    pub fn new(base: String) -> Self {
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("bisl-torah/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        Self { base, http }
    }

    pub fn fetch_calendar(&self) -> Result<Vec<CalendarItem>> {
        let url = format!("{}/api/calendars", self.base);
        let resp = self.http.get(&url).send()?.error_for_status()?;
        let body: CalendarResponse = resp.json().context("parsing calendar JSON")?;
        // Drop entries without a ref — they can't be fetched.
        let items = body
            .calendar_items
            .into_iter()
            .filter(|i| !i.ref_.trim().is_empty())
            .collect();
        Ok(items)
    }

    pub fn fetch_text(&self, ref_: &str) -> Result<RefText> {
        // /api/v3/texts/ accepts URL-encoded ref. The response includes versions; we want
        // one English (return_format=text) and one Hebrew (source).
        let encoded = urlencode(ref_);
        let url = format!(
            "{}/api/v3/texts/{}?version=english&version=hebrew&return_format=text_only",
            self.base, encoded
        );
        let resp = self.http.get(&url).send()?.error_for_status()?;
        let body: V3TextResponse = resp.json().context("parsing v3 text JSON")?;
        let mut he = Vec::new();
        let mut en = Vec::new();
        for v in body.versions {
            let lang = v.language.to_lowercase();
            let segments = flatten_text(&v.text);
            match lang.as_str() {
                "he" | "hebrew" if he.is_empty() => he = segments,
                "en" | "english" if en.is_empty() => en = segments,
                _ => {}
            }
        }
        if he.is_empty() && en.is_empty() {
            return Err(anyhow!("no segments returned for ref {}", ref_));
        }
        Ok(RefText {
            ref_: body.ref_.unwrap_or_else(|| ref_.to_string()),
            he,
            en,
        })
    }
}

fn flatten_text(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    flatten_into(value, &mut out);
    out
}

fn flatten_into(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => out.push(s.clone()),
        serde_json::Value::Array(arr) => {
            for v in arr {
                flatten_into(v, out);
            }
        }
        _ => {}
    }
}

fn urlencode(s: &str) -> String {
    // Minimal percent-encoding for ref strings; Sefaria refs are ASCII with spaces, colons, dots.
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_string_arrays_and_nested() {
        let v: serde_json::Value = serde_json::from_str(r#"["a","b",["c",["d"]]]"#).unwrap();
        let out = flatten_text(&v);
        assert_eq!(out, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn flatten_skips_empty_strings() {
        let v: serde_json::Value = serde_json::from_str(r#"["a","",["","b"]]"#).unwrap();
        let out = flatten_text(&v);
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn urlencode_preserves_alphanum_and_encodes_spaces_and_colons() {
        assert_eq!(urlencode("Halacha 1.2"), "Halacha%201.2");
        assert_eq!(urlencode("Mishnah Avot 1:1"), "Mishnah%20Avot%201%3A1");
    }

    #[test]
    fn parses_minimal_calendar_item() {
        let raw = r#"{
            "title": {"en": "Daf Yomi", "he": ""},
            "displayValue": {"en": "Sanhedrin 7", "he": ""},
            "ref": "Sanhedrin 7",
            "category": "Talmud",
            "order": 1
        }"#;
        let item: CalendarItem = serde_json::from_str(raw).unwrap();
        assert_eq!(item.ref_, "Sanhedrin 7");
        assert_eq!(item.category, "Talmud");
        assert_eq!(item.order, Some(1));
    }
}
