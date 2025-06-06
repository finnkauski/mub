use std::{env::args, path::PathBuf, process::exit};

use anyhow::Result;
use mub::config::Config;

fn main() -> Result<()> {
    let args: Vec<String> = args().collect();
    if args.len() != 2 {
        println!("Usage: mub config.json");
        exit(1);
    }
    let config_path: PathBuf = args[1].parse().unwrap_or_else(|e| {
        eprintln!("Unable to parse config path: {path}", path = args[1]);
        eprintln!("{e}");
        exit(1);
    });
    let config = Config::try_load(&config_path).unwrap_or_else(|e| {
        eprintln!("Unable to load config [{config_path:?}]");
        eprintln!("{e}");
        exit(1);
    });
    mub::generate(config)
}
