use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::{App, TextLayout};
use crate::db::{LibraryChapter, LibraryRepository, LibrarySeries};
use crate::source::{NovelSource, SearchResult};

pub async fn run(
    mut repo: LibraryRepository,
    source: Box<dyn NovelSource>,
    text_layout: TextLayout,
) -> Result<()> {
    ensure_fzf()?;

    loop {
        let Some(choice) = fzf_select("rs-reader> ", &["Library", "Search", "Quit"])? else {
            return Ok(());
        };
        match choice.as_str() {
            "Library" => {
                if let Some(chapter) = pick_library_chapter(&repo)? {
                    open_reader(repo, source, text_layout, chapter).await?;
                    return Ok(());
                }
            }
            "Search" => {
                if let Some(chapter) =
                    search_add_and_pick_chapter(&mut repo, source.as_ref()).await?
                {
                    open_reader(repo, source, text_layout, chapter).await?;
                    return Ok(());
                }
            }
            "Quit" => return Ok(()),
            _ => {}
        }
    }
}

async fn search_add_and_pick_chapter(
    repo: &mut LibraryRepository,
    source: &dyn NovelSource,
) -> Result<Option<LibraryChapter>> {
    let query = fzf_query("Search query> ")?;
    let results = if query.trim().is_empty() {
        source.recommendations().await?
    } else {
        source.search(query.trim()).await?
    };
    let Some(result) = pick_search_result(&results)? else {
        return Ok(None);
    };
    let series = source.series(&result.key).await?;
    repo.add_or_update_series(&series)?;
    let Some(saved) = repo.get_series(&series.key)? else {
        return Ok(None);
    };
    pick_chapter(repo, &saved)
}

fn pick_library_chapter(repo: &LibraryRepository) -> Result<Option<LibraryChapter>> {
    let library = repo.list_series()?;
    let Some(series) = pick_series(&library)? else {
        return Ok(None);
    };
    pick_chapter(repo, &series)
}

fn pick_series(series: &[LibrarySeries]) -> Result<Option<LibrarySeries>> {
    let lines = series
        .iter()
        .map(|series| format!("{}\t{}", series.title, series.key))
        .collect::<Vec<_>>();
    let Some(selected) = fzf_select("Library> ", &lines)? else {
        return Ok(None);
    };
    let Some(key) = selected.rsplit('\t').next() else {
        return Ok(None);
    };
    Ok(series.iter().find(|series| series.key == key).cloned())
}

fn pick_search_result(results: &[SearchResult]) -> Result<Option<SearchResult>> {
    let lines = results
        .iter()
        .map(|result| format!("{}\t{}", result.title, result.key))
        .collect::<Vec<_>>();
    let Some(selected) = fzf_select("Results> ", &lines)? else {
        return Ok(None);
    };
    let Some(key) = selected.rsplit('\t').next() else {
        return Ok(None);
    };
    Ok(results.iter().find(|result| result.key == key).cloned())
}

fn pick_chapter(
    repo: &LibraryRepository,
    series: &LibrarySeries,
) -> Result<Option<LibraryChapter>> {
    let chapters = repo.chapters(&series.key)?;
    if chapters.is_empty() {
        return Ok(None);
    }
    let progress = repo
        .progress_for_series(&series.key)?
        .into_iter()
        .map(|progress| (progress.chapter_key, progress.completed))
        .collect::<HashMap<_, _>>();

    let volumes = collect_volumes(&chapters);
    let volume = if volumes.len() > 1 {
        let lines = volumes
            .iter()
            .map(|volume| volume_label_with_progress(*volume, &chapters, &progress))
            .collect::<Vec<_>>();
        let Some(selected) = fzf_select("Volumes> ", &lines)? else {
            return Ok(None);
        };
        volumes
            .into_iter()
            .find(|volume| volume_label_with_progress(*volume, &chapters, &progress) == selected)
    } else {
        volumes.first().copied()
    };

    let filtered = chapters
        .into_iter()
        .filter(|chapter| volume.is_none_or(|volume| same_volume(chapter.volume, volume)))
        .collect::<Vec<_>>();
    let lines = filtered
        .iter()
        .map(|chapter| {
            format!(
                "{} {}  {}  {}\t{}",
                if progress.get(&chapter.key).copied().unwrap_or(false) {
                    "✓"
                } else {
                    " "
                },
                chapter
                    .number
                    .map(format_number)
                    .unwrap_or_else(|| "?".to_string()),
                chapter.title,
                chapter.published_at.as_deref().unwrap_or(""),
                chapter.key,
            )
        })
        .collect::<Vec<_>>();
    let Some(selected) = fzf_select("Chapters> ", &lines)? else {
        return Ok(None);
    };
    let Some(key) = selected.rsplit('\t').next() else {
        return Ok(None);
    };
    Ok(filtered.into_iter().find(|chapter| chapter.key == key))
}

async fn open_reader(
    repo: LibraryRepository,
    source: Box<dyn NovelSource>,
    text_layout: TextLayout,
    chapter: LibraryChapter,
) -> Result<()> {
    let mut app = App::new(repo, source, text_layout)?;

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = app.run_reader_only(&mut terminal, &chapter).await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}

fn fzf_select(prompt: &str, lines: &[impl AsRef<str>]) -> Result<Option<String>> {
    let mut child = Command::new("fzf")
        .arg("--prompt")
        .arg(prompt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to start fzf")?;
    {
        let stdin = child.stdin.as_mut().context("failed to open fzf stdin")?;
        for line in lines {
            writeln!(stdin, "{}", line.as_ref())?;
        }
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
    ))
}

fn fzf_query(prompt: &str) -> Result<String> {
    let output = Command::new("fzf")
        .arg("--prompt")
        .arg(prompt)
        .arg("--phony")
        .arg("--print-query")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .output()
        .context("failed to start fzf")?;
    if !output.status.success() {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string())
}

fn ensure_fzf() -> Result<()> {
    let status = Command::new("fzf")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !matches!(status, Ok(status) if status.success()) {
        bail!("--fzf requires fzf to be installed and available in PATH");
    }
    Ok(())
}

fn collect_volumes(chapters: &[LibraryChapter]) -> Vec<Option<f64>> {
    let mut volumes = chapters
        .iter()
        .map(|chapter| chapter.volume)
        .collect::<Vec<_>>();
    volumes.sort_by(|left, right| match (left, right) {
        (Some(left), Some(right)) => left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    volumes.dedup_by(|left, right| same_volume(*left, *right));
    volumes
}

fn same_volume(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => (left - right).abs() < f64::EPSILON,
        (None, None) => true,
        _ => false,
    }
}

fn volume_label_with_progress(
    volume: Option<f64>,
    chapters: &[LibraryChapter],
    progress: &HashMap<String, bool>,
) -> String {
    let total = chapters
        .iter()
        .filter(|chapter| same_volume(chapter.volume, volume))
        .count();
    let completed = chapters
        .iter()
        .filter(|chapter| same_volume(chapter.volume, volume))
        .filter(|chapter| progress.get(&chapter.key).copied().unwrap_or(false))
        .count();
    format!("({completed}/{total}) {}", volume_label(volume))
}

fn volume_label(volume: Option<f64>) -> String {
    volume
        .map(|volume| format!("Volume {}", format_number(volume)))
        .unwrap_or_else(|| "Prologue / Extras".to_string())
}

fn format_number(number: f64) -> String {
    if number.fract() == 0.0 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}
