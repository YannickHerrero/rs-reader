# rs-reader

A small terminal light novel reader for [Novel-FR](https://novel-fr.net) and [Syosetu](https://ncode.syosetu.com), built with Rust and Ratatui.

## Features

- Search Novel-FR or Syosetu from the terminal
- Add a series to a local library
- Browse saved series, volumes and chapters
- Read chapters in a minimal scrolling TUI
- Save reading position and mark chapters complete near the end
- Cache fetched chapter text locally

## Install

```bash
cargo install --git https://github.com/YannickHerrero/rs-reader.git
```

## Run

From a checkout:

```bash
cargo run
```

Or after installing:

```bash
rs-reader             # French Novel-FR profile
rs-reader --jp        # Japanese Syosetu profile
rs-reader --fzf       # fzf-driven navigation, then normal reader
rs-reader --jp --fzf  # Japanese profile with fzf navigation
```

The French and Japanese libraries are separated. They are stored in SQLite under the platform data directory, e.g. `~/.local/share/rs-reader/fr/library.sqlite` and `~/.local/share/rs-reader/jp/library.sqlite` on Linux.

## Keybindings

```text
Library:  Enter open · / search · r refresh · q quit
Search:   empty query shows recommendations · Enter search · ↑/↓ select · A add/open · Esc back
Volumes:  Enter open volume · h hide/show read · o sort newest/oldest · r refresh metadata · Esc back
Chapters: Enter read · h hide/show read · o sort newest/oldest · Esc back
Reader:   Tab mode · Space next unit · j/k move · PgUp/PgDn · g/G top/bottom · n/p chapter · Esc back
```

This intentionally uses only one source and avoids browser automation, downloads, account sync, or image rendering.
