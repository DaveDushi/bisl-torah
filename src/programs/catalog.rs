use anyhow::{Context, Result};
use serde::Deserialize;

use crate::state::Unit;

const CATALOG_TOML: &str = include_str!("../../assets/programs.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[serde(rename = "program", default)]
    pub programs: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub display_name: String,
    pub kind: CatalogKind,
    #[serde(default)]
    pub calendar_title: Option<String>,
    #[serde(default)]
    pub root_ref: Option<String>,
    pub unit: Unit,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CatalogKind {
    Cycle,
    Book,
}

impl Catalog {
    pub fn embedded() -> Result<Self> {
        let cat: Catalog = toml::from_str(CATALOG_TOML).context("parsing embedded programs.toml")?;
        Ok(cat)
    }

    pub fn find(&self, id: &str) -> Option<&CatalogEntry> {
        self.programs.iter().find(|p| p.id == id)
    }

    pub fn cycles(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.programs.iter().filter(|p| p.kind == CatalogKind::Cycle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses() {
        let c = Catalog::embedded().unwrap();
        assert!(!c.programs.is_empty());
        // sanity: known entries
        assert!(c.find("daf-yomi").is_some());
        assert!(c.find("book:mishnah-berurah").is_some());
    }

    #[test]
    fn cycles_have_calendar_title() {
        let c = Catalog::embedded().unwrap();
        for p in c.cycles() {
            assert!(
                p.calendar_title.is_some(),
                "cycle {} missing calendar_title",
                p.id
            );
        }
    }

    #[test]
    fn books_have_root_ref() {
        let c = Catalog::embedded().unwrap();
        for p in c.programs.iter().filter(|p| p.kind == CatalogKind::Book) {
            assert!(p.root_ref.is_some(), "book {} missing root_ref", p.id);
        }
    }
}
