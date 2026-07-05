# swpui

<a href="https://asciinema.org/a/CVwE0m5aU0UKql9p"><img alt="swpui screencast" src="https://raw.githubusercontent.com/beeb/swpui/refs/heads/main/screenshot.png" /></a>

<div align="center">
  <a href="https://github.com/beeb/swpui"><img
      alt="github"
      src="https://img.shields.io/badge/github-beeb%2Fswpui-228b22?style=flat&logo=github"
      height="20"
  /></a>
  <a href="https://crates.io/crates/swpui"><img
      alt="crates.io"
      src="https://img.shields.io/crates/v/swpui.svg?style=flat&color=e37602&logo=rust"
      height="20"
  /></a>
  <a href="https://github.com/NixOS/nixpkgs/blob/nixpkgs-unstable/pkgs/by-name/sw/swpui/package.nix#L22"><img
      alt="nixpkgs"
      src="https://img.shields.io/badge/nixpkgs-swpui-5277C3?style=flat&logo=nixos&logoColor=white"
      height="20"
  /></a>
</div>

`swpui` (pronounced "swap UI") is a TUI utility to search and replace text, with a focus on ergonomics, speed and
case-awareness in source code.

Dual-licensed under MIT or Apache 2.0.

## Installation

#### Via `cargo`

```bash
cargo install swpui
```

#### Via [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall)

```bash
cargo binstall swpui
```

#### Via Nix (`nixpkgs-unstable`)

```bash
nix profile install nixpkgs#swpui
```

Or run without installing:

```bash
nix run nixpkgs#swpui
```

#### Pre-built binaries and install script

Head over to the [releases page](https://github.com/beeb/swpui/releases)!

## Usage

### Launch the TUI

```bash
$ swp
```

### Keybindings

#### Global

| Key                | Action                  |
| ------------------ | ----------------------- |
| `Tab`              | Next pane               |
| `Shift+Tab`        | Previous pane           |
| `Ctrl+r` / `Alt+r` | Cycle match mode        |
| `Ctrl+g` / `Alt+g` | Toggle glob filter view |
| `Ctrl+o` / `Alt+o` | Open options menu       |
| `Ctrl+c`           | Quit                    |

#### Options Menu

| Key                        | Action                  |
| -------------------------- | ----------------------- |
| `r`                        | Cycle match mode        |
| `h`                        | Toggle hidden files     |
| `g`                        | Toggle gitignored files |
| `Esc` / `Ctrl+o` / `Alt+o` | Close options menu      |

#### File List

| Key                     | Action                            |
| ----------------------- | --------------------------------- |
| `j` / `Down`            | Next file                         |
| `k` / `Up`              | Previous file                     |
| `l` / `Enter` / `Right` | Focus preview pane                |
| `s` / `Space`           | Skip all matches in file (toggle) |
| `f`                     | Apply replacement to file         |
| `a`                     | Apply replacement to all files    |
| `q`                     | Quit                              |

#### Preview

| Key                  | Action                               |
| -------------------- | ------------------------------------ |
| `j` / `Down`         | Next match                           |
| `k` / `Up`           | Previous match                       |
| `Space`              | Skip selected match (toggle)         |
| `Enter`              | Apply replacement for selected match |
| `h` / `Esc` / `Left` | Back to file list                    |
| `s`                  | Skip all matches in file (toggle)    |
| `f`                  | Apply replacement to file            |
| `q`                  | Quit                                 |

#### Input Panes

| Key   | Action          |
| ----- | --------------- |
| `Esc` | Focus file list |

### Capture groups

In regex mode, the replacement template can reference capture groups from the search pattern using `$0` through `$9`:

- `$0` expands to the entire match.
- `$1`-`$9` expand to the corresponding capture groups (parenthesized sub-expressions in the pattern).
- `$$` produces a literal `$`.
- References to groups that did not participate in the match expand to an empty string.

For example, searching for `(\w+)_(\w+)` and replacing with `$2_$1` swaps the two halves of each `snake_case` pair.

### Glob filters

Press `Ctrl+g` / `Alt+g` to switch to the glob filters view. The search and replace fields are hidden in favor of a
field for and "include" and "exclude" globs list. Each input takes a comma-separated list of globs with
[gitignore](https://git-scm.com/docs/gitignore) matching semantics (same as ripgrep's `-g` flag).

- **Include**: when non-empty, only files matching at least one include glob are searched.
- **Exclude**: files matching any exclude glob are never searched.

Filters combine with the hidden/gitignore options and `.swpignore` files (an include glob does not re-include an ignored
file). Active filters are shown in the status line at the bottom.

## Configuration

Default values for the options can be persisted in a `swpui.toml` (or `.swpui.toml`) file. On startup, `swpui` looks for
them in:

1. The user config directory (`~/.config/swpui/` on Linux and macOS, `%APPDATA%\beeb\swpui\` on Windows).
2. Each directory from the filesystem root down to the directory where `swpui` was launched.

Settings closer to the working directory override settings from parents, and any field omitted from a file falls back to
the value from the next layer up, ultimately defaulting to the built-in defaults shown below:

```toml
[options]
match-mode = "case-aware" # case-aware | literal | regex | regex-multiline
include-hidden = true
include-gitignored = false
include-globs = [] # e.g. ["src/**", "*.{rs,toml}"]
exclude-globs = [] # e.g. ["*_test.rs"]
```

### Custom ignore file

In addition to `.gitignore` and `.ignore` files, `swpui` honors `.swpignore` files. These use the same
[gitignore](https://git-scm.com/docs/gitignore) syntax and are scoped per-directory: a `.swpignore` placed in any
directory applies to that directory and its descendants.

## Features

- [x] Case-aware replacement
- [x] Regex support (incl. multiline)
- [x] Multithreaded search
- [x] Respects (git)ignore files
- [x] Batch actions
- [x] Capture groups replacement
- [x] Toggle hidden files
- [x] Toggle gitignored files
- [x] Custom `.swpignore` files
- [x] Mouse support
- [x] Glob to include/exclude files

## Credits

This tool was inspired by [serpl](https://github.com/yassinebridi/serpl), thanks for the great idea! It would also not
have been possible without the amazing work put into [ratatui](https://ratatui.rs/) and
[rat-widget](https://github.com/thscharler/rat-salsa), thank you! Finally, a massive thanks to the creator of
[ripgrep](https://github.com/burntsushi/ripgrep) for their awesome work on [ignore](https://crates.io/crates/ignore) and
[regex](https://crates.io/crates/regex).
