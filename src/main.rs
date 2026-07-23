mod app;
mod db;
mod fzf_mode;
mod source;

use std::io;

use anyhow::{Result, bail};
use app::{App, TextLayout};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::db::LibraryRepository;
use crate::source::{NovelFrSource, NovelSource, SyosetuSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cli {
    profile: Profile,
    fzf: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    Fr,
    Jp,
}

impl Profile {
    fn from_args() -> Result<Cli> {
        let mut profile = Self::Fr;
        let mut fzf = false;
        for arg in std::env::args().skip(1) {
            match arg.as_str() {
                "--fr" => profile = Self::Fr,
                "--jp" => profile = Self::Jp,
                "--fzf" => fzf = true,
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                unknown => bail!("unknown argument: {unknown}"),
            }
        }
        Ok(Cli { profile, fzf })
    }

    fn db_profile(self) -> &'static str {
        match self {
            Self::Fr => "fr",
            Self::Jp => "jp",
        }
    }

    fn text_layout(self) -> TextLayout {
        match self {
            Self::Fr => TextLayout::SpaceSeparated,
            Self::Jp => TextLayout::LineSeparated,
        }
    }

    fn source(self) -> Result<Box<dyn NovelSource>> {
        match self {
            Self::Fr => Ok(Box::new(NovelFrSource::new()?)),
            Self::Jp => Ok(Box::new(SyosetuSource::new()?)),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Profile::from_args()?;
    let repo = LibraryRepository::open_profile(cli.profile.db_profile())?;
    let source = cli.profile.source()?;
    if cli.fzf {
        return fzf_mode::run(repo, source, cli.profile.text_layout()).await;
    }
    let mut app = App::new(repo, source, cli.profile.text_layout())?;

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

fn print_help() {
    println!("rs-reader");
    println!();
    println!("Usage: rs-reader [--fr|--jp] [--fzf]");
    println!();
    println!("  --fr    Use Novel-FR with the French library (default)");
    println!("  --jp    Use Syosetu with a separate Japanese library");
    println!("  --fzf   Use fzf-driven navigation before opening the reader");
}
