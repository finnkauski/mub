use chrono::NaiveDate;
use serde::{Deserialize, Serialize, Serializer};
use std::{
    collections::HashMap,
    fs::{read_dir, read_to_string, DirEntry},
    path::PathBuf,
    sync::mpsc::channel,
    time::Duration,
};
use tera::Tera;

use anyhow::{Context, Result};
use comrak::{format_html, nodes::NodeValue, parse_document, Arena, Options};
use config::Config;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use yaml_rust2::{Yaml, YamlLoader};

pub mod config {
    use std::{fs::File, io::BufReader, path::PathBuf};

    use anyhow::{Context, Result};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Config {
        /// Template directory
        pub(crate) template_glob: String,
        /// Content description
        pub(crate) content: PathBuf,
        /// Output directory
        pub(crate) output: PathBuf,
    }

    impl Config {
        pub fn try_load(path: PathBuf) -> Result<Self> {
            let file = File::open(path).context("Unable open the config file")?;
            serde_json::from_reader(BufReader::new(file)).context("Unable to deserialize config")
        }
    }
}

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
    raw: String,
    pub(crate) html: String,
}

impl std::fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

fn try_parse_content(entry: DirEntry) -> Result<Content> {
    // Read the markdown
    let filepath = entry.path();
    let content =
        read_to_string(&filepath).context("Unable to read content of a file to string.")?;

    let name = filepath
        .with_extension("")
        .file_name()
        .context("Unable to fetch filename for file file when parsing the filepath")?
        .into();

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
        let yaml = &YamlLoader::load_from_str(yaml).context("Failed to parse frontmatter yaml")?[0];
        if let Yaml::Hash(map) = yaml {
            let yaml: HashMap<Yaml, Yaml> = map.clone().into_iter().collect();
            metadata = Metadata::try_from(yaml)
                .context("Unable to parse out metadata from the frontmatter")?;
        } else {
            return Err(anyhow::anyhow!("Unable to find frontmatter as the first item in the markdown. Make sure to include it."));
        }
    } else {
        return Err(anyhow::anyhow!("Unable to find frontmatter as the first item in the markdown. Make sure to include it."));
    }

    // Turn into HTML
    let mut html = vec![];
    format_html(root, &options, &mut html).context("Unable to turn html syntax tree into html")?;

    Ok(Content {
        metadata,
        filename: name,
        raw: content,
        html: String::from_utf8(html)
            .context("Unable to turn produced html bytes into a valid utf8 String")?,
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct RenderedItem {
    path: PathBuf,
    metadata: Metadata,
}

pub(crate) fn render(content: Content, config: &Config) -> Result<RenderedItem> {
    Ok(match content.metadata.kind {
        ContentKind::Blog => {
            let kind = PathBuf::from(String::from(&content.metadata.kind)); // blog
            let output_dir = config.output.join(&kind); // output/blog
            std::fs::create_dir_all(&output_dir)
                .context("Unable to create output blog directory")?;

            let output_filename = content.filename.with_extension("html"); // post1.html
            let out_file = output_dir.join(&output_filename); // output/blog/post1.html
            std::fs::write(&out_file, &content.html).context(format!(
                "Unable to write blog file into output destination ({})",
                out_file.to_string_lossy()
            ))?;
            RenderedItem {
                path: kind.join(output_filename),
                metadata: content.metadata,
            }
        }
    })
}

fn progress_bar() -> Result<ProgressBar> {
    let style = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:30.cyan/blue} {pos:>2}/{len:3} {msg}",
    )
    .context("Failed to generate a progress bar style from a given template.")?
    .progress_chars("=>-");
    Ok(ProgressBar::new(0)
        .with_style(style)
        .with_message("Rendering..."))
}

pub fn generate(config: Config) -> Result<()> {
    // Cleanup output directory before rendering
    if config.output.exists() {
        std::fs::remove_dir_all(&config.output)
            .context("Unable to remove completely the output directory")?;
    }

    // Some initialisation and prep
    let progress = progress_bar()?;
    let render = |content| render(content, &config);
    let tera = Tera::new(&config.template_glob).context("Failed to initialize templating")?;

    // Main read -> render orchestration
    let rendered = read_dir(&config.content)
        .context("Unable to read blog directory")?
        .par_bridge()
        .map(|entry| -> Result<RenderedItem> {
            progress.inc_length(1);
            entry
                .context("Unable to retrieve an entry from the directory")
                .and_then(try_parse_content)
                .and_then(render)
                .inspect(|_| progress.inc(1))
        })
        .collect::<Result<Vec<RenderedItem>>>()?;

    // Render the index page.
    let mut context = tera::Context::new();
    context.insert("rendered", &rendered);

    let rendered_index = tera.render("index.html", &context)?;
    std::fs::write(config.output.join("index.html"), rendered_index)
        .context("Failed to write the rendered index page")?;

    progress.finish_with_message("Finished rendering.");

    Ok(())
}
