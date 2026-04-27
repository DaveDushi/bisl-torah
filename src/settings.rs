use std::io::{self, Stdout};

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

use crate::config::{
    parse_percent, Config, DisplayMode, Lang, Layout, POPUP_MAX_PCT, POPUP_MIN_PCT,
};
use crate::paths;

const KNOWN_CATEGORIES: &[&str] = &[
    "*",
    "Parashat Hashavua",
    "Haftarah",
    "Daf Yomi",
    "929",
    "Daily Mishnah",
    "Daily Rambam",
    "Daily Rambam (3 Chapters)",
    "Daf a Week",
    "Halakhah",
    "Halakhah Yomit",
    "Mishnah",
    "Chasidut",
    "Tanya Yomi",
    "Tanakh",
    "Tanakh Yomi",
    "Talmud",
    "Yerushalmi Yomi",
    "Chok LeYisrael",
    "Arukh HaShulchan Yomi",
];

const FIELD_LABELS: &[&str] = &[
    "categories",
    "layout",
    "default_lang",
    "nikud",
    "display",
    "popup.wt_width",
    "popup.tmux_width",
    "popup.tmux_height",
];

const POPUP_STEP: i32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browsing,
    EditingCategories,
}

struct Editor {
    cfg: Config,
    original: Config,
    selected_field: usize,
    mode: Mode,
    cat_cursor: usize,
    status: Option<String>,
}

impl Editor {
    fn new(cfg: Config) -> Self {
        let original = cfg.clone();
        Self {
            cfg,
            original,
            selected_field: 0,
            mode: Mode::Browsing,
            cat_cursor: 0,
            status: None,
        }
    }

    fn dirty(&self) -> bool {
        self.cfg.categories != self.original.categories
            || self.cfg.layout != self.original.layout
            || self.cfg.default_lang != self.original.default_lang
            || self.cfg.nikud != self.original.nikud
            || self.cfg.display != self.original.display
            || self.cfg.popup != self.original.popup
    }
}

pub fn run(cfg: Config) -> Result<()> {
    let mut editor = Editor::new(cfg);
    let mut terminal = init_terminal().context("setting up terminal")?;
    let result = run_loop(&mut terminal, &mut editor);
    restore_terminal(&mut terminal).ok();
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

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, editor: &mut Editor) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, editor))?;

        let evt = event::read()?;
        let Event::Key(k) = evt else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c')) {
            return Ok(());
        }

        match editor.mode {
            Mode::Browsing => {
                if handle_browsing(editor, k.code)? {
                    return Ok(());
                }
            }
            Mode::EditingCategories => handle_categories(editor, k.code),
        }
    }
}

fn handle_browsing(editor: &mut Editor, code: KeyCode) -> Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Char('s') => match editor.cfg.save() {
            Ok(()) => {
                editor.original = editor.cfg.clone();
                editor.status = Some("saved".to_string());
            }
            Err(e) => {
                editor.status = Some(format!("error: {}", e));
            }
        },
        KeyCode::Up | KeyCode::Char('k') => {
            if editor.selected_field == 0 {
                editor.selected_field = FIELD_LABELS.len() - 1;
            } else {
                editor.selected_field -= 1;
            }
            editor.status = None;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            editor.selected_field = (editor.selected_field + 1) % FIELD_LABELS.len();
            editor.status = None;
        }
        KeyCode::Left | KeyCode::Char('h') => {
            cycle_field(editor, -1);
            editor.status = None;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            cycle_field(editor, 1);
            editor.status = None;
        }
        KeyCode::Char(' ') if editor.selected_field == field_index("nikud") => {
            editor.cfg.nikud = !editor.cfg.nikud;
            editor.status = None;
        }
        KeyCode::Enter => {
            if editor.selected_field == field_index("categories") {
                editor.mode = Mode::EditingCategories;
                editor.cat_cursor = 0;
                editor.status = None;
            } else if editor.selected_field == field_index("nikud") {
                editor.cfg.nikud = !editor.cfg.nikud;
                editor.status = None;
            } else {
                cycle_field(editor, 1);
                editor.status = None;
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_categories(editor: &mut Editor, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            editor.mode = Mode::Browsing;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if editor.cat_cursor == 0 {
                editor.cat_cursor = KNOWN_CATEGORIES.len() - 1;
            } else {
                editor.cat_cursor -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            editor.cat_cursor = (editor.cat_cursor + 1) % KNOWN_CATEGORIES.len();
        }
        KeyCode::Char(' ') => {
            let cat = KNOWN_CATEGORIES[editor.cat_cursor];
            if let Some(pos) = editor
                .cfg
                .categories
                .iter()
                .position(|c| c.eq_ignore_ascii_case(cat))
            {
                editor.cfg.categories.remove(pos);
            } else {
                editor.cfg.categories.push(cat.to_string());
            }
        }
        _ => {}
    }
}

fn field_index(name: &str) -> usize {
    FIELD_LABELS.iter().position(|f| *f == name).unwrap()
}

fn cycle_field(editor: &mut Editor, delta: i32) {
    let name = FIELD_LABELS[editor.selected_field];
    match name {
        "layout" => editor.cfg.layout = cycle_layout(editor.cfg.layout, delta),
        "default_lang" => editor.cfg.default_lang = cycle_lang(editor.cfg.default_lang, delta),
        "display" => editor.cfg.display = cycle_display(editor.cfg.display, delta),
        "nikud" => editor.cfg.nikud = !editor.cfg.nikud,
        "popup.wt_width" => {
            editor.cfg.popup.wt_width = step_percent(&editor.cfg.popup.wt_width, delta, "40%");
        }
        "popup.tmux_width" => {
            editor.cfg.popup.tmux_width = step_percent(&editor.cfg.popup.tmux_width, delta, "80%");
        }
        "popup.tmux_height" => {
            editor.cfg.popup.tmux_height =
                step_percent(&editor.cfg.popup.tmux_height, delta, "70%");
        }
        _ => {}
    }
}

/// Step a percentage string by `delta * POPUP_STEP`%, clamped to [POPUP_MIN_PCT, POPUP_MAX_PCT].
/// Falls back to `default` if the current value can't be parsed.
fn step_percent(current: &str, delta: i32, default: &str) -> String {
    let current_pct = parse_percent(current)
        .or_else(|| parse_percent(default))
        .map(|f| (f * 100.0).round() as i32)
        .unwrap_or(40);
    let next = (current_pct + delta * POPUP_STEP).clamp(POPUP_MIN_PCT as i32, POPUP_MAX_PCT as i32);
    format!("{}%", next)
}

fn cycle_layout(v: Layout, delta: i32) -> Layout {
    let opts = [Layout::Auto, Layout::SideBySide, Layout::Stacked];
    cycle(&opts, v, delta)
}

fn cycle_lang(v: Lang, delta: i32) -> Lang {
    let opts = [Lang::Both, Lang::Hebrew, Lang::English];
    cycle(&opts, v, delta)
}

fn cycle_display(v: DisplayMode, delta: i32) -> DisplayMode {
    let opts = [
        DisplayMode::Auto,
        DisplayMode::WtSplit,
        DisplayMode::TmuxPopup,
        DisplayMode::NewConsole,
    ];
    cycle(&opts, v, delta)
}

fn cycle<T: Copy + PartialEq>(opts: &[T], current: T, delta: i32) -> T {
    let n = opts.len() as i32;
    let idx = opts.iter().position(|x| *x == current).unwrap_or(0) as i32;
    let next = ((idx + delta) % n + n) % n;
    opts[next as usize]
}

fn draw(f: &mut Frame, editor: &Editor) {
    let area = f.area();
    let chunks = RLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    draw_header(f, chunks[0], editor);
    draw_body(f, chunks[1], editor);
    draw_footer(f, chunks[2], editor);
}

fn draw_header(f: &mut Frame, area: Rect, editor: &Editor) {
    let path = paths::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".to_string());
    let dirty = if editor.dirty() { " * unsaved" } else { "" };
    let title = Line::from(vec![
        Span::styled(
            "bisl-torah settings",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            dirty,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let path_line = Line::from(Span::styled(
        path,
        Style::default().add_modifier(Modifier::DIM),
    ));
    let p = Paragraph::new(vec![title, path_line]).block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(p, area);
}

fn draw_body(f: &mut Frame, area: Rect, editor: &Editor) {
    let columns = RLayout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    draw_field_list(f, columns[0], editor);
    match editor.mode {
        Mode::Browsing => draw_value_pane(f, columns[1], editor),
        Mode::EditingCategories => draw_categories_pane(f, columns[1], editor),
    }
}

fn draw_field_list(f: &mut Frame, area: Rect, editor: &Editor) {
    let lines: Vec<Line> = FIELD_LABELS
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let marker = if i == editor.selected_field {
                "▸ "
            } else {
                "  "
            };
            let style = if i == editor.selected_field {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(vec![Span::styled(format!("{}{}", marker, name), style)])
        })
        .collect();
    let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" fields "));
    f.render_widget(p, area);
}

fn draw_value_pane(f: &mut Frame, area: Rect, editor: &Editor) {
    let name = FIELD_LABELS[editor.selected_field];
    let (current, options, hint) = match name {
        "categories" => {
            let listed = if editor.cfg.categories.is_empty() {
                "(none)".to_string()
            } else {
                editor.cfg.categories.join(", ")
            };
            (
                listed,
                String::new(),
                "press enter to edit list".to_string(),
            )
        }
        "layout" => (
            layout_label(editor.cfg.layout).to_string(),
            "auto · side-by-side · stacked".to_string(),
            "←/→ to cycle".to_string(),
        ),
        "default_lang" => (
            lang_label(editor.cfg.default_lang).to_string(),
            "both · hebrew · english".to_string(),
            "←/→ to cycle".to_string(),
        ),
        "nikud" => (
            if editor.cfg.nikud { "true" } else { "false" }.to_string(),
            "true · false".to_string(),
            "space or enter to toggle".to_string(),
        ),
        "display" => (
            display_label(editor.cfg.display).to_string(),
            "auto · wt-split · tmux-popup · new-console".to_string(),
            "←/→ to cycle".to_string(),
        ),
        "popup.wt_width" => (
            editor.cfg.popup.wt_width.clone(),
            format!(
                "{}% – {}% in {}% steps",
                POPUP_MIN_PCT, POPUP_MAX_PCT, POPUP_STEP
            ),
            "←/→ to adjust by 5% (Windows Terminal split)".to_string(),
        ),
        "popup.tmux_width" => (
            editor.cfg.popup.tmux_width.clone(),
            format!(
                "{}% – {}% in {}% steps",
                POPUP_MIN_PCT, POPUP_MAX_PCT, POPUP_STEP
            ),
            "←/→ to adjust by 5% (tmux popup width)".to_string(),
        ),
        "popup.tmux_height" => (
            editor.cfg.popup.tmux_height.clone(),
            format!(
                "{}% – {}% in {}% steps",
                POPUP_MIN_PCT, POPUP_MAX_PCT, POPUP_STEP
            ),
            "←/→ to adjust by 5% (tmux popup height)".to_string(),
        ),
        _ => (String::new(), String::new(), String::new()),
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("field: ", Style::default().add_modifier(Modifier::DIM)),
            Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("current: ", Style::default().add_modifier(Modifier::DIM)),
            Span::styled(
                current,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    if !options.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("options: ", Style::default().add_modifier(Modifier::DIM)),
            Span::raw(options),
        ]));
    }
    if !hint.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )));
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" value "))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_categories_pane(f: &mut Frame, area: Rect, editor: &Editor) {
    let lines: Vec<Line> = KNOWN_CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let checked = editor
                .cfg
                .categories
                .iter()
                .any(|c| c.eq_ignore_ascii_case(cat));
            let marker = if i == editor.cat_cursor { "▸ " } else { "  " };
            let box_ = if checked { "[x]" } else { "[ ]" };
            let style = if i == editor.cat_cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if checked {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            let label = if *cat == "*" {
                "* (all categories)".to_string()
            } else {
                cat.to_string()
            };
            Line::from(vec![Span::styled(
                format!("{}{} {}", marker, box_, label),
                style,
            )])
        })
        .collect();
    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" categories — space to toggle, enter/esc to return "),
    );
    f.render_widget(p, area);
}

fn draw_footer(f: &mut Frame, area: Rect, editor: &Editor) {
    let hint = match editor.mode {
        Mode::Browsing => "↑↓ field · ←→ cycle · enter edit · s save · q quit",
        Mode::EditingCategories => "↑↓ move · space toggle · enter/esc return",
    };
    let status = editor.status.clone().unwrap_or_default();
    let columns = RLayout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let status_style = if status.starts_with("error") {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    };
    let p_status = Paragraph::new(Span::styled(status, status_style));
    let p_hint = Paragraph::new(Span::styled(
        hint,
        Style::default().add_modifier(Modifier::DIM),
    ))
    .alignment(ratatui::layout::Alignment::Right);

    f.render_widget(p_status, columns[0]);
    f.render_widget(p_hint, columns[1]);
}

fn layout_label(l: Layout) -> &'static str {
    match l {
        Layout::Auto => "auto",
        Layout::SideBySide => "side-by-side",
        Layout::Stacked => "stacked",
    }
}

fn lang_label(l: Lang) -> &'static str {
    match l {
        Lang::Both => "both",
        Lang::Hebrew => "hebrew",
        Lang::English => "english",
    }
}

fn display_label(d: DisplayMode) -> &'static str {
    match d {
        DisplayMode::Auto => "auto",
        DisplayMode::WtSplit => "wt-split",
        DisplayMode::TmuxPopup => "tmux-popup",
        DisplayMode::NewConsole => "new-console",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_layout_wraps_forward() {
        let v = cycle_layout(Layout::Stacked, 1);
        assert_eq!(v, Layout::Auto);
    }

    #[test]
    fn cycle_layout_wraps_backward() {
        let v = cycle_layout(Layout::Auto, -1);
        assert_eq!(v, Layout::Stacked);
    }

    #[test]
    fn cycle_lang_advances() {
        assert_eq!(cycle_lang(Lang::Both, 1), Lang::Hebrew);
        assert_eq!(cycle_lang(Lang::English, 1), Lang::Both);
    }

    #[test]
    fn cycle_display_full_cycle() {
        let mut v = DisplayMode::Auto;
        for _ in 0..4 {
            v = cycle_display(v, 1);
        }
        assert_eq!(v, DisplayMode::Auto);
    }

    #[test]
    fn dirty_flag_tracks_changes() {
        let cfg = Config::default();
        let mut editor = Editor::new(cfg);
        assert!(!editor.dirty());
        editor.cfg.nikud = !editor.cfg.nikud;
        assert!(editor.dirty());
    }

    #[test]
    fn step_percent_advances_by_5() {
        assert_eq!(step_percent("40%", 1, "40%"), "45%");
        assert_eq!(step_percent("40%", -1, "40%"), "35%");
        assert_eq!(step_percent("40%", 2, "40%"), "50%");
    }

    #[test]
    fn step_percent_clamps_at_bounds() {
        assert_eq!(step_percent("90%", 1, "40%"), "90%");
        assert_eq!(step_percent("10%", -1, "40%"), "10%");
        assert_eq!(step_percent("85%", 2, "40%"), "90%");
    }

    #[test]
    fn step_percent_falls_back_to_default_on_garbage() {
        assert_eq!(step_percent("garbage", 1, "40%"), "45%");
    }

    #[test]
    fn dirty_flag_tracks_popup_changes() {
        let cfg = Config::default();
        let mut editor = Editor::new(cfg);
        assert!(!editor.dirty());
        editor.cfg.popup.wt_width = "50%".into();
        assert!(editor.dirty());
    }
}
