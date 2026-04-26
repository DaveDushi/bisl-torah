use anyhow::Result;
use serde::Deserialize;

use crate::sefaria::{CalendarItem, RefText, TitlePair};

const FALLBACK_RAW: &str = include_str!("../assets/avot_1_1.json");

#[derive(Deserialize)]
struct FallbackData {
    #[serde(rename = "ref")]
    ref_: String,
    #[serde(rename = "heRef")]
    he_ref: String,
    title: TitlePair,
    #[serde(rename = "displayValue")]
    display_value: TitlePair,
    category: String,
    he: Vec<String>,
    en: Vec<String>,
}

fn data() -> Result<FallbackData> {
    Ok(serde_json::from_str(FALLBACK_RAW)?)
}

pub fn item() -> Result<CalendarItem> {
    let d = data()?;
    Ok(CalendarItem {
        title: d.title,
        display_value: d.display_value,
        ref_: d.ref_,
        he_ref: Some(d.he_ref),
        category: d.category,
        order: Some(0),
        url: None,
        description: None,
        extra_details: None,
    })
}

pub fn text() -> Result<RefText> {
    let d = data()?;
    Ok(RefText {
        ref_: d.ref_,
        he: d.he,
        en: d.en,
    })
}
