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
    let title_he_raw = if app.item.display_value.he.is_empty() {
        app.item.title.he.clone()
    } else {
        app.item.display_value.he.clone()
    };
    let title_he = bidi::visual_order(&title_he_raw);

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
    let (en_text, he_text) = render_bilingual(&app.text, app.cfg.nikud);

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
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title));
    let alignment = if rtl {
        ratatui::layout::Alignment::Right
    } else {
        ratatui::layout::Alignment::Left
    };
    let p = Paragraph::new(lines.to_vec())
        .block(block)
        .wrap(Wrap { trim: false })
        .alignment(alignment)
        .scroll((scroll, 0));
    f.render_widget(p, area);
}

fn render_bilingual(text: &RefText, nikud: bool) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let mut en_lines: Vec<Line<'static>> = Vec::new();
    for (i, seg) in text.en.iter().enumerate() {
        let seg = format!("{}. {}", i + 1, seg);
        let r = markup::render_segment(&seg);
        en_lines.extend(r.lines);
        en_lines.push(Line::from(""));
    }
    let mut he_lines: Vec<Line<'static>> = Vec::new();
    for (i, seg) in text.he.iter().enumerate() {
        let working = if nikud {
            seg.clone()
        } else {
            bidi::strip_nikud(seg)
        };
        let prefixed = format!("{}. {}", i + 1, working);
        let r = markup::render_segment(&prefixed);
        for line in r.lines {
            let visual: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|s| {
                    let style = s.style;
                    let visual = bidi::visual_order(&s.content);
                    Span::styled(visual, style)
                })
                .collect();
            he_lines.push(Line::from(visual));
        }
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

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let display = if app.item.display_value.en.is_empty() {
        app.item.ref_.clone()
    } else {
        app.item.display_value.en.clone()
    };
    let mut left = format!(" {} · {}", display, app.item.category);
    if app.refresh_pending {
        left.push_str(" · new prompt — press n");
    }
    let right = match app.done {
        DoneState::Running => "q quit · n next · b/h/e lang · f footnotes · j/k scroll".to_string(),
        DoneState::AgentDone => "agent done — press any key".to_string(),
    };

    let columns = RLayout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let style_left = match app.done {
        DoneState::Running => Style::default(),
        DoneState::AgentDone => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    };

    let p_left = Paragraph::new(Span::styled(left, style_left));
    let p_right = Paragraph::new(Span::styled(
        right,
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
