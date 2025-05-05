use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use mub::config::Config;


#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[arg(default_value = "config.json")]
    config: PathBuf
}


fn main() -> Result<()>{
    mub::generate(Config::try_load(Cli::parse().config)?)
}
