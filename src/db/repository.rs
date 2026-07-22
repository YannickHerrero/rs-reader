use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, params};

use crate::source::{ChapterContent, Series};

use super::types::{CachedChapter, LibraryChapter, LibrarySeries, Progress, SeriesProgressSummary};

pub struct LibraryRepository {
    conn: Connection,
}

impl LibraryRepository {
    pub fn open_default() -> Result<Self> {
        let dirs = ProjectDirs::from("me", "yannick", "rs-reader")
            .context("failed to resolve application data directory")?;
        fs::create_dir_all(dirs.data_dir()).context("failed to create data directory")?;
        Self::open(dirs.data_dir().join("library.sqlite"))
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        let conn = Connection::open(path).context("failed to open SQLite database")?;
        let repo = Self { conn };
        repo.migrate()?;
        Ok(repo)
    }

    pub fn add_or_update_series(&mut self, series: &Series) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let tx = self.conn.transaction()?;
        let existing_added_at = tx
            .query_row(
                "select added_at from series where key = ?1",
                params![series.key],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        tx.execute(
            "insert into series (key, title, cover_url, author, description, status, added_at, updated_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             on conflict(key) do update set
               title = excluded.title,
               cover_url = excluded.cover_url,
               author = excluded.author,
               description = excluded.description,
               status = excluded.status,
               updated_at = excluded.updated_at",
            params![
                series.key,
                series.title,
                series.cover_url,
                series.author,
                series.description,
                series.status,
                existing_added_at.unwrap_or(now),
                now,
            ],
        )?;
        for chapter in &series.chapters {
            tx.execute(
                "insert into chapters (key, series_key, title, number, published_at, position)
                 values (?1, ?2, ?3, ?4, ?5, ?6)
                 on conflict(key) do update set
                   series_key = excluded.series_key,
                   title = excluded.title,
                   number = excluded.number,
                   published_at = excluded.published_at,
                   position = excluded.position",
                params![
                    chapter.key,
                    series.key,
                    chapter.title,
                    chapter.number,
                    chapter.published_at,
                    chapter.position,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_series(&self) -> Result<Vec<LibrarySeries>> {
        let mut stmt = self.conn.prepare(
            "select key, title, cover_url, author, description, status, added_at, updated_at
             from series order by added_at desc",
        )?;
        let rows = stmt.query_map([], map_series)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_series(&self, key: &str) -> Result<Option<LibrarySeries>> {
        self.conn
            .query_row(
                "select key, title, cover_url, author, description, status, added_at, updated_at
                 from series where key = ?1",
                params![key],
                map_series,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn chapters(&self, series_key: &str) -> Result<Vec<LibraryChapter>> {
        let mut stmt = self.conn.prepare(
            "select key, series_key, title, number, published_at, position
             from chapters where series_key = ?1 order by position asc",
        )?;
        let rows = stmt.query_map(params![series_key], map_chapter)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn chapter(&self, key: &str) -> Result<Option<LibraryChapter>> {
        self.conn
            .query_row(
                "select key, series_key, title, number, published_at, position
                 from chapters where key = ?1",
                params![key],
                map_chapter,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn save_progress(
        &self,
        series_key: &str,
        chapter_key: &str,
        scroll_line: i64,
        scroll_ratio: f64,
        completed: bool,
    ) -> Result<()> {
        let previous_completed = self
            .progress(chapter_key)?
            .map(|progress| progress.completed)
            .unwrap_or(false);
        self.conn.execute(
            "insert into progress (chapter_key, series_key, scroll_line, scroll_ratio, completed, last_read_at)
             values (?1, ?2, ?3, ?4, ?5, ?6)
             on conflict(chapter_key) do update set
               series_key = excluded.series_key,
               scroll_line = excluded.scroll_line,
               scroll_ratio = excluded.scroll_ratio,
               completed = excluded.completed,
               last_read_at = excluded.last_read_at",
            params![
                chapter_key,
                series_key,
                scroll_line.max(0),
                scroll_ratio.clamp(0.0, 1.0),
                completed || previous_completed,
                Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(())
    }

    pub fn progress(&self, chapter_key: &str) -> Result<Option<Progress>> {
        self.conn
            .query_row(
                "select chapter_key, series_key, scroll_line, scroll_ratio, completed, last_read_at
                 from progress where chapter_key = ?1",
                params![chapter_key],
                map_progress,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn progress_for_series(&self, series_key: &str) -> Result<Vec<Progress>> {
        let mut stmt = self.conn.prepare(
            "select chapter_key, series_key, scroll_line, scroll_ratio, completed, last_read_at
             from progress where series_key = ?1",
        )?;
        let rows = stmt.query_map(params![series_key], map_progress)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn series_progress(&self, series_key: &str) -> Result<SeriesProgressSummary> {
        let chapter_count = self
            .conn
            .query_row(
                "select count(*) from chapters where series_key = ?1",
                params![series_key],
                |row| row.get::<_, i64>(0),
            )?
            .max(0) as usize;
        let completed_count = self
            .conn
            .query_row(
                "select count(*) from progress where series_key = ?1 and completed = 1",
                params![series_key],
                |row| row.get::<_, i64>(0),
            )?
            .max(0) as usize;
        Ok(SeriesProgressSummary {
            completed_count,
            chapter_count,
        })
    }

    pub fn cache_chapter(&self, content: &ChapterContent) -> Result<()> {
        self.conn.execute(
            "insert into chapter_cache (chapter_key, title, text, fetched_at)
             values (?1, ?2, ?3, ?4)
             on conflict(chapter_key) do update set
               title = excluded.title,
               text = excluded.text,
               fetched_at = excluded.fetched_at",
            params![
                content.key,
                content.title,
                content.text,
                Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(())
    }

    pub fn cached_chapter(&self, chapter_key: &str) -> Result<Option<CachedChapter>> {
        self.conn
            .query_row(
                "select chapter_key, title, text, fetched_at from chapter_cache where chapter_key = ?1",
                params![chapter_key],
                |row| {
                    Ok(CachedChapter {
                        chapter_key: row.get(0)?,
                        title: row.get(1)?,
                        text: row.get(2)?,
                        fetched_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "pragma foreign_keys = on;

             create table if not exists series (
               key text primary key,
               title text not null,
               cover_url text,
               author text,
               description text,
               status text,
               added_at integer not null,
               updated_at integer not null
             );

             create table if not exists chapters (
               key text primary key,
               series_key text not null references series(key) on delete cascade,
               title text not null,
               number real,
               published_at text,
               position integer not null
             );
             create index if not exists chapters_series_position on chapters(series_key, position);

             create table if not exists progress (
               chapter_key text primary key references chapters(key) on delete cascade,
               series_key text not null references series(key) on delete cascade,
               scroll_line integer not null,
               scroll_ratio real not null,
               completed integer not null,
               last_read_at integer not null
             );
             create index if not exists progress_series on progress(series_key);

             create table if not exists chapter_cache (
               chapter_key text primary key,
               title text not null,
               text text not null,
               fetched_at integer not null
             );",
        )?;
        Ok(())
    }
}

fn map_series(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibrarySeries> {
    Ok(LibrarySeries {
        key: row.get(0)?,
        title: row.get(1)?,
        cover_url: row.get(2)?,
        author: row.get(3)?,
        description: row.get(4)?,
        status: row.get(5)?,
        added_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn map_chapter(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryChapter> {
    Ok(LibraryChapter {
        key: row.get(0)?,
        series_key: row.get(1)?,
        title: row.get(2)?,
        number: row.get(3)?,
        published_at: row.get(4)?,
        position: row.get(5)?,
    })
}

fn map_progress(row: &rusqlite::Row<'_>) -> rusqlite::Result<Progress> {
    Ok(Progress {
        chapter_key: row.get(0)?,
        series_key: row.get(1)?,
        scroll_line: row.get(2)?,
        scroll_ratio: row.get(3)?,
        completed: row.get(4)?,
        last_read_at: row.get(5)?,
    })
}
