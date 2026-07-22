#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LibrarySeries {
    pub key: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub added_at: i64,
    pub updated_at: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LibraryChapter {
    pub key: String,
    pub series_key: String,
    pub title: String,
    pub number: Option<f64>,
    pub published_at: Option<String>,
    pub position: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Progress {
    pub chapter_key: String,
    pub series_key: String,
    pub scroll_line: i64,
    pub scroll_ratio: f64,
    pub completed: bool,
    pub last_read_at: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CachedChapter {
    pub chapter_key: String,
    pub title: String,
    pub text: String,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SeriesProgressSummary {
    pub completed_count: usize,
    pub chapter_count: usize,
}
