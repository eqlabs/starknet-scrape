use alloy::providers::{Provider, ProviderBuilder};
use clap::Parser;
use serde_json::json;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use starknet_scrape::config::Config;

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
    #[arg(long, short = 'f', value_name = "n", default_value = "934457")]
    pub from_block: std::num::NonZeroU64,
    #[arg(long, short = 't', value_name = "n", default_value = "934467")]
    pub to_block: std::num::NonZeroU64,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    let raw_config = fs::read_to_string(&cli.config_file)?;
    let config: Config = toml::from_str(&raw_config)?;

    let rpc_url = config.pathfinder_rpc_url.parse()?;
    let provider = ProviderBuilder::new().on_http(rpc_url);
    let mut starknet_block_no = cli.from_block.get();
    let starknet_last_block = cli.to_block.get();
    let mut result = HashMap::new();
    while starknet_block_no <= starknet_last_block {
        let single: jsu::StateUpdate = provider
            .client()
            .request(
                "starknet_getStateUpdate",
                [json!({"block_number": starknet_block_no})],
            )
            .await?;
        for desc in single.state_diff.nonces {
            result.insert(desc.contract_address, desc.nonce);
        }

        starknet_block_no += 1;
    }

    let mut addresses = result.keys().collect::<Vec<_>>();
    addresses.sort_by(|a, b| {
        let al = a.len();
        let bl = b.len();
        let ord = al.cmp(&bl);
        if ord == Ordering::Equal {
            a.cmp(b)
        } else {
            ord
        }
    });
    for addr in addresses {
        println!("a: {}", addr);
        println!("n: {}", result.get(addr).unwrap());
    }

    Ok(())
}

pub mod jsu {
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    pub struct StateUpdate {
        pub state_diff: StateDiff,
    }

    #[derive(Deserialize, Debug)]
    pub struct StateDiff {
        pub nonces: Vec<NonceDesc>,
    }

    #[derive(Deserialize, Debug)]
    pub struct NonceDesc {
        pub contract_address: String,
        pub nonce: String,
    }
}
