# bisl-torah

> Learn Torah while your coding agent runs.

While Claude Code (or any AI coding agent) is processing your prompt, a small popup opens beside your terminal with a piece of daily learning from [Sefaria](https://www.sefaria.org) — a halacha, a mishnah, a piece of Tanya. When the agent finishes, the popup invites you to dismiss it. The time you would have spent watching a spinner becomes time spent learning.

The name plays on the Yiddish phrase *"a bisl Torah"* — "a little Torah." Instead of waiting time becoming *bitul torah* (Torah-study time wasted), it becomes a few moments of learning.

## Install

```sh
cargo install bisl-torah
```

Or grab a prebuilt binary from the [Releases page](https://github.com/ddusi/bisl-torah/releases).

> **Windows:** unsigned binary; SmartScreen may warn. Click "More info → Run anyway".
> **macOS:** unnotarized binary; run `xattr -d com.apple.quarantine /path/to/bisl-torah` once.

## Setup

```sh
bisl-torah init       # safely merges hooks into ~/.claude/settings.json
bisl-torah doctor     # validates the install
```

That's it. Open Claude Code, prompt it, and a popup appears.

To remove:
```sh
bisl-torah uninstall
```

## How it works

`bisl-torah init` adds two hooks to your Claude Code settings:

- `UserPromptSubmit` → spawns a popup running the TUI
- `Stop` → tells the popup the agent finished (you press any key to dismiss)

The popup picks a host based on what's available:

| Environment        | Display strategy           |
|--------------------|-----------------------------|
| Windows Terminal   | `wt -w 0 split-pane` (attached side pane) |
| tmux session       | `tmux display-popup` (floating popup) |
| Anywhere else      | New OS console window (Windows: `CREATE_NEW_CONSOLE`; Unix: detached child) |

## Configuration

Defaults live in `~/.bisl-torah/config.toml` (or `%APPDATA%\bisl-torah\config.toml` on Windows). `init` creates it.

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
| `v`       | Toggle Hebrew vowels (nikud) on/off          |
| `f`       | Toggle footnotes drawer                      |

## Hebrew display

Hebrew text is rendered using whichever font your terminal emulator is configured to use. Nikud (vowel marks) and te'amim (cantillation marks) are **combining characters** — they layer onto the base letter. If the marks look misaligned, fuzzy, or fall on the wrong letter, the most likely fix is the terminal font, not the app:

- **Recommended fonts**: SBL Hebrew, Ezra SIL, Taamey Frank CLM, Noto Sans Hebrew, Cardo.
- **Windows Terminal**: Settings → Profiles → Appearance → Font face.
- **iTerm2 / Terminal.app**: Profiles → Text → Font.
- **VS Code integrated terminal**: `terminal.integrated.fontFamily` in settings.

If your terminal still struggles, you can **strip the nikud** instead:

- In the viewer: press `v` to toggle vowels on/off.
- Persistent: set `nikud = false` in `~/.bisl-torah/config.toml`.
- One-off: `bisl-torah show --no-nikud` (or `--nikud` to force on).

## Diagnostics

```sh
bisl-torah doctor       # binary on PATH, hooks wired, Sefaria reachable, log tail
bisl-torah show         # render once to current terminal (debug)
bisl-torah --verbose ... # bumps log level to debug
```

Logs at `~/.bisl-torah/logs/bisl-torah.log` (daily rotation, 7-day retention).

## License

MIT.
