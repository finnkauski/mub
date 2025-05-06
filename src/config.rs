use std::{collections::HashMap, fmt::Display, fs::File, io::BufReader, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Input directory
    pub(crate) input: PathBuf,
    /// Output directory
    pub(crate) output: PathBuf,
    /// Site global metadata
    pub(crate) site: HashMap<String, serde_json::Value>,
}

impl Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::to_string_pretty(self).expect("Failed to Serialize a serializable config into a json object"))
    }
}

impl Config {
    pub fn try_load(path: PathBuf) -> Result<Self> {
        let file = File::open(path).context("Unable open the config file")?;
        serde_json::from_reader(BufReader::new(file)).context("Unable to deserialize config")
    }
}
