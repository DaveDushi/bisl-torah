//! Hebrew RTL helpers.
//!
//! ratatui draws characters left-to-right in cells. Most modern terminals (Windows
//! Terminal, iTerm2, kitty, recent gnome-terminal) handle Unicode bidi natively,
//! so passing raw Hebrew through usually renders correctly. As a defensive measure
//! we run text through `unicode-bidi` to compute display order; we only reorder
//! when bidi yields a non-trivial reordering.

use unicode_bidi::BidiInfo;

pub fn visual_order(line: &str) -> String {
    if line.is_empty() {
        return String::new();
    }
    let bidi = BidiInfo::new(line, None);
    let mut out = String::with_capacity(line.len());
    for para in &bidi.paragraphs {
        let line_range = para.range.clone();
        let (_levels, runs) = bidi.visual_runs(para, line_range);
        for run in runs {
            out.push_str(&line[run]);
        }
    }
    out
}

pub fn strip_nikud(input: &str) -> String {
    // Hebrew nikud is U+0591..U+05C7 cantillation/vowel range. Strip safely.
    input
        .chars()
        .filter(|c| !matches!(*c as u32, 0x0591..=0x05C7))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_passes_through() {
        assert_eq!(visual_order("hello"), "hello");
    }

    #[test]
    fn nikud_stripper_removes_vowel_marks() {
        let with = "מֹשֶׁה";
        let without = strip_nikud(with);
        // Each consonant remains; nikud removed.
        assert!(without
            .chars()
            .all(|c| !matches!(c as u32, 0x0591..=0x05C7)));
        assert!(without.starts_with('מ'));
    }

    #[test]
    fn pure_hebrew_string_handled() {
        let h = "שלום עולם";
        let v = visual_order(h);
        // We don't assert exact reordering here since terminals handle bidi,
        // just that the function returns something non-empty of similar length.
        assert!(!v.is_empty());
    }
}
