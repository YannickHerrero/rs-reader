use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use scraper::{Html, Selector};
use serde::Deserialize;
use tokio::time::sleep;

use super::types::{Chapter, ChapterContent, SearchResult, Series};

const BASE_URL: &str = "https://novel-fr.net";
const PREFIX: &str = "novelFr:";
const USER_AGENT: &str = "rs-reader/0.1 (+personal reading client)";

#[derive(Debug)]
pub struct NovelFrSource {
    client: Client,
    next_request_at: tokio::sync::Mutex<Instant>,
}

#[derive(Debug, Deserialize)]
struct WordPressSearchResult {
    title: Option<String>,
    url: Option<String>,
}

impl NovelFrSource {
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

    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let mut url = Url::parse(BASE_URL)?.join("/wp-json/wp/v2/search")?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("per_page", "20");
        let items: Vec<WordPressSearchResult> = self.request_json(url).await?;
        Ok(items
            .into_iter()
            .filter_map(|item| {
                let key = source_key(item.url.as_deref()?)?;
                if !key.starts_with(&format!("{PREFIX}/series/")) {
                    return None;
                }
                let title = strip_html(item.title.as_deref().unwrap_or_default());
                (!title.is_empty()).then_some(SearchResult { key, title })
            })
            .collect())
    }

    pub async fn series(&self, key: &str) -> Result<Series> {
        let path = source_path(key)?;
        if !path.starts_with("/series/") || !path.ends_with('/') {
            bail!("invalid Novel-FR series key");
        }
        let html = self
            .request_text(Url::parse(BASE_URL)?.join(&path)?)
            .await?;
        let document = Html::parse_document(&html);

        let title = text_of(&document, "h1.entry-title")
            .context("Novel-FR returned a series without a title")?;
        let cover_url = attr_of(&document, ".sertothumb img", "data-src")
            .or_else(|| attr_of(&document, ".sertothumb img", "src"))
            .and_then(|raw| absolute_url(&raw));
        let author = series_field(&document, "Auteur");
        let description = text_of(&document, ".sersys.entry-content");
        let status = text_of(&document, ".sertostat span");
        let chapters = parse_chapters(&document);

        Ok(Series {
            key: key.to_string(),
            title,
            cover_url,
            author,
            description,
            status,
            chapters,
        })
    }

    pub async fn chapter(&self, key: &str) -> Result<ChapterContent> {
        let path = source_path(key)?;
        if !path.starts_with('/') || !path.ends_with('/') || path.starts_with("/series/") {
            bail!("invalid Novel-FR chapter key");
        }
        let html = self
            .request_text(Url::parse(BASE_URL)?.join(&path)?)
            .await?;
        let document = Html::parse_document(&html);
        let title = text_of(&document, "h1.entry-title").unwrap_or_else(|| "Chapitre".to_string());
        let selector = Selector::parse(".entry-content.epcontent").expect("valid selector");
        let chapter_html = document
            .select(&selector)
            .next()
            .map(|node| node.inner_html())
            .unwrap_or_default();
        let text = html2text::from_read(chapter_html.as_bytes(), 100)
            .context("failed to convert chapter html to text")?
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        if text.is_empty() {
            bail!("Novel-FR returned an empty chapter");
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
            bail!("Novel-FR returned HTTP {status}");
        }
        Ok(response.text().await?)
    }

    async fn request_json<T: serde::de::DeserializeOwned>(&self, url: Url) -> Result<T> {
        self.throttle().await;
        let response = self.client.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            bail!("Novel-FR returned HTTP {status}");
        }
        Ok(response.json().await?)
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

fn parse_chapters(document: &Html) -> Vec<Chapter> {
    let item_selector = Selector::parse(".eplister li a").expect("valid selector");
    let num_selector = Selector::parse(".epl-num").expect("valid selector");
    let title_selector = Selector::parse(".epl-title").expect("valid selector");
    let date_selector = Selector::parse(".epl-date").expect("valid selector");
    let mut seen_numbers = HashSet::new();
    let mut chapters = document
        .select(&item_selector)
        .filter_map(|link| {
            let key = source_key(link.value().attr("href")?)?;
            let label = link
                .select(&num_selector)
                .next()
                .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
                .unwrap_or_default();
            let suffix = link
                .select(&title_selector)
                .next()
                .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
                .unwrap_or_default();
            let number = chapter_number(&label)
                .or_else(|| chapter_number(&suffix))
                .or_else(|| chapter_number(&key));
            if let Some(number) = number {
                let number_key = (number * 1000.0).round() as i64;
                if !seen_numbers.insert(number_key) {
                    return None;
                }
            }
            let title = match (number, suffix.is_empty()) {
                (Some(number), false) => format!("Chapitre {} · {suffix}", format_number(number)),
                (Some(number), true) => format!("Chapitre {}", format_number(number)),
                (None, false) => suffix,
                (None, true) => label,
            };
            let published_at = link
                .select(&date_selector)
                .next()
                .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
                .filter(|value| !value.is_empty());
            Some(Chapter {
                key,
                title,
                number,
                published_at,
                position: 0,
            })
        })
        .collect::<Vec<_>>();
    chapters.sort_by(|left, right| {
        right
            .number
            .partial_cmp(&left.number)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (position, chapter) in chapters.iter_mut().enumerate() {
        chapter.position = position as i64;
    }
    chapters
}

fn source_key(raw_url: &str) -> Option<String> {
    let url = Url::parse(BASE_URL).ok()?.join(raw_url).ok()?;
    (url.origin().ascii_serialization() == BASE_URL).then(|| format!("{PREFIX}{}", url.path()))
}

fn source_path(key: &str) -> Result<String> {
    let path = key
        .strip_prefix(PREFIX)
        .context("invalid Novel-FR key prefix")?;
    if !path.starts_with('/') {
        bail!("invalid Novel-FR path");
    }
    Ok(path.to_string())
}

fn absolute_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(BASE_URL).ok()?.join(raw).ok()?;
    if url.scheme() == "http" {
        url.set_scheme("https").ok()?;
    }
    Some(url.to_string())
}

fn text_of(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
        .filter(|text| !text.is_empty())
}

fn attr_of(document: &Html, selector: &str, attr: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|node| node.value().attr(attr))
        .map(str::to_string)
}

fn series_field(document: &Html, label: &str) -> Option<String> {
    let row_selector = Selector::parse(".serl").expect("valid selector");
    let name_selector = Selector::parse(".sername").expect("valid selector");
    let value_selector = Selector::parse(".serval").expect("valid selector");
    document.select(&row_selector).find_map(|row| {
        let name = row
            .select(&name_selector)
            .next()
            .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))?;
        if name != label {
            return None;
        }
        row.select(&value_selector)
            .next()
            .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")))
            .filter(|value| !value.is_empty())
    })
}

fn strip_html(value: &str) -> String {
    clean_text(
        &Html::parse_fragment(value)
            .root_element()
            .text()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_number(number: f64) -> String {
    if number.fract() == 0.0 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

fn chapter_number(label: &str) -> Option<f64> {
    let lower = label.to_lowercase().replace(',', ".");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_source_keys_for_novel_fr_urls() {
        assert_eq!(
            source_key("https://novel-fr.net/series/demo/"),
            Some("novelFr:/series/demo/".to_string())
        );
        assert_eq!(source_key("https://example.com/series/demo/"), None);
    }

    #[test]
    fn parses_chapter_numbers() {
        assert_eq!(chapter_number("Chapitre 12"), Some(12.0));
        assert_eq!(chapter_number("Chap. 10,5"), Some(10.5));
        assert_eq!(chapter_number("Ch. 42"), Some(42.0));
        assert_eq!(chapter_number("novelFr:/demo-chapitre-123/"), Some(123.0));
    }
}
