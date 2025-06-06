use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{read_dir, read_to_string, DirEntry},
    path::{Path, PathBuf},
    sync::Arc,
};
use tera::Tera;

use anyhow::{Context, Result};
use comrak::{format_html, nodes::NodeValue, parse_document, Arena, Options};
use config::Config;
use rayon::prelude::*;
use types::{AvailableContent, Blog, Content, ContentFile};

pub mod config;
pub(crate) mod types;

fn extract_front_matter_data<'line>(string: &'line str) -> Result<HashMap<String, String>> {
    let parse_line = |line: &'line str| -> Option<Result<(String, String)>> {
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
    string.lines().filter_map(parse_line).collect()
}

fn handle_blog(filepath: &Path) -> Result<Content> {
    // Parse the name of the blog from it's filepath
    let name: PathBuf = filepath
        .with_extension("")
        .file_name()
        .context("Unable to fetch filename for file file when parsing the filepath")?
        .into();

    // Read the file
    let content =
        read_to_string(filepath).context("Unable to read content of a file to string.")?;

    let arena = Arena::new();
    let mut options = Options::default();
    // Parse the markdown into an AST
    options.extension.front_matter_delimiter = Some(String::from("---"));
    let root = parse_document(&arena, &content, &options);

    // Parse the frontmatter of the AST into the metadata.
    let metadata;

    // Fetch frontmatter
    let front_matter = root
        .first_child()
        .context("Unable to find any children in the parsed markdown AST")?;
    front_matter.detach(); // We disconnect the front matter from the markdown itself

    // Parse frontmatter metadata
    if let NodeValue::FrontMatter(ref yaml) = front_matter.data.borrow().value {
        metadata = extract_front_matter_data(yaml).context("Failed to parse front matter data")?;
    } else {
        return Err(anyhow::anyhow!("Unable to find frontmatter as the first item in the markdown. Make sure to include it."));
    }

    // Turn into HTML
    let mut buf = vec![];
    format_html(root, &options, &mut buf).context("Unable to turn html syntax tree into html")?;
    let content = String::from_utf8(buf)
        .context("Failed to turn markdown parsing produced bytes into a valid String")?;

    Ok(Content::Blog(Blog {
        name,
        metadata,
        content,
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

fn render_blogs(blogs: &[Blog], tera: Arc<Tera>, config: &Config) -> Result<()> {
    let res = blogs
        .iter()
        .par_bridge()
        .map(|blog| -> Result<()> {
            let mut context = tera::Context::from_serialize(blog).context(
                "Unable to serialize blog object into a valid context for rendering templates",
            )?;
            context.insert("config", config);

            let output_dir = config.output.join("blog"); // output/blog
            std::fs::create_dir_all(&output_dir)
                .context("Unable to create output blog directory")?;

            let filename = blog.name.with_extension("html"); // post1.html
            let out_filepath = output_dir.join(&filename); // output/blog/post1.html

            // Render the template
            let rendered = tera
                .render("blog.html", &context)
                .context("Unable to render the blog post")?;

            std::fs::write(&out_filepath, rendered).context(format!(
                "Unable to write blog file into output destination ({})",
                out_filepath.to_string_lossy()
            ))?;
            Ok(())
        })
        .collect::<Result<()>>();
    res
}

fn render(content: AvailableContent, config: &Config) -> Result<()> {
    let tera = Arc::new(
        Tera::new(
            &config
                .input
                .join("templates")
                .join("*.html")
                .to_string_lossy(),
        )
        .context("Failed to initialize templating")?,
    );
    //   /// Template directory
    // "template_glob": "input/templates/*.html",

    // Cleanup output directory before rendering
    if config.output.exists() {
        std::fs::remove_dir_all(&config.output)
            .context("Unable to remove completely the output directory")?;
    }

    // Render blogs
    render_blogs(&content.blogs, tera.clone(), config)?;

    // Render index
    let mut context = tera::Context::from_serialize(content)
        .context("Unable to create context from config for rendering index")?;
    context.insert("config", &config);

    let rendered_index = tera.render("index.html", &context)?;
    std::fs::write(config.output.join("index.html"), rendered_index)
        .context("Failed to write the rendered index page")?;
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

pub fn generate(config: Config) -> Result<()> {
    // Render
    render(collect_content(&config)?, &config)?;

    // Include extras
    include_extras(config)
}
