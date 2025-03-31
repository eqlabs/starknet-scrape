use clap::Parser;
use serde::Deserialize;

use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(
        long,
        value_name = "config.toml",
        long_help = "Path to config file (must exist)",
        default_value = "config.toml"
    )]
    pub config_file: PathBuf,
    #[arg(
        long,
        short = 'f',
        value_name = "n",
        long_help = "First block to search for",
        default_value = "19427723"
    )]
    pub from_block: std::num::NonZeroU64,
    #[arg(
        long,
        short = 'c',
        value_name = "n",
        long_help = "Number of blocks to check in one `eth_getLogs` call",
        default_value = "1"
    )]
    pub block_count: std::num::NonZeroU64,
    #[arg(
        long,
        short = 'p',
        long_help = "Actually process downloaded blobs",
        default_value = "false"
    )]
    pub parse: bool,
    #[arg(
        long,
        short = 'd',
        long_help = "Dump results of various state update processing stages into the cache directory",
        default_value = "false"
    )]
    pub dump: bool,
    #[arg(
        long,
        short = 'l',
        long_help = "Before connecting to Ethereum, scan the cache directory for previously-dumped unparsed state updates and parse them",
        default_value = "false"
    )]
    pub parse_local: bool,
    #[arg(
        long,
        short = 'a',
        long_help = "Instead of connecting to Ethereum, scan the cache directory for previously-dumped uncompressed state updates and parse them, also annotating the data with the parser's interpretation",
        default_value = "false"
    )]
    pub annotate_only: bool,
    #[arg(
        long,
        short = '0',
        long_help = "Do not connect to Ethereum",
        default_value = "false"
    )]
    pub no_connect: bool,
    #[arg(
        long,
        short = '1',
        long_help = "Call `eth_getLogs` just once, even if it does return data",
        default_value = "false"
    )]
    pub single_shot: bool,
    #[arg(
        long,
        short = 's',
        long_help = "Save downloaded blobs into the cache directory before processing them",
        default_value = "false"
    )]
    pub save: bool,
    #[arg(
        long,
        short = 'j',
        long_help = "Convert parsed blobs to JSON and save it into the cache directory",
        default_value = "false"
    )]
    pub json: bool,
    #[arg(
        long,
        short = 'u',
        long_help = "When saving / dumping data, remove files for already fully-processed updates",
        default_value = "false"
    )]
    pub prune: bool,
}

#[derive(Deserialize)]
pub struct Config {
    pub rpc_url: String,
    pub blob_url_base: String,
    pub cache_dir: PathBuf,
    pub pathfinder_rpc_url: String,
}
