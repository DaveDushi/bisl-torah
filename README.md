# bitul-torah

> Learn while your coding agent runs.

While Claude Code (or any AI coding agent) is processing your prompt, a small popup opens beside your terminal with a piece of daily learning from [Sefaria](https://www.sefaria.org) — a halacha, a mishnah, a piece of Tanya. When the agent finishes, the popup invites you to dismiss it. The time you would have spent watching a spinner becomes time spent learning.

The name is a play on the Yiddish/Hebrew concept of *bitul torah* (time wasted that could have been Torah study).

## Install

```sh
cargo install bitul-torah
```

Or grab a prebuilt binary from the [Releases page](https://github.com/ddusi/bitul-torah/releases).

> **Windows:** unsigned binary; SmartScreen may warn. Click "More info → Run anyway".
> **macOS:** unnotarized binary; run `xattr -d com.apple.quarantine /path/to/bitul-torah` once.

## Setup

```sh
bitul-torah init       # safely merges hooks into ~/.claude/settings.json
bitul-torah doctor     # validates the install
```

That's it. Open Claude Code, prompt it, and a popup appears.

To remove:
```sh
bitul-torah uninstall
```

## How it works

`bitul-torah init` adds two hooks to your Claude Code settings:

- `UserPromptSubmit` → spawns a popup running the TUI
- `Stop` → tells the popup the agent finished (you press any key to dismiss)

The popup picks a host based on what's available:

| Environment        | Display strategy           |
|--------------------|-----------------------------|
| Windows Terminal   | `wt -w 0 split-pane` (attached side pane) |
| tmux session       | `tmux display-popup` (floating popup) |
| Anywhere else      | New OS console window (Windows: `CREATE_NEW_CONSOLE`; Unix: detached child) |

## Configuration

Defaults live in `~/.bitul-torah/config.toml` (or `%APPDATA%\bitul-torah\config.toml` on Windows). `init` creates it.

```toml
# Sefaria daily-calendar categories to rotate through. Use ["*"] for all.
categories = ["Halakhah", "Mishnah", "Chasidut"]

# Layout: "auto" picks side-by-side when wide, stacked when narrow.
layout = "auto"

# Initial language pane: "both" | "hebrew" | "english"
default_lang = "both"

# Keep Hebrew vowel marks (nikud).
nikud = true

# Display host: "auto" | "wt-split" | "tmux-popup" | "new-console"
display = "auto"
```

## Keybinds

| Key       | Action                                       |
|-----------|----------------------------------------------|
| `q`       | Close popup                                  |
| `n`       | Next item (rotate within whitelist)          |
| `j` / `↓` | Scroll down                                  |
| `k` / `↑` | Scroll up                                    |
| `g` / `G` | Jump to top / bottom                         |
| `b`       | Show both Hebrew and English                 |
| `h`       | Hebrew only                                  |
| `e`       | English only                                 |
| `f`       | Toggle footnotes drawer                      |

## Diagnostics

```sh
bitul-torah doctor       # binary on PATH, hooks wired, Sefaria reachable, log tail
bitul-torah show         # render once to current terminal (debug)
bitul-torah --verbose ... # bumps log level to debug
```

Logs at `~/.bitul-torah/logs/bitul-torah.log` (daily rotation, 7-day retention).

## License

MIT OR Apache-2.0.
