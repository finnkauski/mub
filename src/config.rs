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
