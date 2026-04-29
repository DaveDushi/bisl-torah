use std::io::{self, Stdout};

use anyhow::{Context, Result};
use chrono::NaiveDate;
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
use crate::programs::catalog::{Catalog, CatalogEntry, CatalogKind};
use crate::programs::enrollment::{
    enroll_book_with_options, enroll_custom_book, enroll_cycle, BookOptions, CycleStart,
};
use crate::sefaria::{IndexNode, SefariaClient};
use crate::state::{EnrollmentKind, State, Unit};

const FIELD_LABELS: &[&str] = &[
    "programs",
    "layout",
    "default_lang",
    "nikud",
    "display",
    "popup.wt_width",
    "popup.tmux_width",
    "popup.tmux_height",
];

const POPUP_STEP: i32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Browsing,
    ManagingPrograms { cursor: usize },
    PickingCatalog { cursor: usize },
    PickingCycleStart { program_id: String, choice: usize, date_input: String },
    PickingBookOptions {
        program_id: String,
        catalog_root: String,
        catalog_unit: Unit,
        field: usize,           // 0 = starting ref, 1 = unit
        starting_ref_input: String,
        unit_choice: BookUnitChoice,
    },
    BrowsingLibrary { stack: Vec<usize>, cursor: usize },
    ViewingHistory { cursor: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BookUnitChoice {
    Auto,
    Segment,
    Section,
    Daf,
}

impl BookUnitChoice {
    fn label(&self) -> &'static str {
        match self {
            BookUnitChoice::Auto => "Auto",
            BookUnitChoice::Segment => "Segment",
            BookUnitChoice::Section => "Section / chapter",
            BookUnitChoice::Daf => "Daf",
        }
    }

    fn cycle(self, delta: i32) -> Self {
        let opts = [
            BookUnitChoice::Auto,
            BookUnitChoice::Segment,
            BookUnitChoice::Section,
            BookUnitChoice::Daf,
        ];
        let n = opts.len() as i32;
        let idx = opts.iter().position(|x| *x == self).unwrap_or(0) as i32;
        let next = ((idx + delta) % n + n) % n;
        opts[next as usize]
    }

    fn resolve(self) -> Option<Unit> {
        match self {
            BookUnitChoice::Auto => None,
            BookUnitChoice::Segment => Some(Unit::Segment),
            BookUnitChoice::Section => Some(Unit::Section),
            BookUnitChoice::Daf => Some(Unit::Daf),
        }
    }
}

struct Editor {
    cfg: Config,
    original: Config,
    state: State,
    catalog: Catalog,
    library: Option<Vec<IndexNode>>,
    selected_field: usize,
    mode: Mode,
    status: Option<String>,
}

impl Editor {
    fn new(cfg: Config, state: State) -> Result<Self> {
        let original = cfg.clone();
        let catalog = Catalog::embedded()?;
        Ok(Self {
            cfg,
            original,
            state,
            catalog,
            library: None,
            selected_field: 0,
            mode: Mode::Browsing,
            status: None,
        })
    }

    fn dirty(&self) -> bool {
        self.cfg.layout != self.original.layout
            || self.cfg.default_lang != self.original.default_lang
            || self.cfg.nikud != self.original.nikud
            || self.cfg.display != self.original.display
            || self.cfg.popup != self.original.popup
    }
}

pub fn run(cfg: Config, state: State) -> Result<()> {
    let mut editor = Editor::new(cfg, state)?;
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

        let mode = editor.mode.clone();
        match mode {
            Mode::Browsing => {
                if handle_browsing(editor, k.code)? {
                    return Ok(());
                }
            }
            Mode::ManagingPrograms { .. } => handle_programs(editor, k.code),
            Mode::PickingCatalog { .. } => handle_catalog(editor, k.code),
            Mode::PickingCycleStart { .. } => handle_cycle_start(editor, k.code),
            Mode::PickingBookOptions { .. } => handle_book_options(editor, k.code),
            Mode::BrowsingLibrary { .. } => handle_library(editor, k.code),
            Mode::ViewingHistory { .. } => handle_history(editor, k.code),
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
            if editor.selected_field == field_index("programs") {
                editor.mode = Mode::ManagingPrograms { cursor: 0 };
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

fn handle_programs(editor: &mut Editor, code: KeyCode) {
    let Mode::ManagingPrograms { mut cursor } = editor.mode.clone() else {
        return;
    };
    let n = editor.state.enrollments.len();
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.mode = Mode::Browsing;
        }
        KeyCode::Up | KeyCode::Char('k') if n > 0 => {
            cursor = if cursor == 0 { n - 1 } else { cursor - 1 };
            editor.mode = Mode::ManagingPrograms { cursor };
        }
        KeyCode::Down | KeyCode::Char('j') if n > 0 => {
            cursor = (cursor + 1) % n;
            editor.mode = Mode::ManagingPrograms { cursor };
        }
        KeyCode::Char('a') => {
            editor.mode = Mode::PickingCatalog { cursor: 0 };
        }
        KeyCode::Char('b') => {
            ensure_library_loaded(editor);
            editor.mode = Mode::BrowsingLibrary {
                stack: Vec::new(),
                cursor: 0,
            };
        }
        KeyCode::Char('h') => {
            editor.mode = Mode::ViewingHistory { cursor: 0 };
        }
        KeyCode::Char('d') if n > 0 => {
            let id = editor.state.enrollments[cursor].program_id.clone();
            if let Some(_) = editor.state.unenroll(&id) {
                let _ = editor.state.save();
                editor.status = Some(format!("disenrolled {}", id));
                let new_n = editor.state.enrollments.len();
                let new_cursor = if new_n == 0 { 0 } else { cursor.min(new_n - 1) };
                editor.mode = Mode::ManagingPrograms { cursor: new_cursor };
            }
        }
        _ => {}
    }
}

fn handle_catalog(editor: &mut Editor, code: KeyCode) {
    let Mode::PickingCatalog { mut cursor } = editor.mode.clone() else {
        return;
    };
    let entries: Vec<CatalogEntry> = editor.catalog.programs.clone();
    let n = entries.len();
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.mode = Mode::ManagingPrograms { cursor: 0 };
        }
        KeyCode::Up | KeyCode::Char('k') if n > 0 => {
            cursor = if cursor == 0 { n - 1 } else { cursor - 1 };
            editor.mode = Mode::PickingCatalog { cursor };
        }
        KeyCode::Down | KeyCode::Char('j') if n > 0 => {
            cursor = (cursor + 1) % n;
            editor.mode = Mode::PickingCatalog { cursor };
        }
        KeyCode::Enter | KeyCode::Char(' ') if n > 0 => {
            let entry = &entries[cursor];
            if editor.state.enrollment(&entry.id).is_some() {
                editor.status = Some(format!("already enrolled in {}", entry.display_name));
                return;
            }
            match entry.kind {
                CatalogKind::Cycle => {
                    editor.mode = Mode::PickingCycleStart {
                        program_id: entry.id.clone(),
                        choice: 0,
                        date_input: String::new(),
                    };
                }
                CatalogKind::Book => {
                    let catalog_root = entry.root_ref.clone().unwrap_or_default();
                    editor.mode = Mode::PickingBookOptions {
                        program_id: entry.id.clone(),
                        catalog_root,
                        catalog_unit: entry.unit,
                        field: 0,
                        starting_ref_input: String::new(),
                        unit_choice: BookUnitChoice::Auto,
                    };
                }
            }
        }
        _ => {}
    }
}

fn handle_cycle_start(editor: &mut Editor, code: KeyCode) {
    let Mode::PickingCycleStart {
        program_id,
        mut choice,
        mut date_input,
    } = editor.mode.clone()
    else {
        return;
    };
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.mode = Mode::PickingCatalog { cursor: 0 };
        }
        KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j') => {
            choice = 1 - choice;
            editor.mode = Mode::PickingCycleStart {
                program_id,
                choice,
                date_input,
            };
        }
        KeyCode::Char(c) if choice == 1 && (c.is_ascii_digit() || c == '-') => {
            date_input.push(c);
            editor.mode = Mode::PickingCycleStart {
                program_id,
                choice,
                date_input,
            };
        }
        KeyCode::Backspace if choice == 1 => {
            date_input.pop();
            editor.mode = Mode::PickingCycleStart {
                program_id,
                choice,
                date_input,
            };
        }
        KeyCode::Enter => {
            let start = if choice == 0 {
                CycleStart::Today
            } else {
                match NaiveDate::parse_from_str(&date_input, "%Y-%m-%d") {
                    Ok(d) => CycleStart::From(d),
                    Err(_) => {
                        editor.status =
                            Some("invalid date — use YYYY-MM-DD".to_string());
                        editor.mode = Mode::PickingCycleStart {
                            program_id,
                            choice,
                            date_input,
                        };
                        return;
                    }
                }
            };
            match enroll_cycle(&mut editor.state, &editor.catalog, &program_id, start) {
                Ok(()) => {
                    editor.status = Some(format!("enrolled in {}", program_id));
                    editor.mode = Mode::ManagingPrograms { cursor: 0 };
                }
                Err(e) => {
                    editor.status = Some(format!("error: {}", e));
                    editor.mode = Mode::PickingCatalog { cursor: 0 };
                }
            }
        }
        _ => {}
    }
}

fn handle_book_options(editor: &mut Editor, code: KeyCode) {
    let Mode::PickingBookOptions {
        program_id,
        catalog_root,
        catalog_unit,
        mut field,
        mut starting_ref_input,
        mut unit_choice,
    } = editor.mode.clone()
    else {
        return;
    };
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.mode = Mode::PickingCatalog { cursor: 0 };
            return;
        }
        KeyCode::Tab | KeyCode::Up | KeyCode::Down => {
            field = 1 - field;
        }
        KeyCode::Left | KeyCode::Char('h') if field == 1 => {
            unit_choice = unit_choice.cycle(-1);
        }
        KeyCode::Right | KeyCode::Char('l') if field == 1 => {
            unit_choice = unit_choice.cycle(1);
        }
        KeyCode::Char(c) if field == 0 && !c.is_control() => {
            starting_ref_input.push(c);
        }
        KeyCode::Backspace if field == 0 => {
            starting_ref_input.pop();
        }
        KeyCode::Enter => {
            let opts = BookOptions {
                starting_ref: if starting_ref_input.trim().is_empty() {
                    None
                } else {
                    Some(starting_ref_input.trim().to_string())
                },
                unit_override: unit_choice.resolve(),
            };
            match enroll_book_with_options(&mut editor.state, &editor.catalog, &program_id, opts)
            {
                Ok(()) => {
                    editor.status = Some(format!("enrolled in {}", program_id));
                    editor.mode = Mode::ManagingPrograms { cursor: 0 };
                    return;
                }
                Err(e) => {
                    editor.status = Some(format!("error: {}", e));
                }
            }
        }
        _ => {}
    }
    editor.mode = Mode::PickingBookOptions {
        program_id,
        catalog_root,
        catalog_unit,
        field,
        starting_ref_input,
        unit_choice,
    };
}

fn handle_history(editor: &mut Editor, code: KeyCode) {
    let Mode::ViewingHistory { mut cursor } = editor.mode.clone() else {
        return;
    };
    let n = editor.state.completion_log.len();
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.mode = Mode::ManagingPrograms { cursor: 0 };
        }
        KeyCode::Up | KeyCode::Char('k') if n > 0 => {
            cursor = if cursor == 0 { n - 1 } else { cursor - 1 };
            editor.mode = Mode::ViewingHistory { cursor };
        }
        KeyCode::Down | KeyCode::Char('j') if n > 0 => {
            cursor = (cursor + 1) % n;
            editor.mode = Mode::ViewingHistory { cursor };
        }
        KeyCode::PageDown if n > 0 => {
            cursor = (cursor + 10).min(n - 1);
            editor.mode = Mode::ViewingHistory { cursor };
        }
        KeyCode::PageUp => {
            cursor = cursor.saturating_sub(10);
            editor.mode = Mode::ViewingHistory { cursor };
        }
        _ => {}
    }
}

fn handle_library(editor: &mut Editor, code: KeyCode) {
    let Mode::BrowsingLibrary {
        mut stack,
        mut cursor,
    } = editor.mode.clone()
    else {
        return;
    };
    let Some(library) = editor.library.clone() else {
        editor.status = Some("library not loaded — check network".to_string());
        editor.mode = Mode::ManagingPrograms { cursor: 0 };
        return;
    };

    let current_nodes = walk_to(&library, &stack);
    let n = current_nodes.len();

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            if stack.is_empty() {
                editor.mode = Mode::ManagingPrograms { cursor: 0 };
            } else {
                stack.pop();
                editor.mode = Mode::BrowsingLibrary { stack, cursor: 0 };
            }
        }
        KeyCode::Up | KeyCode::Char('k') if n > 0 => {
            cursor = if cursor == 0 { n - 1 } else { cursor - 1 };
            editor.mode = Mode::BrowsingLibrary { stack, cursor };
        }
        KeyCode::Down | KeyCode::Char('j') if n > 0 => {
            cursor = (cursor + 1) % n;
            editor.mode = Mode::BrowsingLibrary { stack, cursor };
        }
        KeyCode::Enter | KeyCode::Char(' ') if n > 0 => {
            let node = &current_nodes[cursor];
            if !node.contents.is_empty() {
                stack.push(cursor);
                editor.mode = Mode::BrowsingLibrary { stack, cursor: 0 };
            } else if let Some(title) = node.title.clone() {
                let starting_ref = format!("{} 1", title);
                match enroll_custom_book(
                    &mut editor.state,
                    &title,
                    &starting_ref,
                    Unit::Section,
                ) {
                    Ok(()) => {
                        editor.status = Some(format!("enrolled in {}", title));
                        editor.mode = Mode::ManagingPrograms { cursor: 0 };
                    }
                    Err(e) => {
                        editor.status = Some(format!("error: {}", e));
                    }
                }
            }
        }
        _ => {}
    }
}

fn ensure_library_loaded(editor: &mut Editor) {
    if editor.library.is_some() {
        return;
    }
    let client = SefariaClient::default();
    match client.fetch_index_tree() {
        Ok(tree) => {
            editor.library = Some(tree);
        }
        Err(e) => {
            editor.status = Some(format!("library fetch failed: {}", e));
        }
    }
}

fn walk_to<'a>(library: &'a [IndexNode], stack: &[usize]) -> Vec<&'a IndexNode> {
    let mut current: Vec<&IndexNode> = library.iter().collect();
    for &i in stack {
        if let Some(node) = current.get(i) {
            current = node.contents.iter().collect();
        } else {
            return Vec::new();
        }
    }
    current
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
    match &editor.mode {
        Mode::Browsing => draw_field_browse(f, area, editor),
        Mode::ManagingPrograms { cursor } => draw_programs(f, area, editor, *cursor),
        Mode::PickingCatalog { cursor } => draw_catalog(f, area, editor, *cursor),
        Mode::PickingCycleStart {
            program_id,
            choice,
            date_input,
        } => draw_cycle_start(f, area, editor, program_id, *choice, date_input),
        Mode::PickingBookOptions {
            program_id,
            catalog_root,
            catalog_unit,
            field,
            starting_ref_input,
            unit_choice,
        } => draw_book_options(
            f,
            area,
            program_id,
            catalog_root,
            *catalog_unit,
            *field,
            starting_ref_input,
            *unit_choice,
        ),
        Mode::BrowsingLibrary { stack, cursor } => draw_library(f, area, editor, stack, *cursor),
        Mode::ViewingHistory { cursor } => draw_history(f, area, editor, *cursor),
    }
}

fn draw_field_browse(f: &mut Frame, area: Rect, editor: &Editor) {
    let columns = RLayout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);
    draw_field_list(f, columns[0], editor);
    draw_value_pane(f, columns[1], editor);
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
        "programs" => {
            let summary = if editor.state.enrollments.is_empty() {
                "(none)".to_string()
            } else {
                format!("{} active", editor.state.enrollments.len())
            };
            (
                summary,
                String::new(),
                "press enter to manage programs".to_string(),
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

fn draw_programs(f: &mut Frame, area: Rect, editor: &Editor, cursor: usize) {
    let chunks = RLayout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(8)])
        .split(area);

    let lines: Vec<Line> = if editor.state.enrollments.is_empty() {
        vec![Line::from(Span::styled(
            "(no enrollments — press 'a' to add from catalog or 'b' to browse Sefaria)",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        editor
            .state
            .enrollments
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let marker = if i == cursor { "▸ " } else { "  " };
                let style = if i == cursor {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let kind_label = match &e.kind {
                    EnrollmentKind::BookTrack { .. } => "book",
                    EnrollmentKind::CycleSubscription { .. } => "cycle",
                };
                let progress = if e.completed.is_empty() {
                    String::new()
                } else {
                    format!(" · {} done", e.completed.len())
                };
                Line::from(vec![Span::styled(
                    format!(
                        "{}{} · {} · {}{}",
                        marker, e.display_name, e.current_ref, kind_label, progress
                    ),
                    style,
                )])
            })
            .collect()
    };

    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" active programs "),
    );
    f.render_widget(p, chunks[0]);

    let archive_lines: Vec<Line> = if editor.state.archive.is_empty() {
        vec![Line::from(Span::styled(
            "(no completed programs yet)",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        editor
            .state
            .archive
            .iter()
            .map(|a| {
                Line::from(Span::styled(
                    format!(
                        "{} · completed {} · {} items",
                        a.display_name,
                        a.completed_at.format("%Y-%m-%d"),
                        a.item_count
                    ),
                    Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
                ))
            })
            .collect()
    };
    let p_arch = Paragraph::new(archive_lines)
        .block(Block::default().borders(Borders::ALL).title(" archive "));
    f.render_widget(p_arch, chunks[1]);
}

fn draw_catalog(f: &mut Frame, area: Rect, editor: &Editor, cursor: usize) {
    let lines: Vec<Line> = editor
        .catalog
        .programs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let marker = if i == cursor { "▸ " } else { "  " };
            let kind = match p.kind {
                CatalogKind::Cycle => "cycle",
                CatalogKind::Book => "book ",
            };
            let already = if editor.state.enrollment(&p.id).is_some() {
                " (enrolled)"
            } else {
                ""
            };
            let style = if i == cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if !already.is_empty() {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };
            Line::from(vec![Span::styled(
                format!("{}{} · {}{}", marker, kind, p.display_name, already),
                style,
            )])
        })
        .collect();
    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" catalog — enter to enroll, esc to return "),
    );
    f.render_widget(p, area);
}

fn draw_cycle_start(
    f: &mut Frame,
    area: Rect,
    editor: &Editor,
    program_id: &str,
    choice: usize,
    date_input: &str,
) {
    let _ = editor;
    let mark = |c: usize| if c == choice { "● " } else { "○ " };
    let lines = vec![
        Line::from(Span::styled(
            format!("Enroll in {} — pick start point:", program_id),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("{}Today", mark(0)),
            if choice == 0 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        )),
        Line::from(Span::styled(
            format!("{}Custom date: {}", mark(1), date_input),
            if choice == 1 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        )),
        Line::from(""),
        Line::from(Span::styled(
            "↑↓ to switch · type YYYY-MM-DD for custom · enter to confirm · esc to cancel",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )),
    ];
    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" cycle start "),
    );
    f.render_widget(p, area);
}

fn draw_book_options(
    f: &mut Frame,
    area: Rect,
    program_id: &str,
    catalog_root: &str,
    catalog_unit: Unit,
    field: usize,
    starting_ref_input: &str,
    unit_choice: BookUnitChoice,
) {
    let active = |idx: usize| {
        if field == idx {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    };
    let cursor_marker = |idx: usize| if field == idx { "▸ " } else { "  " };
    let starting_display = if starting_ref_input.is_empty() {
        format!("(default: {})", catalog_root)
    } else {
        starting_ref_input.to_string()
    };
    let unit_display = match unit_choice {
        BookUnitChoice::Auto => format!("Auto (= {})", unit_label(catalog_unit)),
        other => other.label().to_string(),
    };
    let lines = vec![
        Line::from(Span::styled(
            format!("Enroll in {} — book options:", program_id),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("{}Starting ref: {}", cursor_marker(0), starting_display),
            active(0),
        )),
        Line::from(Span::styled(
            "    type a Sefaria ref (e.g. \"Mishnah Berurah 3\") or leave blank for the start of the book",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("{}Granularity: {}", cursor_marker(1), unit_display),
            active(1),
        )),
        Line::from(Span::styled(
            "    Auto · Segment · Section / chapter · Daf  (←/→ to cycle)",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Tab/↑↓ switch field · enter to enroll · esc to cancel",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )),
    ];
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" book options "))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn unit_label(u: Unit) -> &'static str {
    match u {
        Unit::Segment => "Segment",
        Unit::Section => "Section",
        Unit::Daf => "Daf",
    }
}

fn draw_history(f: &mut Frame, area: Rect, editor: &Editor, cursor: usize) {
    if editor.state.completion_log.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(Span::styled(
                "No learning logged yet.",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press n in the popup to mark items complete; they'll show up here.",
                Style::default().add_modifier(Modifier::DIM),
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title(" history "));
        f.render_widget(p, area);
        return;
    }

    let lines: Vec<Line> = editor
        .state
        .completion_log
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let marker = if i == cursor { "▸ " } else { "  " };
            let style = if i == cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let when = e.at.with_timezone(&chrono::Local);
            Line::from(vec![Span::styled(
                format!(
                    "{}{} · {} · {}",
                    marker,
                    when.format("%Y-%m-%d %H:%M"),
                    e.program_name,
                    e.ref_
                ),
                style,
            )])
        })
        .collect();
    let title = format!(" history — {} entries ", editor.state.completion_log.len());
    let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(p, area);
}

fn draw_library(
    f: &mut Frame,
    area: Rect,
    editor: &Editor,
    stack: &[usize],
    cursor: usize,
) {
    let library = match &editor.library {
        Some(l) => l,
        None => {
            let p = Paragraph::new(Span::styled(
                "library not loaded — check network connection",
                Style::default().fg(Color::Red),
            ))
            .block(Block::default().borders(Borders::ALL).title(" sefaria "));
            f.render_widget(p, area);
            return;
        }
    };
    let nodes = walk_to(library, stack);
    let breadcrumb = breadcrumb_string(library, stack);
    let lines: Vec<Line> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let marker = if i == cursor { "▸ " } else { "  " };
            let label = n
                .category
                .clone()
                .or_else(|| n.title.clone())
                .unwrap_or_else(|| "(unnamed)".to_string());
            let leaf_marker = if n.contents.is_empty() { "" } else { "/" };
            let style = if i == cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(vec![Span::styled(
                format!("{}{}{}", marker, label, leaf_marker),
                style,
            )])
        })
        .collect();
    let title = if breadcrumb.is_empty() {
        " sefaria ".to_string()
    } else {
        format!(" sefaria › {} ", breadcrumb)
    };
    let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(p, area);
}

fn breadcrumb_string(library: &[IndexNode], stack: &[usize]) -> String {
    let mut current: &[IndexNode] = library;
    let mut parts = Vec::new();
    for &i in stack {
        if let Some(node) = current.get(i) {
            let label = node
                .category
                .clone()
                .or_else(|| node.title.clone())
                .unwrap_or_default();
            parts.push(label);
            current = &node.contents;
        } else {
            break;
        }
    }
    parts.join(" › ")
}

fn draw_footer(f: &mut Frame, area: Rect, editor: &Editor) {
    let hint = match editor.mode {
        Mode::Browsing => "↑↓ field · ←→ cycle · enter edit · s save · q quit",
        Mode::ManagingPrograms { .. } => {
            "↑↓ select · a add · b browse · h history · d disenroll · esc back"
        }
        Mode::PickingCatalog { .. } => "↑↓ move · enter enroll · esc back",
        Mode::PickingCycleStart { .. } => "↑↓ option · type date · enter confirm · esc back",
        Mode::PickingBookOptions { .. } => "Tab/↑↓ field · type ref · ←→ unit · enter confirm",
        Mode::BrowsingLibrary { .. } => "↑↓ move · enter open/enroll · esc up/back",
        Mode::ViewingHistory { .. } => "↑↓ scroll · PgUp/PgDn jump · esc back",
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
    fn walk_to_root_returns_top_level() {
        let lib = vec![IndexNode {
            category: Some("Tanakh".into()),
            ..Default::default()
        }];
        let nodes = walk_to(&lib, &[]);
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn walk_to_drills_into_contents() {
        let lib = vec![IndexNode {
            category: Some("Tanakh".into()),
            contents: vec![IndexNode {
                title: Some("Genesis".into()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let nodes = walk_to(&lib, &[0]);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title.as_deref(), Some("Genesis"));
    }
}
