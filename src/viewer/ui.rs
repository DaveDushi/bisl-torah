use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout as RLayout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::config::{Config, Lang, Layout};
use crate::sefaria::{CalendarItem, RefText};
use crate::signals;
use crate::viewer::{
    bidi,
    layout::{self, Resolved},
    markup,
};

pub type NextFn<'a> = dyn FnMut() -> Option<(CalendarItem, RefText)> + 'a;

const POLL_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoneState {
    Running,
    AgentDone,
}

pub struct Session {
    pub id: String,
    pub signal_dir: PathBuf,
}

pub struct Inputs<'a> {
    pub config: &'a Config,
    pub item: &'a CalendarItem,
    pub text: &'a RefText,
    pub session: Option<&'a Session>,
    pub on_next: Option<Box<NextFn<'a>>>,
}

struct App<'a> {
    cfg: &'a Config,
    item: CalendarItem,
    text: RefText,
    session: Option<&'a Session>,
    on_next: Option<Box<NextFn<'a>>>,
    lang: Lang,
    nikud: bool,
    scroll: u16,
    show_footnotes: bool,
    refresh_pending: bool,
    done: DoneState,
    last_tick: Instant,
}

pub fn run(inputs: Inputs<'_>) -> Result<()> {
    let mut app = App {
        cfg: inputs.config,
        item: inputs.item.clone(),
        text: inputs.text.clone(),
        session: inputs.session,
        on_next: inputs.on_next,
        lang: inputs.config.default_lang,
        nikud: inputs.config.nikud,
        scroll: 0,
        show_footnotes: false,
        refresh_pending: false,
        done: DoneState::Running,
        last_tick: Instant::now(),
    };

    if let Some(s) = app.session {
        let pid_path = signals::pid_path(&s.signal_dir, &s.id);
        let _ = signals::write_pid(&pid_path, std::process::id());
    }

    let mut terminal = init_terminal().context("setting up terminal")?;
    let result = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal).ok();

    if let Some(s) = app.session {
        let pid_path = signals::pid_path(&s.signal_dir, &s.id);
        signals::remove_quiet(&pid_path);
        // Leave .done / .refresh-pending; cleaned up by next spawn.
    }

    result
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        // Signal-file polling.
        if app.last_tick.elapsed() >= POLL_INTERVAL {
            poll_signals(app);
            app.last_tick = Instant::now();
        }

        if event::poll(POLL_INTERVAL)? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(k.code, KeyCode::Char('c'))
                {
                    return Ok(());
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    _ if app.done == DoneState::AgentDone => {
                        // Any key dismisses once agent has finished.
                        return Ok(());
                    }
                    KeyCode::Char('n') => next_item(app),
                    KeyCode::Char('b') => app.lang = Lang::Both,
                    KeyCode::Char('h') => app.lang = Lang::Hebrew,
                    KeyCode::Char('e') => app.lang = Lang::English,
                    KeyCode::Char('f') => app.show_footnotes = !app.show_footnotes,
                    KeyCode::Char('v') => app.nikud = !app.nikud,
                    KeyCode::Char('j') | KeyCode::Down => app.scroll = app.scroll.saturating_add(1),
                    KeyCode::Char('k') | KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
                    KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
                    KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
                    KeyCode::Char('g') => app.scroll = 0,
                    KeyCode::Char('G') => app.scroll = u16::MAX / 2,
                    _ => {}
                }
            }
        }
    }
}

fn next_item(app: &mut App) {
    if let Some(cb) = app.on_next.as_mut() {
        if let Some((new_item, new_text)) = cb() {
            app.item = new_item;
            app.text = new_text;
            app.scroll = 0;
            app.show_footnotes = false;
        }
    }
    if app.refresh_pending {
        if let Some(s) = app.session {
            signals::remove_quiet(&signals::refresh_path(&s.signal_dir, &s.id));
        }
        app.refresh_pending = false;
    }
}

fn poll_signals(app: &mut App) {
    let Some(s) = app.session else { return };
    if app.done == DoneState::Running {
        let done_path = signals::done_path(&s.signal_dir, &s.id);
        if done_path.exists() {
            app.done = DoneState::AgentDone;
        }
    }
    if !app.refresh_pending {
        let refresh_path = signals::refresh_path(&s.signal_dir, &s.id);
        if refresh_path.exists() {
            app.refresh_pending = true;
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = RLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(5),    // body
            Constraint::Length(if app.show_footnotes { 6 } else { 0 }),
            Constraint::Length(2), // footer
        ])
        .split(area);

    draw_header(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    if app.show_footnotes {
        draw_footnotes(f, chunks[2], app);
    }
    draw_footer(f, chunks[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title_en = if app.item.display_value.en.is_empty() {
        app.item.title.en.clone()
    } else {
        app.item.display_value.en.clone()
    };
    let title_he = if app.item.display_value.he.is_empty() {
        app.item.title.he.clone()
    } else {
        app.item.display_value.he.clone()
    };
    let title_he = bidi::to_visual(&title_he);

    let columns = RLayout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let en_p = Paragraph::new(Line::from(vec![Span::styled(
        title_en,
        Style::default().add_modifier(Modifier::BOLD),
    )]))
    .block(Block::default().borders(Borders::BOTTOM));
    let he_p = Paragraph::new(Line::from(vec![Span::styled(
        title_he,
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Yellow),
    )]))
    .block(Block::default().borders(Borders::BOTTOM))
    .alignment(ratatui::layout::Alignment::Right);

    f.render_widget(en_p, columns[0]);
    f.render_widget(he_p, columns[1]);
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let resolved = layout::resolve(app.cfg.layout, area.width);
    let (en_text, he_text) = render_bilingual(&app.text, app.nikud);

    match (app.lang, resolved) {
        (Lang::Both, Resolved::SideBySide) => {
            let columns = RLayout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            render_pane(f, columns[0], "English", &en_text, app.scroll, false);
            render_pane(f, columns[1], "עברית", &he_text, app.scroll, true);
        }
        (Lang::Both, Resolved::Stacked) => {
            let rows = RLayout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            render_pane(f, rows[0], "עברית", &he_text, app.scroll, true);
            render_pane(f, rows[1], "English", &en_text, app.scroll, false);
        }
        (Lang::Hebrew, _) => {
            render_pane(f, area, "עברית", &he_text, app.scroll, true);
        }
        (Lang::English, _) => {
            render_pane(f, area, "English", &en_text, app.scroll, false);
        }
    }
}

fn render_pane(
    f: &mut Frame,
    area: Rect,
    title: &str,
    lines: &[Line<'static>],
    scroll: u16,
    rtl: bool,
) {
    let display_title = if rtl {
        bidi::to_visual(title)
    } else {
        title.to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", display_title));
    let alignment = if rtl {
        ratatui::layout::Alignment::Right
    } else {
        ratatui::layout::Alignment::Left
    };

    if rtl {
        // ratatui's wrap operates on logical-order text; if we bidi-reorder
        // first and then let ratatui wrap, words get split mid-character. So
        // we wrap manually in logical order and bidi each wrapped sub-line.
        let inner_width = area.width.saturating_sub(2) as usize;
        let visual = wrap_and_bidi(lines, inner_width);
        let p = Paragraph::new(visual)
            .block(block)
            .alignment(alignment)
            .scroll((scroll, 0));
        f.render_widget(p, area);
    } else {
        let p = Paragraph::new(lines.to_vec())
            .block(block)
            .wrap(Wrap { trim: false })
            .alignment(alignment)
            .scroll((scroll, 0));
        f.render_widget(p, area);
    }
}

fn wrap_and_bidi(lines: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines.to_vec();
    }
    let mut out = Vec::new();
    for line in lines {
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        if plain.is_empty() {
            out.push(Line::from(""));
            continue;
        }
        let wrapped = word_wrap(&plain, width);
        if wrapped.is_empty() {
            out.push(Line::from(""));
        } else {
            for chunk in wrapped {
                out.push(Line::from(bidi::to_visual(&chunk)));
            }
        }
    }
    out
}

fn word_wrap(s: &str, width: usize) -> Vec<String> {
    if width == 0 || s.is_empty() {
        return vec![s.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;
    for word in s.split_whitespace() {
        let word_w = cell_width(word);
        let extra = if current.is_empty() { 0 } else { 1 };
        if current_w + extra + word_w > width && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_w = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_w += 1;
        }
        if word_w > width {
            for c in word.chars() {
                let cw = if is_combining(c) { 0 } else { 1 };
                if current_w + cw > width && !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                    current_w = 0;
                }
                current.push(c);
                current_w += cw;
            }
        } else {
            current.push_str(word);
            current_w += word_w;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn cell_width(s: &str) -> usize {
    s.chars().filter(|c| !is_combining(*c)).count()
}

fn is_combining(c: char) -> bool {
    let cp = c as u32;
    matches!(
        cp,
        0x0300..=0x036F        // combining diacritical marks
        | 0x0591..=0x05BD      // Hebrew nikud / cantillation
        | 0x05BF
        | 0x05C1..=0x05C2
        | 0x05C4..=0x05C5
        | 0x05C7
    )
}

fn render_bilingual(text: &RefText, nikud: bool) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let mut en_lines: Vec<Line<'static>> = Vec::new();
    for (i, seg) in text.en.iter().enumerate() {
        let seg = format!("{}. {}", i + 1, seg);
        let r = markup::render_segment(&seg);
        en_lines.extend(r.lines);
        en_lines.push(Line::from(""));
    }
    // For Hebrew, do NOT prefix segments with LTR digits — that mixes scripts and
    // breaks the terminal's bidi algorithm. Pass raw logical-order Hebrew and let
    // the terminal (Windows Terminal, iTerm2, kitty, etc.) do RTL rendering.
    let mut he_lines: Vec<Line<'static>> = Vec::new();
    for seg in text.he.iter() {
        let working = if nikud {
            seg.clone()
        } else {
            bidi::strip_nikud(seg)
        };
        let r = markup::render_segment(&working);
        he_lines.extend(r.lines);
        he_lines.push(Line::from(""));
    }
    (en_lines, he_lines)
}

fn draw_footnotes(f: &mut Frame, area: Rect, app: &App) {
    let mut all = Vec::new();
    for seg in app.text.en.iter().chain(app.text.he.iter()) {
        let r = markup::render_segment(seg);
        for (i, fn_) in r.footnotes.iter().enumerate() {
            all.push(Line::from(vec![
                Span::styled(
                    format!("[{}] ", i + 1),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                ),
                Span::raw(fn_.clone()),
            ]));
        }
    }
    if all.is_empty() {
        all.push(Line::from(Span::styled(
            "(no footnotes in this segment)",
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    let p = Paragraph::new(all)
        .block(Block::default().borders(Borders::ALL).title(" footnotes "))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

const FOOTER_TITLE_MIN: usize = 10;

const KEYHINTS_RUNNING: &[&str] = &[
    "q quit · n next · b/h/e lang · v vowels · f footnotes · j/k scroll",
    "q quit · n next · b/h/e lang · v vowels",
    "q quit · n next",
    "q quit",
];

const KEYHINTS_DONE: &[&str] = &["agent done — press any key", "agent done", "done"];

/// Pick the longest keyhint that leaves at least `FOOTER_TITLE_MIN + 1` cols for the title.
/// Falls back to the shortest variant if even that doesn't fit.
fn pick_keyhint(footer_w: usize, done: DoneState) -> &'static str {
    let variants = match done {
        DoneState::Running => KEYHINTS_RUNNING,
        DoneState::AgentDone => KEYHINTS_DONE,
    };
    for h in variants {
        let w = h.chars().count();
        if footer_w >= w + 1 + FOOTER_TITLE_MIN {
            return h;
        }
    }
    variants.last().copied().unwrap_or("")
}

fn truncate_with_ellipsis(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

/// Build the footer's left-side title, picking the longest variant that fits in `budget` cols.
///
/// Tiers (longest first):
///   1. " {citation} · {category}{prompt_full}"
///   2. " {citation}{prompt_full}"           (drop category)
///   3. " {citation}{prompt_short}"          (abbreviate to "· n")
///   4. " {citation_truncated}{prompt_short}"
///   5. " {citation_truncated}"              (drop the new-prompt indicator — last resort)
fn build_footer_title(
    citation: &str,
    category: &str,
    refresh_pending: bool,
    budget: usize,
) -> String {
    let prompt_full = if refresh_pending {
        " · new prompt — press n"
    } else {
        ""
    };
    let prompt_short = if refresh_pending { " · n" } else { "" };

    let candidates: [String; 3] = [
        format!(" {} · {}{}", citation, category, prompt_full),
        format!(" {}{}", citation, prompt_full),
        format!(" {}{}", citation, prompt_short),
    ];
    for c in &candidates {
        if c.chars().count() <= budget {
            return c.clone();
        }
    }

    // Tier 4: ellipsis-truncate citation, keep "· n"
    // Require at least 2 cols of citation budget so we keep at least one real char + ellipsis;
    // " … · n" alone is uninformative — better to drop the suffix and show more citation.
    let suffix_count = prompt_short.chars().count();
    let leading = 1; // leading space
    let cite_budget_with_suffix = budget.saturating_sub(leading + suffix_count);
    if cite_budget_with_suffix >= 2 {
        let cite = truncate_with_ellipsis(citation, cite_budget_with_suffix);
        let candidate = format!(" {}{}", cite, prompt_short);
        if candidate.chars().count() <= budget {
            return candidate;
        }
    }

    // Tier 5: drop the indicator entirely.
    let cite_budget = budget.saturating_sub(leading);
    let cite = truncate_with_ellipsis(citation, cite_budget);
    format!(" {}", cite)
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let display = if app.item.display_value.en.is_empty() {
        app.item.ref_.clone()
    } else {
        app.item.display_value.en.clone()
    };

    let footer_w = area.width as usize;
    let keyhint = pick_keyhint(footer_w, app.done);
    let right_w = keyhint.chars().count();
    let left_budget = footer_w.saturating_sub(right_w + 1);

    let left = build_footer_title(
        &display,
        &app.item.category,
        app.refresh_pending,
        left_budget,
    );

    let columns = RLayout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_w as u16 + 1)])
        .split(area);

    let style_left = match app.done {
        DoneState::Running => Style::default(),
        DoneState::AgentDone => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    };

    let p_left = Paragraph::new(Span::styled(left, style_left));
    let p_right = Paragraph::new(Span::styled(
        keyhint,
        Style::default().add_modifier(Modifier::DIM),
    ))
    .alignment(ratatui::layout::Alignment::Right);

    f.render_widget(p_left, columns[0]);
    f.render_widget(p_right, columns[1]);
}

#[allow(dead_code)]
fn dummy_path() -> &'static Path {
    Path::new("")
}

// Re-export Layout for callers convenience (kept private here).
#[allow(dead_code)]
fn _layout_marker(_l: Layout) {}

#[cfg(test)]
mod footer_tests {
    use super::*;

    #[test]
    fn keyhint_full_at_wide_width() {
        let h = pick_keyhint(120, DoneState::Running);
        assert_eq!(h, KEYHINTS_RUNNING[0]);
    }

    #[test]
    fn keyhint_drops_scroll_and_footnotes_at_medium() {
        // Full is 66 chars; full + 1 + TITLE_MIN(10) = 77. At 70 wide we should drop to medium.
        let h = pick_keyhint(70, DoneState::Running);
        assert_eq!(h, KEYHINTS_RUNNING[1]);
    }

    #[test]
    fn keyhint_minimal_q_n_at_narrow() {
        // Should be wide enough for "q quit · n next" (15) + 1 + 10 = 26 but not for medium (40+1+10=51).
        let h = pick_keyhint(30, DoneState::Running);
        assert_eq!(h, "q quit · n next");
    }

    #[test]
    fn keyhint_tiny_at_extreme_width() {
        let h = pick_keyhint(20, DoneState::Running);
        assert_eq!(h, "q quit");
    }

    #[test]
    fn keyhint_falls_back_when_nothing_fits() {
        // Even narrower than tiny + title_min: still returns the shortest variant.
        let h = pick_keyhint(5, DoneState::Running);
        assert_eq!(h, "q quit");
    }

    #[test]
    fn keyhint_done_state_uses_done_variants() {
        let h = pick_keyhint(120, DoneState::AgentDone);
        assert_eq!(h, KEYHINTS_DONE[0]);
    }

    #[test]
    fn title_full_form_at_wide_width() {
        let title = build_footer_title("Mishnah Middot 4:4-5", "Mishnah", false, 80);
        assert_eq!(title, " Mishnah Middot 4:4-5 · Mishnah");
    }

    #[test]
    fn title_with_new_prompt_full() {
        let title = build_footer_title("Mishnah Middot 4:4-5", "Mishnah", true, 80);
        assert_eq!(
            title,
            " Mishnah Middot 4:4-5 · Mishnah · new prompt — press n"
        );
    }

    #[test]
    fn title_drops_category_at_medium() {
        // " Mishnah Middot 4:4-5 · Mishnah" is 31 chars. budget=25 should drop category.
        let title = build_footer_title("Mishnah Middot 4:4-5", "Mishnah", false, 25);
        assert_eq!(title, " Mishnah Middot 4:4-5");
    }

    #[test]
    fn title_abbreviates_new_prompt_at_narrow() {
        // citation(20) + " · new prompt — press n"(23) + leading(1) = 44. budget=30 forces "· n".
        let title = build_footer_title("Mishnah Middot 4:4-5", "Mishnah", true, 30);
        assert_eq!(title, " Mishnah Middot 4:4-5 · n");
    }

    #[test]
    fn title_truncates_citation_keeping_n_indicator() {
        let title = build_footer_title("A Very Long Citation Text", "Mishnah", true, 15);
        assert!(title.ends_with(" · n"));
        assert!(title.contains('…'));
        assert!(title.chars().count() <= 15);
    }

    #[test]
    fn title_drops_n_indicator_at_extreme_narrow() {
        let title = build_footer_title("Mishnah Middot 4:4-5", "Mishnah", true, 6);
        assert!(!title.ends_with(" · n"));
        assert!(title.chars().count() <= 6);
    }

    #[test]
    fn title_no_refresh_no_n_suffix() {
        let title = build_footer_title("Mishnah Middot 4:4-5", "Mishnah", false, 30);
        assert!(!title.contains(" · n"));
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate_with_ellipsis("hello world", 6), "hello…");
        assert_eq!(truncate_with_ellipsis("short", 10), "short");
    }
}
