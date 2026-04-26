//! Convert Sefaria's inline HTML to styled lines + collected footnotes.
//!
//! Sefaria's text usually contains a small set of tags:
//!   - `<b>...</b>` emphasis
//!   - `<i>...</i>` citations / italic
//!   - `<br>` / `<br/>` line breaks
//!   - `<sup class="footnote-marker">N</sup>` markers
//!   - `<i class="footnote">...</i>` footnote bodies (paired with markers)
//!   - generic `<a>` cross-reference links
//!
//! We do a single regex-based pass: capture footnote bodies into a side list
//! (replaced inline with `[N]` markers), then linearise the rest into styled
//! `Span`s grouped into `Line`s.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use regex::Regex;

#[derive(Debug, Default, Clone)]
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    pub footnotes: Vec<String>,
}

pub fn render_segment(raw: &str) -> Rendered {
    let mut footnotes = Vec::new();
    let stripped = extract_footnotes(raw, &mut footnotes);
    let stripped = strip_footnote_markers(&stripped, &footnotes);
    let stripped = normalize_breaks(&stripped);

    let mut lines = Vec::new();
    for line in stripped.split('\n') {
        lines.push(spans_for_line(line));
    }
    Rendered { lines, footnotes }
}

fn extract_footnotes(input: &str, footnotes: &mut Vec<String>) -> String {
    // Match <i class="footnote">...</i> bodies and lift them out.
    let re = Regex::new(r#"(?is)<i\s+class\s*=\s*"footnote"[^>]*>(.*?)</i>"#).unwrap();
    let mut out = String::new();
    let mut last = 0;
    for cap in re.captures_iter(input) {
        let m = cap.get(0).unwrap();
        out.push_str(&input[last..m.start()]);
        let body = strip_tags(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
        footnotes.push(body.trim().to_string());
        last = m.end();
    }
    out.push_str(&input[last..]);
    out
}

fn strip_footnote_markers(input: &str, footnotes: &[String]) -> String {
    // Replace <sup class="footnote-marker">N</sup> with [N]; if the inner number
    // is missing or the markers and bodies got out of sync, fall back to count.
    let re = Regex::new(r#"(?is)<sup\s+class\s*=\s*"footnote-marker"[^>]*>\s*([^<]*?)\s*</sup>"#)
        .unwrap();
    let mut counter = 0usize;
    let mut out = String::new();
    let mut last = 0;
    for cap in re.captures_iter(input) {
        let m = cap.get(0).unwrap();
        out.push_str(&input[last..m.start()]);
        let inner = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        let label = if inner.is_empty() {
            counter += 1;
            format!("[{}]", counter)
        } else {
            counter += 1;
            format!("[{}]", inner)
        };
        out.push_str(&label);
        last = m.end();
    }
    out.push_str(&input[last..]);
    let _ = footnotes;
    out
}

fn normalize_breaks(input: &str) -> String {
    let re = Regex::new(r"(?is)<br\s*/?\s*>").unwrap();
    re.replace_all(input, "\n").into_owned()
}

fn strip_tags(input: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    let no_tags = re.replace_all(input, "");
    decode_entities(&no_tags)
}

fn decode_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Convert a single line (with no `<br>` or `<i class="footnote">`) into spans.
fn spans_for_line(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::default();
    let mut idx = 0usize;
    let bytes = line.as_bytes();

    while idx < bytes.len() {
        if bytes[idx] == b'<' {
            // Try to match a tag.
            if let Some(end_rel) = line[idx..].find('>') {
                let end = idx + end_rel;
                let tag_inner = &line[idx + 1..end];
                let lower = tag_inner.to_ascii_lowercase();
                let lower_trim = lower.trim();
                let new_style_opt = match lower_trim {
                    "b" | "strong" => Some(style.add_modifier(Modifier::BOLD)),
                    "/b" | "/strong" => Some(style.remove_modifier(Modifier::BOLD)),
                    "i" | "em" => Some(style.add_modifier(Modifier::ITALIC)),
                    "/i" | "/em" => Some(style.remove_modifier(Modifier::ITALIC)),
                    "small" => Some(style.add_modifier(Modifier::DIM)),
                    "/small" => Some(style.remove_modifier(Modifier::DIM)),
                    _ => None,
                };
                if let Some(s) = new_style_opt {
                    style = s;
                    idx = end + 1;
                    continue;
                }
                // Unknown tag (e.g. <a ...>, <span ...>): skip it entirely.
                idx = end + 1;
                continue;
            }
        }

        // Plain text run: gather until next '<' or end.
        let next_lt = line[idx..].find('<').map(|p| idx + p).unwrap_or(line.len());
        let chunk = &line[idx..next_lt];
        if !chunk.is_empty() {
            spans.push(Span::styled(decode_entities(chunk), style));
        }
        idx = next_lt;
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn plain_text_passes_through() {
        let r = render_segment("Hello world");
        assert_eq!(r.lines.len(), 1);
        assert_eq!(r.footnotes.len(), 0);
        assert_eq!(line_text(&r.lines[0]), "Hello world");
    }

    #[test]
    fn bold_becomes_bold_span() {
        let r = render_segment("a <b>bold</b> c");
        let line = &r.lines[0];
        let bold_span = line
            .spans
            .iter()
            .find(|s| s.content == "bold")
            .expect("bold span");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn italic_becomes_italic_span() {
        let r = render_segment("see <i>Beit Yosef</i>");
        let line = &r.lines[0];
        let it = line
            .spans
            .iter()
            .find(|s| s.content == "Beit Yosef")
            .unwrap();
        assert!(it.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn br_splits_lines() {
        let r = render_segment("first<br>second<br/>third");
        assert_eq!(r.lines.len(), 3);
        assert_eq!(line_text(&r.lines[0]), "first");
        assert_eq!(line_text(&r.lines[1]), "second");
        assert_eq!(line_text(&r.lines[2]), "third");
    }

    #[test]
    fn footnote_body_extracted() {
        let r = render_segment(
            r#"thus<sup class="footnote-marker">1</sup><i class="footnote">explanation here</i>."#,
        );
        assert_eq!(r.footnotes, vec!["explanation here"]);
        assert_eq!(line_text(&r.lines[0]), "thus[1].");
    }

    #[test]
    fn unknown_tags_stripped() {
        let r = render_segment(r##"<a href="#">Genesis 1:1</a>"##);
        assert_eq!(line_text(&r.lines[0]), "Genesis 1:1");
    }

    #[test]
    fn entities_decoded() {
        let r = render_segment("A&nbsp;B &amp; C");
        assert_eq!(line_text(&r.lines[0]), "A B & C");
    }

    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}
