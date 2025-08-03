use std::{collections::HashMap, path::PathBuf};

use anyhow::Error;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub(crate) struct PhotoProject {
    pub post: Post,
    pub(crate) images: Vec<String>,
    pub(crate) image_locations: Vec<PathBuf>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct Post {
    pub(crate) name: PathBuf,
    pub(crate) metadata: HashMap<String, String>,
    pub(crate) raw: String,
    pub(crate) html: String,
    pub(crate) text: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct SearchableDoc {
    name: PathBuf,
    title: String,
    date: String,
    text: String,
}

impl TryFrom<&Post> for SearchableDoc {
    type Error = Error;

    fn try_from(post: &Post) -> Result<Self, Self::Error> {
        Ok(Self {
            name: post.name.clone(),
            title: post.metadata.get("title").ok_or_else(|| anyhow::anyhow!("Unable to find a title while creating a searchable document for the search index in post: {}", post.name.display()))?.to_string(),
            date: post.metadata.get("date").ok_or_else(|| anyhow::anyhow!("Unable to find a date while creating a searchable document for the search index in post: {}", post.name.display()))?.to_string(),
            text: post.text.clone().unwrap_or_else(|| post.raw.to_owned()).clone()
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
#[serde(rename_all = "lowercase")]
pub(crate) enum ContentKind {
    Post(Post),
    PhotoProject(PhotoProject),
}

impl From<&ContentKind> for String {
    fn from(kind: &ContentKind) -> Self {
        match kind {
            ContentKind::Post(post) => post.name.to_string_lossy().into(),
            ContentKind::PhotoProject(project) => project.post.name.to_string_lossy().into(),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub(crate) enum LocationData {
    PhotoProject { directory: PathBuf },
    Post { filepath: PathBuf },
}

#[derive(Debug, Serialize)]
pub(crate) struct Content {
    pub(crate) location: LocationData,
    pub(crate) value: ContentKind,
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] ({})",
            String::from(&self.value),
            serde_json::to_string(&self.location).expect("Unable to convert location to string"),
        )
    }
}

/// The description for the whole page.
#[derive(Debug, Serialize)]
pub(crate) struct AvailableContent {
    pub(crate) at: DateTime<Utc>,
    pub(crate) posts: Vec<Post>,
    pub(crate) photo_projects: Vec<PhotoProject>,
}

impl Default for AvailableContent {
    fn default() -> Self {
        Self {
            at: Utc::now(),
            posts: Default::default(),
            photo_projects: Default::default(),
        }
    }
}
