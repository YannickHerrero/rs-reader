pub mod novel_fr;
pub mod syosetu;
pub mod types;

use anyhow::Result;
use async_trait::async_trait;

pub use novel_fr::NovelFrSource;
pub use syosetu::SyosetuSource;
pub use types::{ChapterContent, SearchResult, Series};

#[async_trait]
pub trait NovelSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn recommendations(&self) -> Result<Vec<SearchResult>>;
    async fn search(&self, query: &str) -> Result<Vec<SearchResult>>;
    async fn series(&self, key: &str) -> Result<Series>;
    async fn chapter(&self, key: &str) -> Result<ChapterContent>;
}
