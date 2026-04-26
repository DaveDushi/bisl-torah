use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::error;

mod cache;
mod config;
mod content;
mod display;
mod doctor;
mod fallback;
mod hook;
mod install;
mod log;
mod paths;
mod sefaria;
mod signals;
mod state;
mod viewer;

use crate::config::Config;
use crate::install::Scope;
use crate::state::State;

#[derive(Parser)]
#[command(
    name = "bisl-torah",
    version,
    about = "Learn while your coding agent runs."
)]
struct Cli {
    /// Bump log level to debug (default warn).
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install Claude Code hooks (safe-merge into settings.json).
    Init {
        /// Install in the current project (.claude/settings.json) instead of global.
        #[arg(long)]
        project: bool,
    },
    /// Remove the hooks bisl-torah added.
    Uninstall {
        #[arg(long)]
        project: bool,
    },
    /// Validate the install: binary, settings, hooks, Sefaria reachability, log tail.
    Doctor,
    /// Render today's learning to the current terminal (debug or no-tmux fallback).
    Show {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_name = "DIR")]
        signal_dir: Option<PathBuf>,
        #[arg(long, value_enum)]
        display: Option<DisplayCli>,
    },
    /// Hook entry point: read JSON event from stdin and spawn the popup.
    HookOnPrompt,
    /// Hook entry point: read JSON event from stdin and signal the popup to soft-close.
    HookOnStop,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum DisplayCli {
    Auto,
    WtSplit,
    TmuxPopup,
    NewConsole,
}

impl From<DisplayCli> for config::DisplayMode {
    fn from(d: DisplayCli) -> Self {
        match d {
            DisplayCli::Auto => Self::Auto,
            DisplayCli::WtSplit => Self::WtSplit,
            DisplayCli::TmuxPopup => Self::TmuxPopup,
            DisplayCli::NewConsole => Self::NewConsole,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let _guard = log::init(cli.verbose).ok();

    let result = match cli.command {
        Cmd::Init { project } => cmd_init(scope(project)),
        Cmd::Uninstall { project } => cmd_uninstall(scope(project)),
        Cmd::Doctor => cmd_doctor(),
        Cmd::Show {
            session,
            signal_dir,
            display,
        } => cmd_show(session, signal_dir, display.map(Into::into)),
        Cmd::HookOnPrompt => hook::on_prompt(),
        Cmd::HookOnStop => hook::on_stop(),
    };

    if let Err(e) = result {
        error!("{:#}", e);
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}

fn scope(project: bool) -> Scope {
    if project {
        Scope::Project
    } else {
        Scope::Global
    }
}

fn cmd_init(scope: Scope) -> Result<()> {
    paths::ensure_dirs()?;
    let wrote_starter = Config::ensure_starter()?;
    let report = install::install(scope)?;
    println!(
        "settings: {}\n  UserPromptSubmit: {}\n  Stop:             {}",
        report.path.display(),
        outcome_label(report.prompt),
        outcome_label(report.stop)
    );
    if wrote_starter {
        let cfg_path = paths::config_path()?;
        println!("config: wrote starter at {}", cfg_path.display());
    }
    println!("\nrun `bisl-torah doctor` to validate.");
    Ok(())
}

fn outcome_label(o: install::EventOutcome) -> &'static str {
    match o {
        install::EventOutcome::Added => "added",
        install::EventOutcome::AlreadyPresent => "already present",
    }
}

fn cmd_uninstall(scope: Scope) -> Result<()> {
    let report = install::uninstall(scope)?;
    println!(
        "settings: {}\n  UserPromptSubmit: {}\n  Stop:             {}",
        report.path.display(),
        if report.removed_prompt {
            "removed"
        } else {
            "no managed entry"
        },
        if report.removed_stop {
            "removed"
        } else {
            "no managed entry"
        }
    );
    Ok(())
}

fn cmd_doctor() -> Result<()> {
    let report = doctor::run()?;
    doctor::print_report(&report);
    Ok(())
}

fn cmd_show(
    session: Option<String>,
    signal_dir: Option<PathBuf>,
    display_override: Option<config::DisplayMode>,
) -> Result<()> {
    paths::ensure_dirs()?;
    let mut cfg = Config::load().context("loading config")?;
    if let Some(d) = display_override {
        cfg.display = d;
    }

    let mut state = State::load().unwrap_or_default();
    let resolved = content::resolve_for_invocation(&cfg, state.invocation_count)?;
    state.bump();
    let _ = state.save();

    let session_obj = session.as_ref().map(|id| viewer::ui::Session {
        id: id.clone(),
        signal_dir: signal_dir
            .clone()
            .unwrap_or_else(|| paths::sessions_dir().unwrap_or_else(|_| PathBuf::from("."))),
    });

    // Build a `next` callback that re-picks based on a fresh state bump.
    let cfg_for_next = cfg.clone();
    let mut next_state = state.clone();
    let on_next: Box<viewer::ui::NextFn<'_>> =
        Box::new(move || content::next_for(&mut next_state, &cfg_for_next));

    let inputs = viewer::ui::Inputs {
        config: &cfg,
        item: &resolved.item,
        text: &resolved.text,
        session: session_obj.as_ref(),
        on_next: Some(on_next),
    };
    viewer::run(inputs)
}
