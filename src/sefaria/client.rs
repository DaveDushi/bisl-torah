use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RefText {
    #[serde(rename = "ref", default)]
    pub ref_: String,
    #[serde(default)]
    pub he: Vec<String>,
    #[serde(default)]
    pub en: Vec<String>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub prev: Option<String>,
}

/// Sefaria's `/api/v3/texts/{ref}` returns `versions`, `ref`, `next`, `prev`, etc.
#[derive(Debug, Deserialize)]
struct V3TextResponse {
    #[serde(default)]
    versions: Vec<V3Version>,
    #[serde(default, rename = "ref")]
    ref_: Option<String>,
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    prev: Option<String>,
}

#[derive(Debug, Deserialize)]
struct V3Version {
    #[serde(default)]
    language: String,
    #[serde(default)]
    text: serde_json::Value,
}

/// Node in the Sefaria library index tree.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexNode {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, rename = "heCategory")]
    pub he_category: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "heTitle")]
    pub he_title: Option<String>,
    #[serde(default)]
    pub contents: Vec<IndexNode>,
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
        Ok(body
            .calendar_items
            .into_iter()
            .filter(|i| !i.ref_.trim().is_empty())
            .collect())
    }

    pub fn fetch_calendar_for_date(&self, date: NaiveDate) -> Result<Vec<CalendarItem>> {
        let url = format!(
            "{}/api/calendars?year={}&month={}&day={}&diaspora=1",
            self.base,
            date.format("%Y"),
            date.format("%-m"),
            date.format("%-d"),
        );
        let resp = self.http.get(&url).send()?.error_for_status()?;
        let body: CalendarResponse = resp.json().context("parsing calendar JSON")?;
        Ok(body
            .calendar_items
            .into_iter()
            .filter(|i| !i.ref_.trim().is_empty())
            .collect())
    }

    pub fn fetch_text(&self, ref_: &str) -> Result<RefText> {
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
            next: body.next,
            prev: body.prev,
        })
    }

    pub fn fetch_index_tree(&self) -> Result<Vec<IndexNode>> {
        let url = format!("{}/api/index", self.base);
        let resp = self.http.get(&url).send()?.error_for_status()?;
        let body: Vec<IndexNode> = resp.json().context("parsing index JSON")?;
        Ok(body)
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

    #[test]
    fn ref_text_parses_next_prev() {
        let raw = r#"{
            "ref": "Mishnah Berakhot 1:2",
            "he": ["א"],
            "en": ["A"],
            "next": "Mishnah Berakhot 1:3",
            "prev": "Mishnah Berakhot 1:1"
        }"#;
        let t: RefText = serde_json::from_str(raw).unwrap();
        assert_eq!(t.next.as_deref(), Some("Mishnah Berakhot 1:3"));
        assert_eq!(t.prev.as_deref(), Some("Mishnah Berakhot 1:1"));
    }

    #[test]
    fn ref_text_without_next_prev_is_none() {
        let raw = r#"{
            "ref": "Mishnah Berakhot 1:2",
            "he": ["א"],
            "en": ["A"]
        }"#;
        let t: RefText = serde_json::from_str(raw).unwrap();
        assert_eq!(t.next, None);
        assert_eq!(t.prev, None);
    }
}
