use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{read_dir, read_to_string, DirEntry, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use config::Config;
use minijinja::{context, Environment};
use rayon::prelude::*;
use types::{AvailableContent, Blog, Content, ContentFile, SearchableDoc};

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

fn handle_blog(filepath: &Path) -> Result<Content> {
    // Parse the name of the blog from it's filepath
    let name: PathBuf = filepath
        .with_extension("")
        .file_name()
        .context("Unable to fetch filename for file file when parsing the filepath")?
        .into();

    // Read the file
    let markdown =
        read_to_string(filepath).context("Unable to read content of a file to string.")?;

    let (front_matter, markdown) = markdown
        .split_once("---")
        .context("Unable to find the '---' delimiter marking the end of front matter")?;

    let metadata = parse_front_matter(front_matter)
        .context("Unable to extract front matter metadata for blog")?;

    let mut html = String::new();
    let mut text = String::new();

    // Parse markdown
    let parser = pulldown_cmark::Parser::new(markdown).inspect(|event| {
        if let pulldown_cmark::Event::Text(t) = event {
            text.push_str(t);
            text.push(' ')
        }
    });
    pulldown_cmark::html::push_html(&mut html, parser);

    Ok(Content::Blog(Blog {
        name,
        metadata,
        text,
        html,
        markdown: markdown.to_owned(),
    }))
}

fn try_parse_content(entry: DirEntry) -> Result<ContentFile> {
    // Read markdown
    let filepath = entry.path();
    let extension = filepath
        .extension()
        .with_context(|| {
            format!(
                "Provided content file does not have an extension ({})",
                filepath.to_string_lossy()
            )
        })
        .and_then(|s| {
            OsStr::to_str(s).with_context(|| "Unable to turn provided extension to valid `str`")
        });

    let value = match extension? {
        "md" => handle_blog(&filepath)?,
        e => return Err(anyhow::anyhow!("Unsupported extension found ({e})")),
    };

    Ok(ContentFile { filepath, value })
}

fn render_blogs(blogs: &[Blog], templates: Arc<Environment>, config: &Config) -> Result<()> {
    let res = blogs
        .iter()
        .par_bridge()
        .map(|blog| -> Result<()> {
            let output_dir = config.output.join("blog"); // output/blog
            std::fs::create_dir_all(&output_dir)
                .context("Unable to create output blog directory")?;

            let filename = blog.name.with_extension("html"); // post1.html
            let out_filepath = output_dir.join(&filename); // output/blog/post1.html

            // Render the template
            let context = context!(blog => blog, ..context!(config));
            let rendered = templates
                .get_template("blog.html")?
                .render(&context)
                .context("Unable to render the blog post")?;

            let mut writer = BufWriter::new(
                File::create(&out_filepath).context("Unable to create a file for a blog.")?,
            );
            writer.write_all(rendered.as_bytes()).context(format!(
                "Unable to write blog file into output destination ({})",
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

    // Render blogs
    render_blogs(&content.blogs, templates.clone(), config)?;

    // Context for rendering supplamentary pages
    let context = context!(content, ..context!(config));

    // Render index
    let rendered = templates.get_template("index.html")?.render(&context)?;

    let out_filepath = config.output.join("index.html");
    let mut writer = BufWriter::new(
        File::create(&out_filepath).context("Unable to create a file for index.html")?,
    );
    writer
        .write_all(rendered.as_bytes())
        .context("Failed to write the rendered index page")?;

    let rendered = templates.get_template("search.html")?.render(&context)?;

    let out_filepath = config.output.join("search.html");
    let mut writer = BufWriter::new(
        File::create(&out_filepath).context("Unable to create a file for search.html")?,
    );
    writer
        .write_all(rendered.as_bytes())
        .context("Failed to write the rendered search page")?;

    Ok(())
}

fn collect_content(config: &Config) -> Result<AvailableContent> {
    read_dir(config.input.join("content"))
        .context("Unable to read blog directory")?
        .par_bridge()
        .map(|entry| -> Result<ContentFile> {
            entry
                .context("Unable to retrieve an entry from the directory")
                .and_then(try_parse_content)
        })
        .try_fold(
            AvailableContent::default,
            |mut acc, content_file| -> Result<AvailableContent> {
                match content_file?.value {
                    Content::Blog(blog) => acc.blogs.push(blog),
                }
                Ok(acc)
            },
        )
        .try_reduce(
            AvailableContent::default,
            |mut a, mut b| -> Result<AvailableContent> {
                a.blogs.append(&mut b.blogs);
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

fn write_search_index(content: &AvailableContent, config: &Config) -> Result<()> {
    let output_path = config.output.join("search-index.json");
    let writer = BufWriter::new(File::create(&output_path).context(format!(
        "Unable to create a file for the search index: {}",
        output_path.display()
    ))?);
    let docs: Result<Vec<SearchableDoc>> = content.blogs.par_iter().map(TryFrom::try_from).collect();
    serde_json::to_writer(writer, &docs?)?;
    Ok(())
}

pub fn generate(config: Config) -> Result<()> {
    let content = collect_content(&config)?;
    // Render
    render(&content, &config)?;

    // Create searchable index
    write_search_index(&content, &config)?;

    // Include extras
    include_extras(config)
}
