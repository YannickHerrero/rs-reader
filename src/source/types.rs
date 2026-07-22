#[derive(Debug, Clone)]
pub struct SearchResult {
    pub key: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Chapter {
    pub key: String,
    pub title: String,
    pub number: Option<f64>,
    pub published_at: Option<String>,
    pub position: i64,
}

#[derive(Debug, Clone)]
pub struct Series {
    pub key: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone)]
pub struct ChapterContent {
    pub key: String,
    pub title: String,
    pub text: String,
}
