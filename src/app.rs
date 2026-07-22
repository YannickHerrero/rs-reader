use std::collections::HashMap;
use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::db::{LibraryChapter, LibraryRepository, LibrarySeries, Progress};
use crate::source::{NovelFrSource, SearchResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Library,
    Search,
    Series,
    Reader,
}

pub struct App {
    repo: LibraryRepository,
    source: NovelFrSource,
    screen: Screen,
    library: Vec<LibrarySeries>,
    library_progress: HashMap<String, (usize, usize)>,
    selected_library: usize,
    search_query: String,
    search_results: Vec<SearchResult>,
    selected_search: usize,
    current_series: Option<LibrarySeries>,
    chapters: Vec<LibraryChapter>,
    chapter_progress: HashMap<String, Progress>,
    selected_chapter: usize,
    reader_title: String,
    reader_series_key: String,
    reader_chapter_key: String,
    reader_lines: Vec<String>,
    reader_scroll: usize,
    status: String,
    should_quit: bool,
}

impl App {
    pub fn new(repo: LibraryRepository, source: NovelFrSource) -> Result<Self> {
        let mut app = Self {
            repo,
            source,
            screen: Screen::Library,
            library: Vec::new(),
            library_progress: HashMap::new(),
            selected_library: 0,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search: 0,
            current_series: None,
            chapters: Vec::new(),
            chapter_progress: HashMap::new(),
            selected_chapter: 0,
            reader_title: String::new(),
            reader_series_key: String::new(),
            reader_chapter_key: String::new(),
            reader_lines: Vec::new(),
            reader_scroll: 0,
            status: String::new(),
            should_quit: false,
        };
        app.reload_library()?;
        Ok(app)
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key).await?;
                }
            }
        }
        Ok(())
    }

    fn draw(&self, frame: &mut ratatui::Frame<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(frame.area());

        let title = match self.screen {
            Screen::Library => "rs-reader · Library",
            Screen::Search => "rs-reader · Search Novel-FR",
            Screen::Series => "rs-reader · Chapters",
            Screen::Reader => "rs-reader · Reader",
        };
        frame.render_widget(Paragraph::new(title), chunks[0]);

        match self.screen {
            Screen::Library => self.draw_library(frame, chunks[1]),
            Screen::Search => self.draw_search(frame, chunks[1]),
            Screen::Series => self.draw_series(frame, chunks[1]),
            Screen::Reader => self.draw_reader(frame, chunks[1]),
        }

        let help = match self.screen {
            Screen::Library => "Enter open · / search · r refresh · q quit",
            Screen::Search => "Type query · Enter search · A add/open · Esc back",
            Screen::Series => "Enter read · r refresh metadata · Esc back",
            Screen::Reader => "j/k scroll · PgUp/PgDn · g/G · n/p chapter · Esc back",
        };
        frame.render_widget(
            Paragraph::new(vec![Line::from(help), Line::from(self.status.as_str())]),
            chunks[2],
        );
    }

    fn draw_library(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        if self.library.is_empty() {
            frame.render_widget(
                Paragraph::new("Your library is empty. Press / to search Novel-FR.")
                    .block(Block::default().borders(Borders::ALL).title("Library")),
                area,
            );
            return;
        }
        let items = self.library.iter().enumerate().map(|(index, series)| {
            let marker = if index == self.selected_library {
                "> "
            } else {
                "  "
            };
            let (done, total) = self
                .library_progress
                .get(&series.key)
                .copied()
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::raw(marker),
                Span::styled(
                    series.title.clone(),
                    if index == self.selected_library {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::raw(format!("  [{done}/{total}]")),
            ]))
        });
        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title("Library")),
            area,
        );
    }

    fn draw_search(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(area);
        frame.render_widget(
            Paragraph::new(self.search_query.as_str())
                .block(Block::default().borders(Borders::ALL).title("Query")),
            chunks[0],
        );
        let items = self
            .search_results
            .iter()
            .enumerate()
            .map(|(index, result)| {
                let marker = if index == self.selected_search {
                    "> "
                } else {
                    "  "
                };
                ListItem::new(format!("{marker}{}", result.title))
            });
        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title("Results")),
            chunks[1],
        );
    }

    fn draw_series(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let title = self
            .current_series
            .as_ref()
            .map(|series| series.title.as_str())
            .unwrap_or("Chapters");
        let items = self.chapters.iter().enumerate().map(|(index, chapter)| {
            let marker = if index == self.selected_chapter {
                ">"
            } else {
                " "
            };
            let state = self
                .chapter_progress
                .get(&chapter.key)
                .map(|progress| if progress.completed { "✓" } else { "…" })
                .unwrap_or(" ");
            ListItem::new(format!("{marker} {state} {}", chapter.title))
        });
        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
            area,
        );
    }

    fn draw_reader(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let visible = area.height.saturating_sub(2) as usize;
        let end = self.reader_lines.len().min(self.reader_scroll + visible);
        let text = self.reader_lines[self.reader_scroll.min(end)..end].join("\n");
        frame.render_widget(
            Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(self.reader_title.as_str()),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return Ok(());
        }
        match self.screen {
            Screen::Library => self.handle_library_key(key).await,
            Screen::Search => self.handle_search_key(key).await,
            Screen::Series => self.handle_series_key(key).await,
            Screen::Reader => self.handle_reader_key(key).await,
        }
    }

    async fn handle_library_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('/') | KeyCode::Char('s') => {
                self.screen = Screen::Search;
                self.status.clear();
            }
            KeyCode::Char('r') => self.reload_library()?,
            KeyCode::Down | KeyCode::Char('j') => {
                move_down(&mut self.selected_library, self.library.len())
            }
            KeyCode::Up | KeyCode::Char('k') => move_up(&mut self.selected_library),
            KeyCode::Enter => self.open_selected_library_series()?,
            _ => {}
        }
        Ok(())
    }

    async fn handle_search_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Library,
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Char('A') if !self.search_results.is_empty() => {
                self.add_selected_search_result().await?
            }
            KeyCode::Enter => self.run_search().await?,
            KeyCode::Down => move_down(&mut self.selected_search, self.search_results.len()),
            KeyCode::Up => move_up(&mut self.selected_search),
            KeyCode::Char(ch) => self.search_query.push(ch),
            _ => {}
        }
        Ok(())
    }

    async fn handle_series_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.screen = Screen::Library;
                self.reload_library()?;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_down(&mut self.selected_chapter, self.chapters.len())
            }
            KeyCode::Up | KeyCode::Char('k') => move_up(&mut self.selected_chapter),
            KeyCode::Enter => self.open_selected_chapter().await?,
            KeyCode::Char('r') => self.refresh_current_series().await?,
            _ => {}
        }
        Ok(())
    }

    async fn handle_reader_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.save_reader_progress()?;
                self.open_current_series()?;
            }
            KeyCode::Down | KeyCode::Char('j') => self.scroll_reader(1)?,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_reader(-1)?,
            KeyCode::PageDown => self.scroll_reader(10)?,
            KeyCode::PageUp => self.scroll_reader(-10)?,
            KeyCode::Char('g') => self.set_reader_scroll(0)?,
            KeyCode::Char('G') => {
                self.set_reader_scroll(self.reader_lines.len().saturating_sub(1))?
            }
            KeyCode::Char('n') => self.open_relative_chapter(1).await?,
            KeyCode::Char('p') => self.open_relative_chapter(-1).await?,
            _ => {}
        }
        Ok(())
    }

    fn reload_library(&mut self) -> Result<()> {
        self.library = self.repo.list_series()?;
        self.library_progress.clear();
        for series in &self.library {
            let summary = self.repo.series_progress(&series.key)?;
            self.library_progress.insert(
                series.key.clone(),
                (summary.completed_count, summary.chapter_count),
            );
        }
        clamp_index(&mut self.selected_library, self.library.len());
        Ok(())
    }

    async fn run_search(&mut self) -> Result<()> {
        let query = self.search_query.trim();
        if query.is_empty() {
            self.status = "Enter a search query first.".to_string();
            return Ok(());
        }
        self.status = format!("Searching for {query}...");
        self.search_results = self.source.search(query).await?;
        self.selected_search = 0;
        self.status = format!("Found {} result(s).", self.search_results.len());
        Ok(())
    }

    async fn add_selected_search_result(&mut self) -> Result<()> {
        let Some(result) = self.search_results.get(self.selected_search).cloned() else {
            return Ok(());
        };
        self.status = format!("Fetching {}...", result.title);
        let series = self.source.series(&result.key).await?;
        self.repo.add_or_update_series(&series)?;
        self.status = format!("Added {}.", series.title);
        self.current_series = self.repo.get_series(&series.key)?;
        self.open_current_series()?;
        Ok(())
    }

    fn open_selected_library_series(&mut self) -> Result<()> {
        let Some(series) = self.library.get(self.selected_library).cloned() else {
            return Ok(());
        };
        self.current_series = Some(series);
        self.open_current_series()
    }

    fn open_current_series(&mut self) -> Result<()> {
        let Some(series) = &self.current_series else {
            return Ok(());
        };
        self.chapters = self.repo.chapters(&series.key)?;
        self.chapter_progress = self
            .repo
            .progress_for_series(&series.key)?
            .into_iter()
            .map(|progress| (progress.chapter_key.clone(), progress))
            .collect();
        clamp_index(&mut self.selected_chapter, self.chapters.len());
        self.screen = Screen::Series;
        Ok(())
    }

    async fn refresh_current_series(&mut self) -> Result<()> {
        let Some(series) = self.current_series.clone() else {
            return Ok(());
        };
        self.status = format!("Refreshing {}...", series.title);
        let fresh = self.source.series(&series.key).await?;
        self.repo.add_or_update_series(&fresh)?;
        self.current_series = self.repo.get_series(&series.key)?;
        self.open_current_series()?;
        self.status = "Series refreshed.".to_string();
        Ok(())
    }

    async fn open_selected_chapter(&mut self) -> Result<()> {
        let Some(chapter) = self.chapters.get(self.selected_chapter).cloned() else {
            return Ok(());
        };
        self.open_chapter(chapter).await
    }

    async fn open_chapter(&mut self, chapter: LibraryChapter) -> Result<()> {
        self.save_reader_progress().ok();
        let cached = self.repo.cached_chapter(&chapter.key)?;
        let (title, text) = if let Some(cached) = cached {
            (cached.title, cached.text)
        } else {
            self.status = format!("Fetching {}...", chapter.title);
            let content = self.source.chapter(&chapter.key).await?;
            self.repo.cache_chapter(&content)?;
            (content.title, content.text)
        };
        self.reader_title = title;
        self.reader_series_key = chapter.series_key.clone();
        self.reader_chapter_key = chapter.key.clone();
        self.reader_lines = text.lines().map(str::to_string).collect();
        self.reader_scroll = self
            .repo
            .progress(&chapter.key)?
            .map(|progress| progress.scroll_line.max(0) as usize)
            .unwrap_or(0)
            .min(self.reader_lines.len().saturating_sub(1));
        self.screen = Screen::Reader;
        self.status.clear();
        Ok(())
    }

    async fn open_relative_chapter(&mut self, offset: isize) -> Result<()> {
        self.save_reader_progress()?;
        let Some(index) = self
            .chapters
            .iter()
            .position(|chapter| chapter.key == self.reader_chapter_key)
        else {
            return Ok(());
        };
        let next = (index as isize + offset)
            .clamp(0, self.chapters.len().saturating_sub(1) as isize) as usize;
        self.selected_chapter = next;
        self.open_selected_chapter().await
    }

    fn scroll_reader(&mut self, delta: isize) -> Result<()> {
        let next = (self.reader_scroll as isize + delta)
            .clamp(0, self.reader_lines.len().saturating_sub(1) as isize)
            as usize;
        self.set_reader_scroll(next)
    }

    fn set_reader_scroll(&mut self, scroll: usize) -> Result<()> {
        self.reader_scroll = scroll;
        self.save_reader_progress()
    }

    fn save_reader_progress(&self) -> Result<()> {
        if self.reader_chapter_key.is_empty() || self.reader_lines.is_empty() {
            return Ok(());
        }
        let max = self.reader_lines.len().saturating_sub(1).max(1);
        let ratio = self.reader_scroll as f64 / max as f64;
        self.repo.save_progress(
            &self.reader_series_key,
            &self.reader_chapter_key,
            self.reader_scroll as i64,
            ratio,
            ratio >= 0.95,
        )
    }
}

fn move_down(index: &mut usize, len: usize) {
    if len > 0 {
        *index = (*index + 1).min(len - 1);
    }
}

fn move_up(index: &mut usize) {
    *index = index.saturating_sub(1);
}

fn clamp_index(index: &mut usize, len: usize) {
    if len == 0 {
        *index = 0;
    } else if *index >= len {
        *index = len - 1;
    }
}
