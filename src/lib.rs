use std::{
    ffi::OsStr,
    fs::{read_dir, read_to_string, File},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use config::Config;
use glob::glob;
use minijinja::{context, Environment};
use rayon::prelude::*;
use serde::Serialize;
use types::{AvailableContent, Content, Post, PostSourceKind, SearchableDoc};

use crate::types::{LocationData, Metadata};

const POSTS_DIR: &str = "posts";

pub mod config;
pub(crate) mod types;

fn try_parse_post(filepath: PathBuf) -> Result<Post> {
    let kind = PostSourceKind::try_from(
        filepath
            .extension()
            .with_context(|| {
                anyhow!("Provided content file does not have an extension [{filepath:?}]",)
            })
            .and_then(|s| {
                OsStr::to_str(s)
                    .with_context(|| "Unable to turn provided extension to valid `str` [{s:?}]")
            })?,
    )?;

    // Read the file
    let content = read_to_string(&filepath)
        .with_context(|| anyhow!("Unable to read content of a file to string [{filepath:?}]"))?;

    let (front_matter, content) = content.split_once("---").with_context(|| {
        anyhow!("Unable to find the '---' delimiter marking the end of front matter for file [{filepath:?}]")
    })?;

    let metadata: Metadata = front_matter.try_into().with_context(|| {
        anyhow!("Unable to extract front matter metadata for a markdown file: [{filepath:?}]")
    })?;

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

    Ok(Post {
        metadata,
        text,
        html,
        raw,
    })
}

fn render_content<S>(
    content: &Content,
    templates: Arc<Environment>,
    config: &Config,
    data: S,
) -> Result<()>
where
    S: Serialize,
{
    if !content.bare {
        if let Some(folder) = &content.location.dst.parent() {
            std::fs::create_dir_all(folder).context("Unable to create post output directory")?;
        }

        // Render the template
        let context = context!(data => data, ..context!(config));

        let rendered = templates
            .get_template(&content.post.metadata.template)?
            .render(&context)
            .with_context(|| {
                anyhow!(
                    "Unable to render the post: [{:?}]",
                    content.post.metadata.name
                )
            })?;

        let mut writer =
            BufWriter::new(File::create(&content.location.dst).with_context(|| {
                anyhow!(
                    "Unable to create a file  for a post: [{:?}]",
                    content.post.metadata.name
                )
            })?);
        writer.write_all(rendered.as_bytes()).with_context(|| {
            anyhow!(
                "Unable to write post file into output destination [{}]",
                content.location.dst.to_string_lossy()
            )
        })?;
    }

    Ok(())
}

fn render_contents(
    content: &[Content],
    templates: Arc<Environment>,
    config: &Config,
) -> Result<()> {
    content
        .iter()
        .par_bridge()
        .filter(|content| content.publish)
        .map(|content| render_content(content, templates.clone(), config, content))
        .collect::<Result<()>>()
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

    // Create Posts directory
    std::fs::create_dir_all(&config.output).context("Unable to create post output directory")?;

    // Render posts
    render_contents(&content.content, templates.clone(), config)?;

    // Context for rendering supplamentary pages
    let context = context!(data => content, ..context!(config));

    for template in config.render.iter() {
        // Render index
        let rendered = templates.get_template(template)?.render(&context)?;
        let out_filepath = config.output.join(template);
        let mut writer = BufWriter::new(
            File::create(&out_filepath)
                .context(format!("Unable to create a file for [{template}]"))?,
        );
        writer.write_all(rendered.as_bytes()).context(format!(
            "Failed to write the rendered template [{template}]"
        ))?;
    }

    if config.search {
        // Create searchable index
        write_search_index(content, config)?;
    }

    Ok(())
}

fn write_search_index(contents: &AvailableContent, config: &Config) -> Result<()> {
    let output_path = config.output.join("search-index.json");
    let writer = BufWriter::new(File::create(&output_path).context(format!(
        "Unable to create a file for the search index: [{}]",
        output_path.display()
    ))?);
    let docs = contents
        .content
        .par_iter()
        .filter(|content| content.post.metadata.publish)
        .map(TryFrom::try_from)
        .collect::<Result<Vec<SearchableDoc>>>()?;

    serde_json::to_writer(writer, &docs)?;
    Ok(())
}

fn collect_content(config: &Config) -> Result<AvailableContent> {
    let content_dir = config.input.join("content");
    read_dir(content_dir)
        .context("Unable to read content directory")?
        .par_bridge()
        .filter_map(|entry| {
            entry.ok().and_then(|entry| {
                let path = entry.path();
                if path.is_file() {
                    return Some(path);
                }
                None
            })
        })
        .map(|filepath| -> Result<Content> {
            try_parse_post(filepath.clone()).and_then(|post| {
                let publish = post.metadata.publish;
                let bare = post.metadata.bare;
                Ok(Content {
                    location: LocationData::for_post(filepath, config)?,
                    publish,
                    bare,
                    post,
                })
            })
        })
        .try_fold(
            AvailableContent::default,
            |mut acc, content| -> Result<AvailableContent> {
                acc.content.push(content?);
                Ok(acc)
            },
        )
        .try_reduce(
            AvailableContent::default,
            |mut a, mut b| -> Result<AvailableContent> {
                a.content.append(&mut b.content);
                Ok(a)
            },
        )
}

fn include_extras(config: Config) -> Result<()> {
    let include_dir = config.input.join("include");
    if include_dir.exists() {
        if let Some(include_dir_str) = include_dir.to_str() {
            let pattern = format!("{include_dir_str}/**/*");
            glob(&pattern)
                .with_context(|| anyhow!("Unable to glob include directory: [{pattern}]"))?
                .par_bridge()
                .filter_map(Result::ok)
                .map(|src| -> Result<()> {
                    let file = src.strip_prefix("include").with_context(|| {
                        anyhow!("Unable to strip the prefix [{include_dir:?}] from a glob pattern: [{src:?}]")
                    })?;
                    let dst = config.output.join(file);

                    std::fs::copy(&src, &dst).with_context(|| {
                        anyhow!(
                            "Unable to copy include file [{src:?}] into output directory as [{dst:?}]"
                        )
                    })?;
                    Ok(())
                })
                .collect::<Result<()>>()?;
        };
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
