use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{read_dir, read_to_string, DirEntry, File},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result};
use config::Config;
use minijinja::{context, Environment};
use rayon::prelude::*;
use serde::Serialize;
use types::{
    AvailableContent, Content, ContentKind, LocationData, PhotoProject, Post, PostSourceKind,
    SearchableDoc,
};

const POSTS_DIR: &str = "posts";
const PHOTO_DIR: &str = "photos";

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

fn try_parse_post(filepath: PathBuf) -> Result<Post> {
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
    let mut name: PathBuf = filepath
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

    // TODO: these should be well defined fields with a proper metadata backing struct
    // to deserialize
    let publish = metadata
        .get("publish")
        .and_then(|val| val.parse().ok())
        .unwrap_or(false);
    if let Some(new_name) = metadata.get("name").map(PathBuf::from) {
        name = new_name;
    }

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
        name,
        publish,
        metadata,
        text,
        html,
        raw,
    })
}

fn try_parse_photo_project(directory: PathBuf) -> Result<Content> {
    assert!(
        directory.is_dir(),
        "Found a DirEntry in photo project parsing that isn't a directory"
    );
    // NOTE(art): assumes all projects are written in html
    // TODO(art): should support both html and markdown
    let mut content_file = directory.clone();
    content_file.push("post.html");

    let post = try_parse_post(content_file)?;
    let publish = post.publish;

    let image_locations = read_dir(&directory)
        .context("Unable to read images in project")?
        .par_bridge()
        .filter_map(|entry: Result<DirEntry, _>| -> Option<PathBuf> {
            let entry = entry.ok()?;
            let extension = entry
                .path()
                .extension()
                .map(|ext| ext.to_string_lossy().to_string());
            if let Some("jpg") = extension.map(|s| s.to_lowercase()).as_deref() {
                return Some(entry.path());
            }
            None
        })
        .try_fold(Vec::new, |mut acc, image| -> Result<Vec<PathBuf>> {
            acc.push(image);
            Ok(acc)
        })
        .try_reduce(Vec::new, |mut a, mut b| -> Result<Vec<PathBuf>> {
            a.append(&mut b);
            Ok(a)
        })?;

    let images = image_locations
        .par_iter()
        .map(|p| -> Result<String> {
            p.file_name()
                .context("Unable to produce file name")
                .map(|os_str| os_str.to_string_lossy().to_string())
        })
        .collect::<Result<Vec<String>>>()?;

    Ok(Content {
        location: LocationData::PhotoProject { directory },
        value: ContentKind::PhotoProject(PhotoProject {
            post,
            publish,
            images,
            image_locations,
        }),
    })
}

fn render_post<S>(post: &Post, templates: Arc<Environment>, config: &Config, data: S) -> Result<()>
where
    S: Serialize,
{
    // Create Posts directory
    let output_dir = config.output.join(POSTS_DIR); // output/posts
    std::fs::create_dir_all(&output_dir).context("Unable to create post output directory")?;

    let out_name = post.name.with_extension("html");

    let out_filepath = output_dir.join(&out_name); // output/pages/post1.html

    // Render the template
    let context = context!(data => data, ..context!(config));
    let template = post
        .metadata
        .get("template")
        .cloned()
        .unwrap_or("post.html".into());
    let rendered = templates
        .get_template(&template)?
        .render(&context)
        .context("Unable to render the post")?;

    let mut writer =
        BufWriter::new(File::create(&out_filepath).context("Unable to create a file for a post.")?);
    writer.write_all(rendered.as_bytes()).context(format!(
        "Unable to write post file into output destination ({})",
        out_filepath.to_string_lossy()
    ))?;
    Ok(())
}

fn render_posts(posts: &[Post], templates: Arc<Environment>, config: &Config) -> Result<()> {
    posts
        .iter()
        .par_bridge()
        .filter(|post| post.publish)
        .map(|post| render_post(post, templates.clone(), config, post))
        .collect::<Result<()>>()
}

fn render_photo_projects(
    photo_projects: &[PhotoProject],
    templates: Arc<Environment>,
    config: &Config,
) -> Result<()> {
    // Create Photos directory output/photos
    std::fs::create_dir_all(config.output.join(PHOTO_DIR))
        .context("Unable to create photo output directory")?;

    photo_projects
        .iter()
        .par_bridge()
        .filter(|project| project.publish)
        .map(|project| -> Result<()> {
            project
                .image_locations
                .par_iter()
                .map(|src: &PathBuf| -> Result<()> {
                    let filename = src
                        .file_name()
                        .context("Cannot extract file name from image")?;

                    std::fs::copy(src, config.output.join(PHOTO_DIR).join(filename))
                        .context("Unable to copy image file into output directory")?;
                    Ok(())
                })
                .collect::<Result<()>>()?;
            render_post(&project.post, templates.clone(), config, project)?;
            Ok(())
        })
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

    // Render posts
    render_posts(&content.posts, templates.clone(), config)?;

    // Render projects
    render_photo_projects(&content.photo_projects, templates.clone(), config)?;

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
    let mut docs = content
        .posts
        .par_iter()
        .filter(|p| p.publish)
        .map(TryFrom::try_from)
        .collect::<Result<Vec<SearchableDoc>>>()?;
    let project_posts = content
        .photo_projects
        .par_iter()
        .filter(|p| p.publish)
        .map(|project| TryFrom::try_from(&project.post))
        .collect::<Result<Vec<SearchableDoc>>>()?;

    docs.extend(project_posts);
    serde_json::to_writer(writer, &docs)?;
    Ok(())
}

fn collect_content(config: &Config) -> Result<AvailableContent> {
    read_dir(config.input.join("content"))
        .context("Unable to read content directory")?
        .par_bridge()
        .map(|entry| -> Result<Content> {
            let entry = entry.context("Unable to retrieve an entry from the directory")?;
            let filepath = entry.path();
            if filepath.is_file() {
                let post = try_parse_post(filepath.clone())?;
                Ok(Content {
                    location: types::LocationData::Post { filepath },
                    value: ContentKind::Post(post),
                })
            } else {
                try_parse_photo_project(filepath)
            }
        })
        .try_fold(
            AvailableContent::default,
            |mut acc, content| -> Result<AvailableContent> {
                match content?.value {
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
                a.photo_projects.append(&mut b.photo_projects);
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
