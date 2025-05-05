use anyhow::{Context, Result};
use comrak::{
    format_html,
    nodes::{AstNode, NodeValue},
    parse_document, Arena, Options,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{read_dir, read_to_string, DirEntry, File},
    io::BufReader,
    path::PathBuf,
};
use thiserror::Error;
use yaml_rust2::{
    Yaml::{self, Hash},
    YamlLoader,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    blogs: PathBuf,
    output: PathBuf,
}

impl Config {
    pub fn try_load(path: PathBuf) -> Result<Self> {
        let file = File::open(path).context("Unable open the config file")?;
        serde_json::from_reader(BufReader::new(file)).context("Unable to deserialize config")
    }
}

pub type TextFiles = Vec<Post>;

#[derive(Debug)]
pub struct Inputs {
    blogs: TextFiles,
}

impl Inputs {
    pub fn try_load(config: Config) -> Result<Self> {
        let blogs = read_dir(config.blogs)
            .context("Unable to read blog directory")?
            .par_bridge()
            .filter_map(|entry| {
                entry
                    .context("Unable to retrieve an entry from the directory")
                    .and_then(Post::try_from)
                    .ok()
            })
            .collect();
        Ok(Self { blogs })
    }
}

/// Metadata derived from the frontmatter of the post markdown
#[derive(Debug)]
pub struct Metadata {
    title: String,
}

impl TryFrom<HashMap<Yaml, Yaml>> for Metadata {
    type Error = anyhow::Error;

    fn try_from(mut map: HashMap<Yaml, Yaml>) -> std::result::Result<Self, Self::Error> {
        let title = map
            .remove(&Yaml::String("title".to_string()))
            .context("Unable to find title in the blog post metadata.")?
            .into_string()
            .context("The provided title in blog post metadata is not a string.")?;
        Ok(Self { title })
    }
}

#[derive(Debug)]
pub struct Post {
    metadata: Metadata,
    filepath: PathBuf,
    raw: String,
    html: String,
}

impl std::fmt::Display for Post {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[derive(Error, Debug)]
pub struct MarkdownParsingError(String);

impl std::fmt::Display for MarkdownParsingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<DirEntry> for Post {
    type Error = anyhow::Error;

    fn try_from(entry: DirEntry) -> Result<Self> {
        // Read the markdown
        let filepath = entry.path();
        let content =
            read_to_string(&filepath).context("Unable to read content of a blog to string.")?;

        // Comrak markdown parsing init
        let arena = Arena::new();
        let mut options = Options::default();

        // Parse the markdown into an AST
        options.extension.front_matter_delimiter = Some(String::from("---"));
        let root = parse_document(&arena, &content, &options);

        // Parse the frontmatter of the AST into the metadata.
        let metadata;
        let front_matter = root
            .first_child()
            .context("Unable to find any children in the parsed markdown AST")?;
        front_matter.detach(); // We disconnect the front matter from the markdown itself
        if let NodeValue::FrontMatter(ref yaml) = front_matter.data.borrow().value {
            let yaml =
                &YamlLoader::load_from_str(yaml).context("Failed to parse frontmatter yaml")?[0];
            if let Hash(map) = yaml {
                let yaml: HashMap<Yaml, Yaml> = map.clone().into_iter().collect();
                metadata = Metadata::try_from(yaml)
                    .context("Unable to parse out metadata from the frontmatter of a post")?;
            } else {
                return Err(anyhow::anyhow!("Unable to find frontmatter as the first item in the markdown. Make sure to include it."));
            }
        } else {
            return Err(anyhow::anyhow!("Unable to find frontmatter as the first item in the markdown. Make sure to include it."));
        }

        // Turn into HTML
        let mut html = vec![];
        format_html(root, &options, &mut html)
            .context("Unable to turn html syntax tree into html")?;

        Ok(Self {
            metadata,
            filepath,
            raw: content,
            html: String::from_utf8(html)
                .context("Unable to turn produced html bytes into a valid utf8 String")?,
        })
    }
}

pub fn generate(config: Config) -> Result<()> {
    let inputs = Inputs::try_load(config)?;
    for i in inputs.blogs {
        println!("{:?}", i);
    }
    Ok(())
}
