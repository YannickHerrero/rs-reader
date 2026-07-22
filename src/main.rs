mod app;
mod db;
mod source;

use std::io;

use anyhow::Result;
use app::App;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::db::LibraryRepository;
use crate::source::NovelFrSource;

#[tokio::main]
async fn main() -> Result<()> {
    let repo = LibraryRepository::open_default()?;
    let source = NovelFrSource::new()?;
    let mut app = App::new(repo, source)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = app.run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}
