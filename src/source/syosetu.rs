use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::{Client, Url};
use scraper::{Html, Selector};
use tokio::time::sleep;

use super::NovelSource;
use super::types::{Chapter, ChapterContent, SearchResult, Series};

const BASE_URL: &str = "https://ncode.syosetu.com";
const SEARCH_URL: &str = "https://yomou.syosetu.com";
const PREFIX: &str = "syosetu:";
const USER_AGENT: &str = "rs-reader/0.1 (+personal reading client)";

#[derive(Debug)]
pub struct SyosetuSource {
    client: Client,
    next_request_at: tokio::sync::Mutex<Instant>,
}

impl SyosetuSource {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            client,
            next_request_at: tokio::sync::Mutex::new(Instant::now()),
        })
    }

    async fn recommendations_impl(&self) -> Result<Vec<SearchResult>> {
        let html = self
            .request_text(Url::parse(SEARCH_URL)?.join("/rank/list/type/total_total/")?)
            .await?;
        let document = Html::parse_document(&html);
        Ok(parse_listing(&document, ".p-ranklist-item__title a"))
    }

    async fn search_impl(&self, query: &str) -> Result<Vec<SearchResult>> {
        let mut url = Url::parse(SEARCH_URL)?.join("/search.php")?;
        url.query_pairs_mut().append_pair("word", query);
        let html = self.request_text(url).await?;
        let document = Html::parse_document(&html);
        Ok(parse_listing(&document, ".novel_h a.tl"))
    }

    async fn series_impl(&self, key: &str) -> Result<Series> {
        let ncode = source_ncode(key)?;
        let html = self
            .request_text(Url::parse(BASE_URL)?.join(&format!("/{ncode}/"))?)
            .await?;
        let document = Html::parse_document(&html);
        let title = text_of(&document, ".p-novel__title")
            .or_else(|| text_of(&document, "title"))
            .context("Syosetu returned a series without a title")?;
        let author = text_of(&document, ".p-novel__author").map(|author| {
            author
                .strip_prefix("作者：")
                .unwrap_or(&author)
                .trim()
                .to_string()
        });
        let description = text_of(&document, ".p-novel__summary, #novel_ex");
        let chapters = parse_chapters(&document, &ncode);
        if chapters.is_empty() {
            bail!("Syosetu returned a series without chapters");
        }

        Ok(Series {
            key: format!("{PREFIX}{ncode}"),
            title,
            cover_url: None,
            author,
            description,
            status: None,
            chapters,
        })
    }

    async fn chapter_impl(&self, key: &str) -> Result<ChapterContent> {
        let (ncode, episode) = source_chapter(key)?;
        let html = self
            .request_text(Url::parse(BASE_URL)?.join(&format!("/{ncode}/{episode}/"))?)
            .await?;
        let document = Html::parse_document(&html);
        let title =
            text_of(&document, ".p-novel__title").unwrap_or_else(|| format!("Episode {episode}"));
        let text = parse_chapter_text(&document);
        if text.trim().is_empty() {
            bail!("Syosetu returned an empty chapter");
        }
        Ok(ChapterContent {
            key: key.to_string(),
            title,
            text,
        })
    }

    async fn request_text(&self, url: Url) -> Result<String> {
        self.throttle().await;
        let response = self.client.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            bail!("Syosetu returned HTTP {status}");
        }
        Ok(response.text().await?)
    }

    async fn throttle(&self) {
        let mut next = self.next_request_at.lock().await;
        let now = Instant::now();
        if *next > now {
            sleep(*next - now).await;
        }
        *next = Instant::now() + Duration::from_millis(500);
    }
}

#[async_trait]
impl NovelSource for SyosetuSource {
    fn name(&self) -> &'static str {
        "Syosetu"
    }

    async fn recommendations(&self) -> Result<Vec<SearchResult>> {
        self.recommendations_impl().await
    }

    async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        self.search_impl(query).await
    }

    async fn series(&self, key: &str) -> Result<Series> {
        self.series_impl(key).await
    }

    async fn chapter(&self, key: &str) -> Result<ChapterContent> {
        self.chapter_impl(key).await
    }
}

fn parse_listing(document: &Html, selector: &str) -> Vec<SearchResult> {
    let selector = Selector::parse(selector).expect("valid selector");
    let mut seen = HashSet::new();
    document
        .select(&selector)
        .filter_map(|link| {
            let key = source_key(link.value().attr("href")?)?;
            if !seen.insert(key.clone()) {
                return None;
            }
            let title = clean_text(&link.text().collect::<Vec<_>>().join(" "));
            (!title.is_empty()).then_some(SearchResult { key, title })
        })
        .collect()
}

fn parse_chapters(document: &Html, ncode: &str) -> Vec<Chapter> {
    let chapter_selector = Selector::parse(".p-eplist__chapter-title").expect("valid selector");
    let item_selector = Selector::parse(".p-eplist__sublist").expect("valid selector");
    let link_selector = Selector::parse(".p-eplist__subtitle").expect("valid selector");
    let date_selector = Selector::parse(".p-eplist__update").expect("valid selector");

    let mut volume_titles = document
        .select(&chapter_selector)
        .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
        .filter(|title| !title.is_empty());
    let mut current_volume = 1.0;
    let mut next_volume_title = volume_titles.next();

    let mut chapters = Vec::new();
    for (position, item) in document.select(&item_selector).enumerate() {
        if next_volume_title.is_some() && chapters.is_empty() {
            next_volume_title = volume_titles.next();
        }
        let Some(link) = item.select(&link_selector).next() else {
            continue;
        };
        let href = link.value().attr("href").unwrap_or_default();
        let Some(episode) = episode_from_path(href) else {
            continue;
        };
        let title = clean_text(&link.text().collect::<Vec<_>>().join(" "));
        let published_at = item
            .select(&date_selector)
            .next()
            .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
            .filter(|value| !value.is_empty());
        chapters.push(Chapter {
            key: format!("{PREFIX}{ncode}/{episode}"),
            title: if title.is_empty() {
                format!("第{episode}話")
            } else {
                title
            },
            number: Some(episode as f64),
            volume: Some(current_volume),
            published_at,
            position: position as i64,
        });

        // Syosetu's HTML does not nest episodes in chapter containers, so keep the
        // first version simple: expose a single volume when exact arc boundaries are
        // not available from the sublist item itself.
        if next_volume_title.is_some() {
            current_volume = 1.0;
        }
    }
    chapters
}

fn parse_chapter_text(document: &Html) -> String {
    let paragraph_selector = Selector::parse(".js-novel-text p").expect("valid selector");
    document
        .select(&paragraph_selector)
        .map(|paragraph| paragraph.text().collect::<Vec<_>>().join(""))
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn source_key(raw_url: &str) -> Option<String> {
    let url = Url::parse(BASE_URL).ok()?.join(raw_url).ok()?;
    if url.host_str()? != "ncode.syosetu.com" {
        return None;
    }
    let ncode = url.path_segments()?.find(|part| is_ncode(part))?;
    Some(format!("{PREFIX}{ncode}"))
}

fn source_ncode(key: &str) -> Result<String> {
    let value = key
        .strip_prefix(PREFIX)
        .context("invalid Syosetu key prefix")?;
    let ncode = value.split('/').next().unwrap_or_default();
    if !is_ncode(ncode) {
        bail!("invalid Syosetu ncode");
    }
    Ok(ncode.to_string())
}

fn source_chapter(key: &str) -> Result<(String, usize)> {
    let value = key
        .strip_prefix(PREFIX)
        .context("invalid Syosetu key prefix")?;
    let mut parts = value.split('/');
    let ncode = parts.next().unwrap_or_default();
    let episode = parts
        .next()
        .and_then(|part| part.parse::<usize>().ok())
        .context("invalid Syosetu chapter key")?;
    if !is_ncode(ncode) {
        bail!("invalid Syosetu ncode");
    }
    Ok((ncode.to_string(), episode))
}

fn episode_from_path(path: &str) -> Option<usize> {
    let url = Url::parse(BASE_URL).ok()?.join(path).ok()?;
    url.path_segments()?
        .filter_map(|part| part.parse().ok())
        .next()
}

fn is_ncode(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('n' | 'N'))
        && chars.any(|ch| ch.is_ascii_digit())
        && value.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn text_of(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
        .filter(|text| !text.is_empty())
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_syosetu_keys() {
        assert_eq!(
            source_key("https://ncode.syosetu.com/n9669bk/"),
            Some("syosetu:n9669bk".to_string())
        );
    }

    #[test]
    fn parses_chapter_keys() {
        assert_eq!(
            source_chapter("syosetu:n9669bk/12").unwrap(),
            ("n9669bk".to_string(), 12)
        );
    }
}
