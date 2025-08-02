use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{read_dir, read_to_string, DirEntry, File},
    io::{BufReader, BufWriter, Write},
    path::{PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use config::Config;
use minijinja::{context, Environment};
use rayon::prelude::*;
use types::{
    AvailableContent, Content, ContentKind, LocationData, PhotoProject, PhotoProjectDescription,
    Post, PostSourceKind, SearchableDoc,
};

pub mod config;
pub(crate) mod types;

// TODO: this should be a strictly typed frontmatter
fn parse_front_matter(front_matter: &str) -> Result<HashMap<String, String>> {
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
    front_matter
        .lines()
        .filter_map(parse_line)
        .collect::<Result<_>>()
}

fn try_parse_content(filepath: PathBuf) -> Result<Content> {
    let kind = PostSourceKind::try_from(
        filepath
            .extension()
            .with_context(|| {
                format!(
                    "Provided content file does not have an extension ({})",
                    filepath.to_string_lossy()
                )
            })
            .and_then(|s| {
                OsStr::to_str(s).with_context(|| "Unable to turn provided extension to valid `str`")
            })?,
    )?;

    // Parse the name of the markdown file from it's filepath
    let name: PathBuf = filepath
        .with_extension("")
        .file_name()
        .context("Unable to fetch filename for file when parsing the filepath")?
        .into();

    // Read the file
    let content =
        read_to_string(&filepath).context("Unable to read content of a file to string.")?;

    let (front_matter, content) = content
        .split_once("---")
        .context("Unable to find the '---' delimiter marking the end of front matter")?;

    let metadata = parse_front_matter(front_matter)
        .context("Unable to extract front matter metadata for a markdown file")?;

    let raw = String::from(content);
    let mut html = raw.clone();
    let mut text = None;

    // Parse markdown if needs conversion
    if let PostSourceKind::Markdown = kind {
        let mut text_in_markdown = String::new();
        html = String::new();
        let parser = pulldown_cmark::Parser::new(content).inspect(|event| {
            if let pulldown_cmark::Event::Text(t) = event {
                text_in_markdown.push_str(t);
                text_in_markdown.push(' ')
            }
        });
        // Push the html
        pulldown_cmark::html::push_html(&mut html, parser);
        text = Some(text_in_markdown);
    }

    let value = ContentKind::Post(Post {
        name,
        metadata,
        text,
        html,
        raw,
    });

    Ok(Content {
        location: types::LocationData::Post { filepath },
        value,
    })
}

fn try_parse_photo_project(entry: DirEntry) -> Result<Content> {
    let directory = entry.path();
    assert!(
        directory.is_dir(),
        "Found a DirEntry in photo project parsing that isn't a directory"
    );
    let mut info = directory.clone();
    info.push("info.json");
    let file = File::open(&info).context("Unable open a project info file")?;
    let project_info: PhotoProjectDescription = serde_json::from_reader(BufReader::new(file))
        .context("Unable to deserialize project info file")?;

    Ok(Content {
        location: LocationData::PhotoProject { info, directory },
        value: ContentKind::PhotoProject(PhotoProject {
            info: project_info,
            images: Vec::new(),
        }),
    })
}

fn render_posts(posts: &[Post], templates: Arc<Environment>, config: &Config) -> Result<()> {
    let res = posts
        .iter()
        .par_bridge()
        .map(|post| -> Result<()> {
            let output_dir = config.output.join("posts"); // output/posts
            std::fs::create_dir_all(&output_dir)
                .context("Unable to create output pages directory")?;

            let filename = post.name.with_extension("html"); // post1.html
            let out_filepath = output_dir.join(&filename); // output/pages/post1.html

            // Render the template
            let context = context!(post => post, ..context!(config));
            let template = post
                .metadata
                .get("template")
                .cloned()
                .unwrap_or("post.html".into());
            let rendered = templates
                .get_template(&template)?
                .render(&context)
                .context("Unable to render the post")?;

            let mut writer = BufWriter::new(
                File::create(&out_filepath).context("Unable to create a file for a post.")?,
            );
            writer.write_all(rendered.as_bytes()).context(format!(
                "Unable to write post file into output destination ({})",
                out_filepath.to_string_lossy()
            ))?;
            Ok(())
        })
        .collect::<Result<()>>();
    res
}

fn render(content: &AvailableContent, config: &Config) -> Result<()> {
    let templates = Arc::new({
        let mut env = Environment::new();
        let template_dir = &config.input.join("templates");
        env.set_loader(minijinja::path_loader(template_dir));
        env
    });

    // Cleanup output directory before rendering
    if config.output.exists() {
        std::fs::remove_dir_all(&config.output)
            .context("Unable to remove completely the output directory")?;
    }

    // Render posts
    render_posts(&content.posts, templates.clone(), config)?;

    // Context for rendering supplamentary pages
    let context = context!(content, ..context!(config));

    for template in config.render.iter() {
        // Render index
        let rendered = templates.get_template(template)?.render(&context)?;
        let out_filepath = config.output.join(template);
        let mut writer = BufWriter::new(
            File::create(&out_filepath)
                .context(format!("Unable to create a file for {template}"))?,
        );
        writer
            .write_all(rendered.as_bytes())
            .context(format!("Failed to write the rendered '{template}'"))?;
    }

    if config.search {
        // Create searchable index
        write_search_index(content, config)?;
    }

    Ok(())
}

fn write_search_index(content: &AvailableContent, config: &Config) -> Result<()> {
    let output_path = config.output.join("search-index.json");
    let writer = BufWriter::new(File::create(&output_path).context(format!(
        "Unable to create a file for the search index: {}",
        output_path.display()
    ))?);
    let docs: Result<Vec<SearchableDoc>> =
        content.posts.par_iter().map(TryFrom::try_from).collect();
    serde_json::to_writer(writer, &docs?)?;
    Ok(())
}

fn collect_content(config: &Config) -> Result<AvailableContent> {
    read_dir(config.input.join("content"))
        .context("Unable to read content directory")?
        .par_bridge()
        .map(|entry| -> Result<Content> {
            let entry = entry.context("Unable to retrieve an entry from the directory")?;
            let path = entry.path();
            if path.is_file() {
                try_parse_content(path)
            } else {
                try_parse_photo_project(entry)
            }
        })
        .try_fold(
            AvailableContent::default,
            |mut acc, content_file| -> Result<AvailableContent> {
                match content_file?.value {
                    ContentKind::Post(post) => acc.posts.push(post),
                    ContentKind::PhotoProject(project) => acc.photo_projects.push(project),
                }
                Ok(acc)
            },
        )
        .try_reduce(
            AvailableContent::default,
            |mut a, mut b| -> Result<AvailableContent> {
                a.posts.append(&mut b.posts);
                Ok(a)
            },
        )
}

fn include_extras(config: Config) -> Result<()> {
    let include_dir = config.input.join("include");
    if include_dir.exists() {
        read_dir(include_dir)
            .context("Unable to read include directory")?
            .par_bridge()
            .map(|entry| -> Result<()> {
                let filepath = entry
                    .context("Unable to get dir entry when reading files in `include`")?
                    .path();

                let filename = filepath
                    .file_name()
                    .context("Unable to retrieve filename when including extra files")?;

                std::fs::copy(&filepath, config.output.join(filename))
                    .context("Unable to copy include file into output directory")?;
                Ok(())
            })
            .collect::<Result<()>>()?;
    }
    Ok(())
}

pub fn generate(config: Config) -> Result<()> {
    let content = collect_content(&config)?;

    // Render
    render(&content, &config)?;

    // Include extras
    include_extras(config)
}
