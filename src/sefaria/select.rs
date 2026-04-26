use crate::config::Config;
use crate::sefaria::CalendarItem;

pub fn filtered<'a>(items: &'a [CalendarItem], cfg: &Config) -> Vec<&'a CalendarItem> {
    let mut filtered: Vec<&CalendarItem> = items
        .iter()
        .filter(|i| cfg.category_matches(&i.category))
        .collect();
    filtered.sort_by_key(|i| i.order.unwrap_or(u32::MAX));
    filtered
}

pub fn pick<'a>(items: &'a [CalendarItem], cfg: &Config, count: u64) -> Option<&'a CalendarItem> {
    let filtered = filtered(items, cfg);
    if filtered.is_empty() {
        // Fall back to *any* item if filter excludes everything.
        let mut all: Vec<&CalendarItem> = items.iter().collect();
        all.sort_by_key(|i| i.order.unwrap_or(u32::MAX));
        if all.is_empty() {
            return None;
        }
        let idx = (count as usize) % all.len();
        return Some(all[idx]);
    }
    let idx = (count as usize) % filtered.len();
    Some(filtered[idx])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sefaria::TitlePair;

    fn item(category: &str, order: u32, ref_: &str) -> CalendarItem {
        CalendarItem {
            title: TitlePair::default(),
            display_value: TitlePair::default(),
            ref_: ref_.to_string(),
            he_ref: None,
            category: category.to_string(),
            order: Some(order),
            url: None,
            description: None,
            extra_details: None,
        }
    }

    #[test]
    fn rotates_through_whitelist_in_order() {
        let items = vec![
            item("Tanakh", 0, "Genesis 1:1"),
            item("Halakhah", 1, "Halacha A"),
            item("Mishnah", 2, "Mishnah B"),
            item("Talmud", 3, "Sanhedrin 7"),
        ];
        let cfg = Config::default(); // Halakhah, Mishnah, Chasidut

        let p0 = pick(&items, &cfg, 0).unwrap();
        let p1 = pick(&items, &cfg, 1).unwrap();
        let p2 = pick(&items, &cfg, 2).unwrap();
        assert_eq!(p0.ref_, "Halacha A");
        assert_eq!(p1.ref_, "Mishnah B");
        assert_eq!(p2.ref_, "Halacha A"); // wraps
    }

    #[test]
    fn wildcard_includes_everything() {
        let items = vec![item("Tanakh", 0, "Genesis 1:1")];
        let cfg = Config {
            categories: vec!["*".into()],
            ..Default::default()
        };
        let p = pick(&items, &cfg, 0).unwrap();
        assert_eq!(p.ref_, "Genesis 1:1");
    }

    #[test]
    fn falls_back_when_filter_excludes_all() {
        let items = vec![item("Tanakh", 0, "Genesis 1:1")];
        let cfg = Config {
            categories: vec!["Halakhah".into()],
            ..Default::default()
        };
        let p = pick(&items, &cfg, 0).unwrap();
        assert_eq!(p.ref_, "Genesis 1:1");
    }

    #[test]
    fn empty_items_returns_none() {
        let items: Vec<CalendarItem> = vec![];
        let cfg = Config::default();
        assert!(pick(&items, &cfg, 0).is_none());
    }
}
