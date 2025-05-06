use std::{collections::HashMap, path::PathBuf};

use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub(crate) struct Blog {
    pub(crate) name: PathBuf,
    pub(crate) metadata: HashMap<String, String>,
    pub(crate) content: String,
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
#[derive(Debug, Default, Serialize)]
pub(crate) struct AvailableContent {
    pub(crate) blogs: Vec<Blog>,
}

