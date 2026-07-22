use std::cmp::Ordering;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChapterSort {
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeriesView {
    Volumes,
    Chapters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReaderMode {
    Normal,
    Paragraph,
    Sentence,
}

impl ReaderMode {
    fn next(self) -> Self {
        match self {
            Self::Normal => Self::Paragraph,
            Self::Paragraph => Self::Sentence,
            Self::Sentence => Self::Normal,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Paragraph => "paragraph",
            Self::Sentence => "sentence",
        }
    }
}

impl ChapterSort {
    fn label(self) -> &'static str {
        match self {
            Self::NewestFirst => "newest first",
            Self::OldestFirst => "oldest first",
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::NewestFirst => Self::OldestFirst,
            Self::OldestFirst => Self::NewestFirst,
        }
    }
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
    series_view: SeriesView,
    volumes: Vec<Option<f64>>,
    selected_volume: usize,
    chapters: Vec<LibraryChapter>,
    all_chapters: Vec<LibraryChapter>,
    chapter_sort: ChapterSort,
    chapter_progress: HashMap<String, Progress>,
    selected_chapter: usize,
    chapter_view_offset: usize,
    reader_title: String,
    reader_series_key: String,
    reader_chapter_key: String,
    reader_lines: Vec<String>,
    reader_paragraphs: Vec<String>,
    reader_sentences: Vec<String>,
    reader_scroll: usize,
    reader_unit_index: usize,
    reader_mode: ReaderMode,
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
            series_view: SeriesView::Chapters,
            volumes: Vec::new(),
            selected_volume: 0,
            chapters: Vec::new(),
            all_chapters: Vec::new(),
            chapter_sort: ChapterSort::OldestFirst,
            chapter_progress: HashMap::new(),
            selected_chapter: 0,
            chapter_view_offset: 0,
            reader_title: String::new(),
            reader_series_key: String::new(),
            reader_chapter_key: String::new(),
            reader_lines: Vec::new(),
            reader_paragraphs: Vec::new(),
            reader_sentences: Vec::new(),
            reader_scroll: 0,
            reader_unit_index: 0,
            reader_mode: ReaderMode::Normal,
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

    fn draw(&mut self, frame: &mut ratatui::Frame<'_>) {
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
            Screen::Series => match self.series_view {
                SeriesView::Volumes => {
                    "Enter open volume · o sort newest/oldest · r refresh metadata · Esc back"
                }
                SeriesView::Chapters => {
                    "Enter read · o sort newest/oldest · r refresh metadata · Esc back"
                }
            },
            Screen::Reader => "Tab mode · j/k move · PgUp/PgDn · g/G · n/p chapter · Esc back",
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

    fn draw_series(&mut self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let title = self
            .current_series
            .as_ref()
            .map(|series| format!("{} ({})", series.title, self.chapter_sort.label()))
            .unwrap_or_else(|| format!("Chapters ({})", self.chapter_sort.label()));
        if self.series_view == SeriesView::Volumes {
            self.draw_volumes(frame, area, title.as_str());
            return;
        }

        let visible_cards = ((area.height.saturating_sub(2) as usize) / 4).max(1);
        self.keep_selected_chapter_visible(visible_cards);
        let items = self
            .chapters
            .iter()
            .enumerate()
            .skip(self.chapter_view_offset)
            .take(visible_cards)
            .map(|(index, chapter)| {
                let selected = index == self.selected_chapter;
                let marker = if selected { "▸" } else { " " };
                let progress = self.chapter_progress.get(&chapter.key);
                let state = progress
                    .map(|progress| {
                        if progress.completed {
                            "completed"
                        } else {
                            "in progress"
                        }
                    })
                    .unwrap_or("unread");
                let number = effective_chapter_number(chapter)
                    .map(format_chapter_number)
                    .unwrap_or_else(|| "?".to_string());
                let released = chapter.published_at.as_deref().unwrap_or("unknown date");
                let accent = if selected {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{marker} Chapter {number}"), accent),
                        Span::raw(format!("  ·  {state}")),
                    ]),
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            strip_existing_chapter_prefix(&chapter.title).to_string(),
                            accent,
                        ),
                    ]),
                    Line::from(format!("  Released: {released}")),
                    Line::from(""),
                ])
            });
        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title(title.as_str())),
            area,
        );
    }

    fn draw_volumes(
        &mut self,
        frame: &mut ratatui::Frame<'_>,
        area: ratatui::layout::Rect,
        title: &str,
    ) {
        let visible_cards = ((area.height.saturating_sub(2) as usize) / 4).max(1);
        if self.selected_volume < self.chapter_view_offset {
            self.chapter_view_offset = self.selected_volume;
        } else if self.selected_volume >= self.chapter_view_offset + visible_cards {
            self.chapter_view_offset = self.selected_volume + 1 - visible_cards;
        }
        self.chapter_view_offset = self
            .chapter_view_offset
            .min(self.volumes.len().saturating_sub(visible_cards));

        let items = self
            .volumes
            .iter()
            .enumerate()
            .skip(self.chapter_view_offset)
            .take(visible_cards)
            .map(|(index, volume)| {
                let selected = index == self.selected_volume;
                let marker = if selected { "▸" } else { " " };
                let chapters = self.chapters_for_volume(*volume);
                let completed = chapters
                    .iter()
                    .filter(|chapter| {
                        self.chapter_progress
                            .get(&chapter.key)
                            .map(|progress| progress.completed)
                            .unwrap_or(false)
                    })
                    .count();
                let accent = if selected {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(vec![
                    Line::from(Span::styled(
                        format!("{marker} {}", volume_label(*volume)),
                        accent,
                    )),
                    Line::from(format!("  {} chapter(s)", chapters.len())),
                    Line::from(format!(
                        "  Progress: {completed}/{} completed",
                        chapters.len()
                    )),
                    Line::from(""),
                ])
            });
        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
            area,
        );
    }

    fn draw_reader(&self, frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect) {
        let has_counter = self.reader_mode != ReaderMode::Normal;
        let block = Block::default().borders(Borders::ALL).title(format!(
            "{} · {}",
            self.reader_title,
            self.reader_mode.label()
        ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(if has_counter {
                [Constraint::Min(1), Constraint::Length(1)]
            } else {
                [Constraint::Min(1), Constraint::Length(0)]
            })
            .split(inner);

        let (text, counter) = match self.reader_mode {
            ReaderMode::Normal => {
                let visible = chunks[0].height as usize;
                let end = self.reader_lines.len().min(self.reader_scroll + visible);
                let text = self.reader_lines[self.reader_scroll.min(end)..end].join("\n");
                (text, None)
            }
            ReaderMode::Paragraph => {
                let total = self.reader_paragraphs.len().max(1);
                let current = self.reader_unit_index.min(total - 1);
                let text = self
                    .reader_paragraphs
                    .get(current)
                    .cloned()
                    .unwrap_or_default();
                (text, Some(format!("paragraph {}/{}", current + 1, total)))
            }
            ReaderMode::Sentence => {
                let total = self.reader_sentences.len().max(1);
                let current = self.reader_unit_index.min(total - 1);
                let text = self
                    .reader_sentences
                    .get(current)
                    .cloned()
                    .unwrap_or_default();
                (text, Some(format!("sentence {}/{}", current + 1, total)))
            }
        };

        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), chunks[0]);
        if let Some(counter) = counter {
            frame.render_widget(Paragraph::new(counter), chunks[1]);
        }
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
                if self.series_view == SeriesView::Chapters && !self.volumes.is_empty() {
                    self.series_view = SeriesView::Volumes;
                    self.chapter_view_offset = 0;
                } else {
                    self.screen = Screen::Library;
                    self.reload_library()?;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_series_selection_down(),
            KeyCode::Up | KeyCode::Char('k') => self.move_series_selection_up(),
            KeyCode::Enter => self.activate_series_selection().await?,
            KeyCode::Char('o') => self.toggle_chapter_sort(),
            KeyCode::Char('r') => self.refresh_current_series().await?,
            _ => {}
        }
        Ok(())
    }

    async fn handle_reader_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.save_reader_progress()?;
                self.reload_chapter_progress()?;
                self.screen = Screen::Series;
            }
            KeyCode::Tab => self.cycle_reader_mode()?,
            KeyCode::Char(' ') => self.move_reader(1)?,
            KeyCode::Down | KeyCode::Char('j') => self.move_reader(1)?,
            KeyCode::Up | KeyCode::Char('k') => self.move_reader(-1)?,
            KeyCode::PageDown => self.move_reader(10)?,
            KeyCode::PageUp => self.move_reader(-10)?,
            KeyCode::Char('g') => self.move_reader_to_start()?,
            KeyCode::Char('G') => self.move_reader_to_end()?,
            KeyCode::Char('n') => self.open_relative_chapter(1).await?,
            KeyCode::Char('p') => self.open_relative_chapter(-1).await?,
            _ => {}
        }
        Ok(())
    }

    fn move_series_selection_down(&mut self) {
        match self.series_view {
            SeriesView::Volumes => move_down(&mut self.selected_volume, self.volumes.len()),
            SeriesView::Chapters => move_down(&mut self.selected_chapter, self.chapters.len()),
        }
    }

    fn move_series_selection_up(&mut self) {
        match self.series_view {
            SeriesView::Volumes => move_up(&mut self.selected_volume),
            SeriesView::Chapters => move_up(&mut self.selected_chapter),
        }
    }

    async fn activate_series_selection(&mut self) -> Result<()> {
        match self.series_view {
            SeriesView::Volumes => {
                self.open_selected_volume();
                Ok(())
            }
            SeriesView::Chapters => self.open_selected_chapter().await,
        }
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
        let Some(series_key) = self
            .current_series
            .as_ref()
            .map(|series| series.key.clone())
        else {
            return Ok(());
        };
        self.all_chapters = self.repo.chapters(&series_key)?;
        self.volumes = collect_volumes(&self.all_chapters, self.chapter_sort);
        clamp_index(&mut self.selected_volume, self.volumes.len());
        if self.volumes.is_empty() {
            self.series_view = SeriesView::Chapters;
            self.chapters = self.all_chapters.clone();
            self.sort_chapters(None);
        } else {
            self.series_view = SeriesView::Volumes;
            self.chapters.clear();
            self.chapter_view_offset = 0;
        }
        self.reload_chapter_progress()?;
        clamp_index(&mut self.selected_chapter, self.chapters.len());
        self.screen = Screen::Series;
        Ok(())
    }

    fn reload_chapter_progress(&mut self) -> Result<()> {
        let Some(series_key) = self
            .current_series
            .as_ref()
            .map(|series| series.key.clone())
        else {
            return Ok(());
        };
        self.chapter_progress = self
            .repo
            .progress_for_series(&series_key)?
            .into_iter()
            .map(|progress| (progress.chapter_key.clone(), progress))
            .collect();
        Ok(())
    }

    fn toggle_chapter_sort(&mut self) {
        self.chapter_sort = self.chapter_sort.toggled();
        self.volumes = collect_volumes(&self.all_chapters, self.chapter_sort);
        clamp_index(&mut self.selected_volume, self.volumes.len());
        if self.series_view == SeriesView::Chapters {
            self.sort_chapters(None);
        }
        self.status = format!("Sorted chapters {}.", self.chapter_sort.label());
    }

    fn open_selected_volume(&mut self) {
        let Some(volume) = self.volumes.get(self.selected_volume).copied() else {
            return;
        };
        self.chapters = self
            .all_chapters
            .iter()
            .filter(|chapter| same_volume(chapter.volume, volume))
            .cloned()
            .collect();
        self.selected_chapter = 0;
        self.chapter_view_offset = 0;
        self.sort_chapters(None);
        self.series_view = SeriesView::Chapters;
    }

    fn chapters_for_volume(&self, volume: Option<f64>) -> Vec<&LibraryChapter> {
        self.all_chapters
            .iter()
            .filter(|chapter| same_volume(chapter.volume, volume))
            .collect()
    }

    fn sort_chapters(&mut self, selected_key: Option<&str>) {
        self.chapters.sort_by(|left, right| {
            let number_order = compare_chapters_by_number(
                effective_chapter_number(left),
                effective_chapter_number(right),
                self.chapter_sort,
            );
            number_order.then_with(|| match self.chapter_sort {
                ChapterSort::NewestFirst => left.position.cmp(&right.position),
                ChapterSort::OldestFirst => right.position.cmp(&left.position),
            })
        });
        if let Some(selected_key) = selected_key {
            if let Some(index) = self
                .chapters
                .iter()
                .position(|chapter| chapter.key == selected_key)
            {
                self.selected_chapter = index;
            }
        }
        clamp_index(&mut self.selected_chapter, self.chapters.len());
    }

    fn keep_selected_chapter_visible(&mut self, visible_cards: usize) {
        if self.chapters.is_empty() {
            self.chapter_view_offset = 0;
            return;
        }
        let visible_cards = visible_cards.max(1);
        if self.selected_chapter < self.chapter_view_offset {
            self.chapter_view_offset = self.selected_chapter;
        } else if self.selected_chapter >= self.chapter_view_offset + visible_cards {
            self.chapter_view_offset = self.selected_chapter + 1 - visible_cards;
        }
        self.chapter_view_offset = self
            .chapter_view_offset
            .min(self.chapters.len().saturating_sub(visible_cards));
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
        let reader_text = reflow_reader_text(&text);
        self.reader_title = title;
        self.reader_series_key = chapter.series_key.clone();
        self.reader_chapter_key = chapter.key.clone();
        self.reader_lines = reader_text.lines().map(str::to_string).collect();
        self.reader_paragraphs = split_paragraphs(&reader_text);
        self.reader_sentences = split_sentences(&reader_text);
        let saved_progress = self.repo.progress(&chapter.key)?;
        self.reader_scroll = saved_progress
            .as_ref()
            .map(|progress| progress.scroll_line.max(0) as usize)
            .unwrap_or(0)
            .min(self.reader_lines.len().saturating_sub(1));
        let ratio = saved_progress
            .as_ref()
            .map(|progress| progress.scroll_ratio.clamp(0.0, 1.0))
            .unwrap_or(0.0);
        self.reader_unit_index = unit_index_from_ratio(self.current_reader_units_len(), ratio);
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

    fn cycle_reader_mode(&mut self) -> Result<()> {
        let ratio = self.current_reader_ratio();
        self.reader_mode = self.reader_mode.next();
        self.reader_unit_index = unit_index_from_ratio(self.current_reader_units_len(), ratio);
        self.reader_scroll = unit_index_from_ratio(self.reader_lines.len(), ratio);
        self.status = format!("Reader mode: {}.", self.reader_mode.label());
        self.save_reader_progress()
    }

    fn move_reader(&mut self, delta: isize) -> Result<()> {
        match self.reader_mode {
            ReaderMode::Normal => {
                let max = self.reader_lines.len().saturating_sub(1) as isize;
                self.set_reader_scroll((self.reader_scroll as isize + delta).clamp(0, max) as usize)
            }
            ReaderMode::Paragraph | ReaderMode::Sentence => {
                let max = self.current_reader_units_len().saturating_sub(1) as isize;
                self.reader_unit_index =
                    (self.reader_unit_index as isize + delta).clamp(0, max) as usize;
                self.reader_scroll =
                    unit_index_from_ratio(self.reader_lines.len(), self.current_reader_ratio());
                self.save_reader_progress()
            }
        }
    }

    fn move_reader_to_start(&mut self) -> Result<()> {
        self.reader_scroll = 0;
        self.reader_unit_index = 0;
        self.save_reader_progress()
    }

    fn move_reader_to_end(&mut self) -> Result<()> {
        self.reader_scroll = self.reader_lines.len().saturating_sub(1);
        self.reader_unit_index = self.current_reader_units_len().saturating_sub(1);
        self.save_reader_progress()
    }

    fn set_reader_scroll(&mut self, scroll: usize) -> Result<()> {
        self.reader_scroll = scroll.min(self.reader_lines.len().saturating_sub(1));
        self.reader_unit_index =
            unit_index_from_ratio(self.current_reader_units_len(), self.current_reader_ratio());
        self.save_reader_progress()
    }

    fn current_reader_units_len(&self) -> usize {
        match self.reader_mode {
            ReaderMode::Normal => self.reader_lines.len(),
            ReaderMode::Paragraph => self.reader_paragraphs.len(),
            ReaderMode::Sentence => self.reader_sentences.len(),
        }
    }

    fn current_reader_ratio(&self) -> f64 {
        match self.reader_mode {
            ReaderMode::Normal => ratio_for_index(self.reader_scroll, self.reader_lines.len()),
            ReaderMode::Paragraph | ReaderMode::Sentence => {
                ratio_for_index(self.reader_unit_index, self.current_reader_units_len())
            }
        }
    }

    fn save_reader_progress(&self) -> Result<()> {
        if self.reader_chapter_key.is_empty() || self.reader_lines.is_empty() {
            return Ok(());
        }
        let ratio = self.current_reader_ratio();
        self.repo.save_progress(
            &self.reader_series_key,
            &self.reader_chapter_key,
            self.reader_scroll as i64,
            ratio,
            ratio >= 0.95,
        )
    }
}

fn reflow_reader_text(text: &str) -> String {
    split_paragraphs(text).join("\n\n")
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let paragraphs = text
        .split("\n\n")
        .map(collapse_whitespace)
        .filter(|paragraph| !paragraph.is_empty())
        .collect::<Vec<_>>();
    if paragraphs.is_empty() {
        text.lines()
            .map(collapse_whitespace)
            .filter(|line| !line.is_empty())
            .collect()
    } else {
        paragraphs
    }
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        current.push(ch);

        if is_sentence_terminal(ch) && should_split_sentence(&current, &chars, index) {
            while let Some(next) = chars.get(index + 1).copied() {
                if !is_closing_punctuation(next) {
                    break;
                }
                current.push(next);
                index += 1;
            }

            let sentence = current.trim();
            if !sentence.is_empty() {
                sentences.push(collapse_whitespace(sentence));
            }
            current.clear();
        }
        index += 1;
    }

    let rest = current.trim();
    if !rest.is_empty() {
        sentences.push(collapse_whitespace(rest));
    }
    if sentences.is_empty() {
        split_paragraphs(text)
    } else {
        sentences
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_sentence_terminal(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?' | '…')
}

fn is_closing_punctuation(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '”' | '’' | '»' | ')' | ']' | '}')
}

fn should_split_sentence(current: &str, chars: &[char], index: usize) -> bool {
    let ch = chars[index];
    if ch == '.' {
        if is_decimal_point(chars, index) || ends_with_abbreviation(current) {
            return false;
        }
    }

    chars[index + 1..]
        .iter()
        .copied()
        .find(|ch| !is_closing_punctuation(*ch))
        .is_none_or(char::is_whitespace)
}

fn is_decimal_point(chars: &[char], index: usize) -> bool {
    index > 0
        && index + 1 < chars.len()
        && chars[index - 1].is_ascii_digit()
        && chars[index + 1].is_ascii_digit()
}

fn ends_with_abbreviation(current: &str) -> bool {
    let word = current
        .trim_end()
        .trim_end_matches('.')
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .trim_matches(|ch: char| !ch.is_alphabetic())
        .to_lowercase();
    matches!(
        word.as_str(),
        "m" | "mr" | "mrs" | "ms" | "mme" | "mlle" | "dr" | "prof" | "st" | "ste"
    )
}

fn ratio_for_index(index: usize, len: usize) -> f64 {
    let max = len.saturating_sub(1);
    if max == 0 {
        1.0
    } else {
        index.min(max) as f64 / max as f64
    }
}

fn unit_index_from_ratio(len: usize, ratio: f64) -> usize {
    let max = len.saturating_sub(1);
    ((ratio.clamp(0.0, 1.0) * max as f64).round() as usize).min(max)
}

fn collect_volumes(chapters: &[LibraryChapter], sort: ChapterSort) -> Vec<Option<f64>> {
    let has_unvolumed_chapters = chapters.iter().any(|chapter| chapter.volume.is_none());
    let mut volumes = chapters
        .iter()
        .filter_map(|chapter| chapter.volume)
        .collect::<Vec<_>>();
    volumes.sort_by(|left, right| match sort {
        ChapterSort::NewestFirst => right.partial_cmp(left).unwrap_or(Ordering::Equal),
        ChapterSort::OldestFirst => left.partial_cmp(right).unwrap_or(Ordering::Equal),
    });
    volumes.dedup_by(|left, right| (*left - *right).abs() < f64::EPSILON);
    let mut volumes = volumes.into_iter().map(Some).collect::<Vec<_>>();
    if has_unvolumed_chapters {
        match sort {
            ChapterSort::OldestFirst => volumes.insert(0, None),
            ChapterSort::NewestFirst => volumes.push(None),
        }
    }
    volumes
}

fn same_volume(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => (left - right).abs() < f64::EPSILON,
        (None, None) => true,
        _ => false,
    }
}

fn volume_label(volume: Option<f64>) -> String {
    volume
        .map(|volume| format!("Volume {}", format_chapter_number(volume)))
        .unwrap_or_else(|| "Prologue / Extras".to_string())
}

fn effective_chapter_number(chapter: &LibraryChapter) -> Option<f64> {
    chapter
        .number
        .or_else(|| chapter_number_from_text(&chapter.title))
        .or_else(|| chapter_number_from_text(&chapter.key))
}

fn chapter_number_from_text(value: &str) -> Option<f64> {
    let lower = value.to_lowercase().replace(',', ".");
    ["chapitre", "chap.", "chap", "ch.", "ch", "chapter"]
        .iter()
        .filter_map(|marker| {
            lower
                .find(marker)
                .map(|index| &lower[index + marker.len()..])
        })
        .find_map(first_decimal_number)
}

fn first_decimal_number(value: &str) -> Option<f64> {
    value
        .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .find_map(|part| {
            (!part.is_empty() && part.chars().any(|ch| ch.is_ascii_digit()))
                .then(|| part.parse::<f64>().ok())
                .flatten()
        })
}

fn strip_existing_chapter_prefix(title: &str) -> &str {
    let trimmed = title.trim();
    let lower = trimmed.to_lowercase();
    let Some(rest) = lower.strip_prefix("chapitre ") else {
        return trimmed;
    };
    let consumed = rest
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit() || *ch == '.' || *ch == ',')
        .last()
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    let original_rest = &trimmed["Chapitre ".len() + consumed..];
    original_rest
        .trim_start_matches(|ch: char| ch.is_whitespace() || ch == '·' || ch == '-' || ch == ':')
        .trim()
}

fn format_chapter_number(number: f64) -> String {
    if number.fract() == 0.0 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

fn compare_chapters_by_number(
    left: Option<f64>,
    right: Option<f64>,
    sort: ChapterSort,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => match sort {
            ChapterSort::NewestFirst => right.partial_cmp(&left).unwrap_or(Ordering::Equal),
            ChapterSort::OldestFirst => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        },
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_common_abbreviations_inside_sentences() {
        assert_eq!(
            split_sentences("Mme. Layvin entra. Mr. Kane sourit."),
            vec!["Mme. Layvin entra.", "Mr. Kane sourit."]
        );
    }

    #[test]
    fn keeps_closing_quotes_with_sentence() {
        assert_eq!(
            split_sentences("Il dit: \"Bonjour.\" Elle hocha la tête."),
            vec!["Il dit: \"Bonjour.\"", "Elle hocha la tête."]
        );
    }

    #[test]
    fn reflows_aesthetic_line_breaks_inside_paragraphs() {
        assert_eq!(
            reflow_reader_text("Je ne sais pas comment\nvous l'expliquer.\n\nMais voilà."),
            "Je ne sais pas comment vous l'expliquer.\n\nMais voilà."
        );
    }
}
