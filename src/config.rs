use clap::Parser;
use serde::Deserialize;

use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(
        long,
        value_name = "config.toml",
        long_help = "Path to config file (must exist).",
        default_value = "config.toml"
    )]
    pub config_file: PathBuf,
    #[arg(long, short = 'l', default_value = "false")]
    pub parse_local: bool,
    #[arg(long, short = '0', default_value = "false")]
    pub no_connect: bool,
    #[arg(long, short = '1', default_value = "false")]
    pub single_shot: bool,
    #[arg(long, short = 'd', default_value = "false")]
    pub dump: bool,
    #[arg(long, short = 'f', value_name = "n", default_value = "19427723")]
    pub from_block: std::num::NonZeroU64,
    #[arg(long, short = 'c', value_name = "n", default_value = "1")]
    pub block_count: std::num::NonZeroU64,
    #[arg(long, short = 'p', default_value = "false")]
    pub parse: bool,
    #[arg(long, short = 's', default_value = "false")]
    pub save: bool,
    #[arg(long, short = 'u', default_value = "false")]
    pub prune: bool,
}

#[derive(Deserialize)]
pub struct Config {
    pub rpc_url: String,
    pub blob_url_base: String,
    pub cache_dir: PathBuf,
}
