use std::{collections::HashMap, path::PathBuf};

use anyhow::Context;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize, Serializer};
use yaml_rust2::Yaml;

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ContentKind {
    Blog,
}

impl From<&ContentKind> for String {
    fn from(kind: &ContentKind) -> Self {
        match kind {
            ContentKind::Blog => "blog",
        }
        .into()
    }
}

impl TryFrom<&str> for ContentKind {
    type Error = anyhow::Error;

    fn try_from(string: &str) -> std::result::Result<Self, Self::Error> {
        match string {
            "blog" => Ok(Self::Blog),
            _ => Err(anyhow::anyhow!(
                "Unknown enum type for content kind provided ({})",
                string
            )),
        }
    }
}

fn naivedate_to_string<S>(date: &NaiveDate, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let string = date.format("%Y-%m-%d").to_string();
    serializer.serialize_str(&string)
}

/// Metadata derived from the frontmatter of the post markdown
#[derive(Debug, Serialize)]
pub(crate) struct Metadata {
    pub(crate) title: String,
    #[serde(serialize_with = "naivedate_to_string")]
    pub(crate) date: NaiveDate,
    pub(crate) kind: ContentKind,
}

impl TryFrom<HashMap<Yaml, Yaml>> for Metadata {
    type Error = anyhow::Error;

    fn try_from(mut map: HashMap<Yaml, Yaml>) -> std::result::Result<Self, Self::Error> {
        // Extract title
        let title = map
            .remove(&Yaml::String("title".to_string()))
            .context("Unable to find title in the text metadata.")?
            .into_string()
            .context("The provided title in the text metadata is not a string.")?;
        // Extract date
        let date_string = map
            .remove(&Yaml::String("date".to_string())).context("Unable to find date in the text metadata")?
            .into_string().context("Unable to extract string from the date field in the text metadata, ensure it's a string")?;
        let date = NaiveDate::parse_from_str(&date_string, "%Y-%m-%d")
            .context("Unable to parse date for a given text file metadata field")?;

        // Extract content kind
        let kind_string = map
            .remove(&Yaml::String("kind".to_string())).context("Unable to find blog kind in the metadata.")?
            .into_string().context("Unable to extract string from the kind field in the metadata, ensure it's a string")?;
        let kind = kind_string
            .as_str()
            .try_into()
            .context("Unable to parse the kind from the given metadata")?;

        Ok(Self { title, date, kind })
    }
}

#[derive(Debug)]
pub(crate) struct Content {
    pub(crate) metadata: Metadata,
    pub(crate) filename: PathBuf,
    pub(crate) raw: String,
    pub(crate) html: String,
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}
#[derive(Debug, Serialize)]
pub(crate) struct RenderedItem {
    pub(crate) path: PathBuf,
    pub(crate) metadata: Metadata,
}

