//! Hebrew RTL helpers.
//!
//! ratatui draws cell-by-cell into a grid via crossterm. Most terminals do not
//! apply Unicode bidi reordering across discrete cell writes, so logical-order
//! Hebrew arrives on screen in logical order (left-to-right). We compensate by
//! converting each line to visual order ourselves before handing it to ratatui:
//! reverse the characters within RTL runs and mirror paired punctuation.

use unicode_bidi::BidiInfo;

/// Convert a single logical-order line to visual (display-order) form.
pub fn to_visual(line: &str) -> String {
    if line.is_empty() {
        return String::new();
    }
    let bidi = BidiInfo::new(line, None);
    let mut out = String::with_capacity(line.len());
    for para in &bidi.paragraphs {
        let line_range = para.range.clone();
        let (levels, runs) = bidi.visual_runs(para, line_range);
        for run in runs {
            let chunk = &line[run.clone()];
            let level = levels[run.start];
            if level.is_rtl() {
                for c in chunk.chars().rev() {
                    out.push(mirror(c));
                }
            } else {
                out.push_str(chunk);
            }
        }
    }
    out
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        _ => c,
    }
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
        assert_eq!(to_visual("hello"), "hello");
    }

    #[test]
    fn nikud_stripper_removes_vowel_marks() {
        let with = "מֹשֶׁה";
        let without = strip_nikud(with);
        assert!(without
            .chars()
            .all(|c| !matches!(c as u32, 0x0591..=0x05C7)));
        assert!(without.starts_with('מ'));
    }

    #[test]
    fn pure_hebrew_reverses_to_visual() {
        // Logical "שלום עולם" → visual L→R is the chars reversed.
        let v = to_visual("שלום עולם");
        assert_eq!(v, "םלוע םולש");
    }

    #[test]
    fn ltr_paragraph_with_embedded_hebrew() {
        // LTR base: "abc שלום xyz" → visual keeps LTR runs in place,
        // reverses only the embedded Hebrew run.
        let v = to_visual("abc שלום xyz");
        assert_eq!(v, "abc םולש xyz");
    }

    #[test]
    fn brackets_mirror_in_rtl_run() {
        // "(שלום)" — base RTL. Visual L→R: ')' then reversed Hebrew then '('.
        // Both parens are part of the RTL run and get mirrored.
        let v = to_visual("(שלום)");
        assert_eq!(v, "(םולש)");
    }
}
