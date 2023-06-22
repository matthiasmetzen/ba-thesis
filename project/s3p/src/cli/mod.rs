use std::path::{Path, PathBuf};

use clap::{Args, Parser};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub(crate) struct CliArgs {
    /// Name of the person to greet
    #[command(flatten)]
    pub config: ConfigFile,
}

/// Arguments for loading configuration from file
#[derive(Args, Debug, Clone)]
#[group(required = false)]
#[command(about)]
pub(crate) struct ConfigFile {
    #[arg(short, long)]
    pub config_file: Option<PathBuf>,
    #[arg(short, long)]
    pub regenerate: bool,
    #[arg(short, long)]
    pub generate_if_missing: bool,
}
