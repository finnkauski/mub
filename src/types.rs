use std::{collections::HashMap, path::PathBuf};

use anyhow::Error;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub(crate) struct Blog {
    pub(crate) name: PathBuf,
    pub(crate) metadata: HashMap<String, String>,
    pub(crate) html: String,
    pub(crate) text: String,
    pub(crate) markdown: String,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct SearchableDoc {
    name: PathBuf,
    title: String,
    date: String,
    text: String,
}

impl TryFrom<&Blog> for SearchableDoc {
    type Error = Error;

    fn try_from(blog: &Blog) -> Result<Self, Self::Error> {
        Ok(Self {
            name: blog.name.clone(),
            title: blog.metadata.get("title").ok_or_else(|| anyhow::anyhow!("Unable to find a title while creating a searchable document for the search index in blog: {}", blog.name.display()))?.to_string(),
            date: blog.metadata.get("date").ok_or_else(|| anyhow::anyhow!("Unable to find a date while creating a searchable document for the search index in blog: {}", blog.name.display()))?.to_string(),
            text: blog.text.clone()
        })
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Content {
    Blog(Blog),
}

impl From<&Content> for String {
    fn from(kind: &Content) -> Self {
        match kind {
            Content::Blog { .. } => "blog",
        }
        .into()
    }
}

#[derive(Debug)]
pub(crate) struct ContentFile {
    pub(crate) filepath: PathBuf,
    pub(crate) value: Content,
}

impl std::fmt::Display for ContentFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({})",
            String::from(&self.value),
            self.filepath.to_string_lossy()
        )
    }
}

/// The description for the whole page.
#[derive(Debug, Serialize)]
pub(crate) struct AvailableContent {
    pub(crate) at: DateTime<Utc>,
    pub(crate) blogs: Vec<Blog>,
}

impl Default for AvailableContent {
    fn default() -> Self {
        Self {
            at: Utc::now(),
            blogs: Default::default(),
        }
    }
}
