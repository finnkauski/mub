use std::{
    collections::HashMap,
    fs::{read_dir, read_to_string, DirEntry},
    path::{Path, PathBuf},
};
use tera::Tera;

use anyhow::{Context, Result};
use comrak::{
    format_html,
    nodes::{AstNode, NodeValue},
    parse_document, Arena, Options,
};
use config::Config;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use types::{Content, ContentKind, Metadata, RenderedItem};
use yaml_rust2::{Yaml, YamlLoader};

pub mod config;
pub(crate) mod types;

fn read_markdown(filepath: &Path) -> Result<String> {
    read_to_string(filepath).context("Unable to read content of a file to string.")
}

fn get_name(filepath: &Path) -> Result<PathBuf> {
    Ok(filepath
        .with_extension("")
        .file_name()
        .context("Unable to fetch filename for file file when parsing the filepath")?
        .into())
}

fn parse_markdown<'arena, 'opt>(
    arena: &'arena Arena<AstNode<'arena>>,
    content: &str,
) -> Result<(
    &'arena comrak::arena_tree::Node<'arena, std::cell::RefCell<comrak::nodes::Ast>>,
    Options<'opt>,
    Metadata,
)> {
    let mut options = Options::default();
    // Parse the markdown into an AST
    options.extension.front_matter_delimiter = Some(String::from("---"));
    let root = parse_document(arena, content, &options);

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
    Ok((root, options, metadata))
}

fn try_parse_content(entry: DirEntry) -> Result<Content> {
    // Read markdown
    let filepath = entry.path();
    let content = read_markdown(&filepath)?;
    let name = get_name(&filepath)?;

    // Parse Markdown
    let arena = Arena::new();
    let (root, options, metadata) = parse_markdown(&arena, &content)?;

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

fn render(content: Content, config: &Config) -> Result<RenderedItem> {
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
