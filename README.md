# rs-reader

A small terminal light novel reader for [Novel-FR](https://novel-fr.net), built with Rust and Ratatui.

## Features

- Search Novel-FR from the terminal
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
rs-reader
```

The library is stored in SQLite under the platform data directory, e.g. `~/.local/share/rs-reader/library.sqlite` on Linux.

## Keybindings

```text
Library:  Enter open · / search · r refresh · q quit
Search:   type query · Enter search · ↑/↓ select · A add/open · Esc back
Volumes:  Enter open volume · o sort newest/oldest · r refresh metadata · Esc back
Chapters: Enter read · o sort newest/oldest · Esc back
Reader:   j/k scroll · PgUp/PgDn · g/G top/bottom · n/p chapter · Esc back
```

This intentionally uses only one source and avoids browser automation, downloads, account sync, or image rendering.
