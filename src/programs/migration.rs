//! v1 → v2 state migration: when a user upgrades from a state.json that has
//! only `invocation_count`, surface a one-time TUI prompt that proposes cycle
//! enrollments based on the categories they had in `config.toml`.

use std::io::{self, Stdout};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::config::Config;
use crate::programs::catalog::Catalog;
use crate::programs::enrollment::{enroll_cycle, CycleStart};
use crate::state::{State, CURRENT_SCHEMA_VERSION};

/// Map a category from old config.toml to suggested cycle program ids.
pub fn suggested_for_category(category: &str) -> &'static [&'static str] {
    match category.to_lowercase().as_str() {
        "halakhah" => &["halakhah-yomit", "daily-rambam"],
        "halakhah yomit" => &["halakhah-yomit"],
        "daily rambam" => &["daily-rambam"],
        "daily rambam (3 chapters)" => &["rambam-3-chapters"],
        "arukh hashulchan yomi" => &["arukh-hashulchan-yomi"],
        "mishnah" | "daily mishnah" => &["mishnah-yomi"],
        "chasidut" | "tanya yomi" => &["tanya-yomi"],
        "talmud" | "daf yomi" => &["daf-yomi"],
        "yerushalmi yomi" => &["yerushalmi-yomi"],
        "daf a week" => &["daf-a-week"],
        "tanakh" | "tanakh yomi" => &["tanakh-yomi"],
        "929" => &["929"],
        "parashat hashavua" => &["shnayim-mikra"],
        "haftarah" => &["haftarah"],
        "chok leyisrael" => &["chok-leyisrael"],
        "*" => &[
            "daf-yomi",
            "mishnah-yomi",
            "tanya-yomi",
            "halakhah-yomit",
        ],
        _ => &[],
    }
}

/// Compute the de-duped list of suggested program ids given the v1 categories.
pub fn suggested_ids(categories: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for cat in categories {
        for id in suggested_for_category(cat) {
            if !out.iter().any(|x| x == *id) {
                out.push((*id).to_string());
            }
        }
    }
    out
}

#[derive(Debug)]
struct Row {
    id: String,
    display_name: String,
    selected: bool,
}

/// Run the migration prompt. Returns `true` if the user confirmed (enrollments
/// written, schema bumped) or `false` if they skipped.
pub fn run_prompt(state: &mut State, cfg: &Config) -> Result<bool> {
    let catalog = Catalog::embedded()?;

    let suggested = suggested_ids(&cfg.categories);
    let mut rows: Vec<Row> = Vec::new();
    // Suggested rows first.
    for id in &suggested {
        if let Some(p) = catalog.find(id) {
            rows.push(Row {
                id: p.id.clone(),
                display_name: p.display_name.clone(),
                selected: true,
            });
        }
    }
    // All other cycle programs follow, unselected.
    for p in catalog.cycles() {
        if rows.iter().any(|r| r.id == p.id) {
            continue;
        }
        rows.push(Row {
            id: p.id.clone(),
            display_name: p.display_name.clone(),
            selected: false,
        });
    }

    let mut terminal = init_terminal().context("setting up migration terminal")?;
    let confirmed = run_loop(&mut terminal, &mut rows);
    restore_terminal(&mut terminal).ok();
    let confirmed = confirmed?;

    if confirmed {
        let chosen: Vec<String> = rows
            .into_iter()
            .filter(|r| r.selected)
            .map(|r| r.id)
            .collect();
        for id in &chosen {
            if let Err(e) = enroll_cycle(state, &catalog, id, CycleStart::Today) {
                tracing::warn!(id = %id, error = %e, "migration enroll failed");
            }
        }
    }
    state.schema_version = CURRENT_SCHEMA_VERSION;
    state.migration_done = true;
    state.save()?;
    Ok(confirmed)
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    rows: &mut Vec<Row>,
) -> Result<bool> {
    let mut cursor: usize = 0;
    loop {
        terminal.draw(|f| draw(f, rows, cursor))?;
        let evt = event::read()?;
        let Event::Key(k) = evt else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c')) {
            return Ok(false);
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
            KeyCode::Up | KeyCode::Char('k') => {
                if !rows.is_empty() {
                    cursor = if cursor == 0 { rows.len() - 1 } else { cursor - 1 };
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !rows.is_empty() {
                    cursor = (cursor + 1) % rows.len();
                }
            }
            KeyCode::Char(' ') => {
                if let Some(r) = rows.get_mut(cursor) {
                    r.selected = !r.selected;
                }
            }
            KeyCode::Enter => return Ok(true),
            _ => {}
        }
    }
}

fn draw(f: &mut Frame, rows: &[Row], cursor: usize) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);
    draw_header(f, chunks[0]);
    draw_list(f, chunks[1], rows, cursor);
    draw_footer(f, chunks[2]);
}

fn draw_header(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "Welcome to bisl-torah programs",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::raw(
            "We've redesigned how learning works. Pick the programs you want to follow:",
        )),
    ];
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::BOTTOM))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, area: Rect, rows: &[Row], cursor: usize) {
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let marker = if i == cursor { "▸ " } else { "  " };
            let box_ = if r.selected { "[x]" } else { "[ ]" };
            let style = if i == cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if r.selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            Line::from(vec![Span::styled(
                format!("{}{} {}", marker, box_, r.display_name),
                style,
            )])
        })
        .collect();
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" cycle programs "));
    f.render_widget(p, area);
}

fn draw_footer(f: &mut Frame, area: Rect) {
    let p = Paragraph::new(Span::styled(
        "↑↓ move · space toggle · enter confirm · esc skip",
        Style::default().add_modifier(Modifier::DIM),
    ));
    f.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_handles_known_categories() {
        assert!(suggested_for_category("Halakhah").contains(&"halakhah-yomit"));
        assert!(suggested_for_category("Mishnah").contains(&"mishnah-yomi"));
        assert!(suggested_for_category("Chasidut").contains(&"tanya-yomi"));
        assert!(suggested_for_category("Talmud").contains(&"daf-yomi"));
    }

    #[test]
    fn suggested_handles_case_insensitive() {
        assert_eq!(
            suggested_for_category("HALAKHAH"),
            suggested_for_category("Halakhah")
        );
    }

    #[test]
    fn suggested_unknown_category_returns_empty() {
        assert!(suggested_for_category("NotARealCategory").is_empty());
    }

    #[test]
    fn suggested_ids_dedupes() {
        let ids = suggested_ids(&[
            "Halakhah".into(),
            "Halakhah Yomit".into(), // also resolves to halakhah-yomit
        ]);
        let count = ids.iter().filter(|x| *x == "halakhah-yomit").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn suggested_ids_for_default_categories_includes_all_three() {
        let ids = suggested_ids(&["Halakhah".into(), "Mishnah".into(), "Chasidut".into()]);
        assert!(ids.contains(&"halakhah-yomit".to_string()));
        assert!(ids.contains(&"mishnah-yomi".to_string()));
        assert!(ids.contains(&"tanya-yomi".to_string()));
    }
}
