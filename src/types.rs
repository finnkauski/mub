use std::{collections::HashMap, path::PathBuf};

use anyhow::{anyhow, Context, Error, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::POSTS_DIR;

#[derive(Debug, Serialize, Clone)]
pub(crate) struct Metadata {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) template: String,
    pub(crate) date: String,
    pub(crate) publish: bool,
    pub(crate) bare: bool,
    pub(crate) extra: HashMap<String, String>,
}

// TODO: this should be a deserialize implementation
// TODO: tie lifetimes here with &str
impl TryFrom<&str> for Metadata {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let parse_line = |line: &str| -> Option<Result<(String, String)>> {
            if line == "---" || line.is_empty() {
                return None;
            };
            Some(
                line.split_once(":")
                    .ok_or_else(|| {
                        anyhow::anyhow!("Unable to find `:` in the front matter line: [{line}]")
                    })
                    .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())),
            )
        };
        let extra = value
            .lines()
            .filter_map(parse_line)
            .collect::<Result<HashMap<String, String>>>()?;

        Ok(Self {
            name: extra
                .get("name")
                .cloned()
                .ok_or_else(|| anyhow!("Unable to find name in metadata"))?,
            title: extra
                .get("title")
                .cloned()
                .ok_or_else(|| anyhow!("Unable to find title in metadata"))?,
            template: extra
                .get("template")
                .cloned()
                .unwrap_or_else(|| String::from("post.html")),
            date: extra
                .get("date")
                .cloned()
                .ok_or_else(|| anyhow!("Unable to find date in metadata"))?,
            publish: extra
                .get("publish")
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            bare: extra
                .get("bare")
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            extra,
        })
    }
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct Post {
    pub(crate) metadata: Metadata,
    pub(crate) raw: String,
    pub(crate) html: String,
    pub(crate) text: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct SearchableDoc {
    path: PathBuf,
    title: String,
    date: String,
    text: String,
}

impl TryFrom<&Content> for SearchableDoc {
    type Error = Error;

    fn try_from(content: &Content) -> Result<Self, Self::Error> {
        Ok(Self {
            path: content.location.dst.clone(),
            title: content.post.metadata.title.clone(),
            date: content.post.metadata.date.clone(),
            text: content
                .post
                .text
                .clone()
                .unwrap_or_else(|| {
                    content
                        .post
                        .text
                        .clone() // TODO: clean these up
                        .unwrap_or(content.post.raw.clone())
                })
                .clone(),
        })
    }
}

#[derive(Debug)]
pub(crate) enum PostSourceKind {
    Html,
    Markdown,
}

impl TryFrom<&str> for PostSourceKind {
    type Error = Error;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value {
            "md" => Ok(Self::Markdown),
            "html" => Ok(Self::Html),
            _ => Err(anyhow::anyhow!("Unknown extension passed: {value}")),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct LocationData {
   pub(crate) src: PathBuf,
   pub(crate) dst: PathBuf,
   pub(crate) url: PathBuf,
   pub(crate) filename: String,
}

impl LocationData {
    pub(crate) fn for_post(filepath: PathBuf, config: &crate::config::Config) -> Result<LocationData> {
        let filename = filepath
            .with_extension("html")
            .file_name()
            .with_context(|| {
                anyhow::anyhow!("Unable to fetch location output filename for post: {filepath:?}")
            })?
            .to_string_lossy()
            .to_string();

        let url = PathBuf::from(POSTS_DIR).join(&filename);
        let dst = config.output.join(&url);

        Ok(Self {
            src: filepath,
            dst,
            url,
            filename,
        })
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Content {
    /// Whether any copying has to happen for this content or is it just
    /// virtualised and presented in the context
    pub(crate) bare: bool,
    /// Whether this content should be visible at all
    pub(crate) publish: bool,
    pub(crate) location: LocationData,
    pub(crate) post: Post,
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}]", self.post.metadata.name,)
    }
}

/// The description for the whole page.
#[derive(Debug, Serialize)]
pub(crate) struct AvailableContent {
    pub(crate) at: DateTime<Utc>,
    pub(crate) content: Vec<Content>,
}

impl Default for AvailableContent {
    fn default() -> Self {
        Self {
            at: Utc::now(),
            content: Default::default(),
        }
    }
}
