use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use mub::Config;


#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[arg(default_value = "config.json")]
    config: PathBuf
}


fn main() -> Result<()>{
    let cli = Cli::parse();
    let config = Config::try_load(cli.config)?;
    mub::generate(config)?;
    Ok(())
}
