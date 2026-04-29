use anyhow::Result;
use serde::Deserialize;

use crate::sefaria::RefText;

const FALLBACK_RAW: &str = include_str!("../assets/avot_1_1.json");

#[derive(Deserialize)]
struct FallbackData {
    #[serde(rename = "ref")]
    ref_: String,
    he: Vec<String>,
    en: Vec<String>,
}

pub fn text() -> Result<RefText> {
    let d: FallbackData = serde_json::from_str(FALLBACK_RAW)?;
    Ok(RefText {
        ref_: d.ref_,
        he: d.he,
        en: d.en,
        next: None,
        prev: None,
    })
}
