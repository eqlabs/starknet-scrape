use alloy::{
    consensus::transaction::TxEip4844Variant,
    primitives::{FixedBytes, address},
    providers::{Provider, ProviderBuilder},
    rpc::types::{Filter, Transaction},
    sol_types::SolEvent,
};
use clap::Parser;
use eyre::{ContextCompat, WrapErr, anyhow};
use num_bigint::BigUint;
use num_traits::{Num, ToPrimitive};
use tokio::time::{Duration, sleep};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use std::cell::RefCell;
use std::fs;
use std::io::{BufRead, BufReader, LineWriter, Write};
use std::path::PathBuf;
use std::rc::Rc;

use starknet_scrape::{
    config::{Cli, Config},
    decomp::Decompressor,
    download::Downloader,
    dump::{Dumper, uncond_dump},
    eth::StarknetCore::LogStateUpdate,
    lookup::Lookup,
    packing::{
        v0_13_1::make_pack_const as make_pack_const1, v0_13_3::make_pack_const as make_pack_const3,
    },
    parser::StateUpdateParser,
};

/// `MAX_RETRIES` is the maximum number of retries on failed tx retrieval.
const MAX_RETRIES: usize = 5;
/// The interval in seconds to wait before retrying to fetch tx.
const FAILED_FETCH_RETRY_INTERVAL_S: u64 = 10;

fn start_logger(default_level: LevelFilter) {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter
            .add_directive("alloy=off".parse().unwrap())
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("reqwest=off".parse().unwrap()),
        _ => EnvFilter::default()
            .add_directive(default_level.into())
            .add_directive("alloy=off".parse().unwrap())
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("reqwest=off".parse().unwrap()),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn parse_local(
    lookup: Rc<RefCell<Lookup>>,
    cache_dir: &PathBuf,
    annotate: bool,
    dump: bool,
) -> eyre::Result<()> {
    let cache_dir = fs::canonicalize(cache_dir)?;
    let mask = if annotate { "*.unc" } else { "*.seq" };
    let seq_mask = cache_dir.join(mask);
    for raw_entry in glob::glob(seq_mask.to_str().context("invalid cache dir")?)? {
        let mut entry = raw_entry?;
        let file = fs::File::open(&entry)?;
        let mut elements = Vec::new();
        for res in BufReader::new(file).lines() {
            let ln = res?;
            let (data, radix) = match ln.strip_prefix("0x") {
                Some(tail) => (tail, 16),
                None => (ln.as_str(), 10),
            };
            let el = BigUint::from_str_radix(data, radix)
                .map_err(|_| anyhow!("invalid integer {}", ln))?;
            elements.push(el);
        }

        if dump {
            entry.set_extension("unc");
        };
        do_parse(
            lookup.clone(),
            elements,
            annotate,
            dump,
            dump,
            !annotate,
            entry,
        )?;
    }

    Ok(())
}

fn do_parse(
    lookup: Rc<RefCell<Lookup>>,
    seq: Vec<BigUint>,
    uncompressed: bool,
    dump_uncompressed: bool,
    dump_annotated: bool,
    save_json: bool,
    dump_target: PathBuf,
) -> eyre::Result<()> {
    if seq.len() == 0 {
        return Err(anyhow!("empty sequence"));
    }

    let mut dump_target = dump_target;

    // uncompressed means the sequence had been compressed previously,
    // i.e. has the v0_13_3 format
    let (seq, unpacker) = if uncompressed {
        (seq, make_pack_const3())
    } else {
        // This isn't really a _guaranteed_ check for the compressed
        // format, but the update would have to be pretty much empty
        // to have all high header bits clear... Maybe we should lock
        // the format change the first time we encounter it?
        if seq[0].to_usize().is_some() {
            (seq, make_pack_const1())
        } else {
            let (unc, tail_size) = Decompressor::decompress(seq.into_iter())?;
            tracing::debug!(
                "{} zeroes after decompressed sequence of {} words",
                tail_size,
                unc.len()
            );

            if dump_uncompressed {
                uncond_dump(&unc, &dump_target)?;
            }

            (unc, make_pack_const3())
        }
    };

    let anno_dump: Box<dyn Write> = if dump_annotated {
        dump_target.set_extension("anno");
        let file = fs::File::create(&dump_target)?;
        Box::new(LineWriter::new(file))
    } else {
        Box::new(std::io::empty())
    };

    let (state_diff, tail_size) =
        StateUpdateParser::parse(seq.into_iter(), unpacker, lookup, anno_dump)?;
    tracing::debug!("{} zeroes after parsed blob", tail_size);
    if save_json {
        dump_target.set_extension("json");
        let j = state_diff.to_json_state_diff();
        fs::write(dump_target, j.to_string())?;
    }

    Ok(())
}

struct App<P> {
    cli: Cli,
    provider: P,
    filter_base: Filter,
    downloader: Downloader,
    dumper: Dumper,
    lookup: Rc<RefCell<Lookup>>,
}

impl<P> App<P>
where
    P: Provider,
{
    pub fn new(
        cli: Cli,
        config: Config,
        provider: P,
        lookup: Rc<RefCell<Lookup>>,
    ) -> eyre::Result<Self> {
        let cache_dir = fs::canonicalize(&config.cache_dir)?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Accept",
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        let starknet_core = address!("0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4");
        let filter_base = Filter::new()
            .address(starknet_core)
            .event_signature(LogStateUpdate::SIGNATURE_HASH);

        let downloader = Downloader::new(
            client,
            config.blob_url_base,
            cli.save,
            cache_dir.clone(),
            cli.prune,
        );
        let dumper = Dumper::new(cli.dump, cache_dir.clone(), cli.prune);

        Ok(Self {
            cli,
            provider,
            filter_base,
            downloader,
            dumper,
            lookup,
        })
    }

    pub async fn cycle(&mut self, from_block: u64, to_block: u64) -> eyre::Result<()> {
        let filter = self
            .filter_base
            .clone()
            .from_block(from_block)
            .to_block(to_block);
        let logs = self
            .provider
            .get_logs(&filter)
            .await
            .context("can't get logs")?;
        tracing::info!("got {} log(s)", logs.len());
        if logs.is_empty() {
            // obviously it would be better to increase the range here
            // (unless we're already at the newest block), but that's
            // getting into the full dynamic range support...
            return Err(anyhow!("no logs found"));
        }

        for log in logs {
            let cur_block_no = log.block_number.context("block not set")?;
            self.dumper.set_block_no(cur_block_no)?;
            let decoded_log = LogStateUpdate::decode_log(&log.inner, true)?;
            tracing::debug!(
                "processing Ethereum block {} (Starknet {})...",
                cur_block_no,
                decoded_log.data.blockNumber
            );
            let tx_hash = log.transaction_hash.context("log has no tx hash")?;
            let outer = self.repeat_get_transaction(&tx_hash).await?;
            if let Some(signed) = outer.inner.as_eip4844() {
                if let TxEip4844Variant::TxEip4844(tx) = signed.tx() {
                    if tx.blob_versioned_hashes.is_empty() {
                        return Err(anyhow!("no blobs"));
                    }
                    let mut seq = Vec::new();
                    for blob in tx.blob_versioned_hashes.iter() {
                        let mut transformed = self.downloader.download(blob).await?;
                        seq.append(&mut transformed);
                    }
                    self.dumper.cond_dump(&seq)?;
                    self.cond_parse(seq)?;
                } else {
                    // this would in fact be ideal, but doesn't happen in
                    // practice...
                    return Err(anyhow!("tx already includes blob"));
                }
            } else {
                // this can actually happen for older txs (and
                // theoretically even newer ones, if Starknet switches to
                // calldata for some reason), but we aren't supporting
                // them yet...
                return Err(anyhow!("tx not EIP4844"));
            }
        }

        Ok(())
    }

    async fn repeat_get_transaction(
        &mut self,
        tx_hash: &FixedBytes<32>,
    ) -> eyre::Result<Transaction> {
        for attempt in 1..=MAX_RETRIES {
            match self.provider.get_transaction_by_hash(*tx_hash).await {
                Ok(opt_tx) => {
                    return opt_tx.ok_or_else(|| anyhow!("logged tx {} not found", tx_hash));
                }
                Err(e) => {
                    tracing::warn!(
                        "attempt {}: get_transaction_by_hash error: {:?}",
                        attempt,
                        e
                    );
                    sleep(Duration::from_secs(FAILED_FETCH_RETRY_INTERVAL_S)).await;
                }
            }
        }

        Err(anyhow!("can't get logged tx {}", tx_hash))
    }

    fn cond_parse(&mut self, seq: Vec<BigUint>) -> eyre::Result<()> {
        if self.cli.parse {
            do_parse(
                self.lookup.clone(),
                seq,
                false,
                self.cli.dump,
                false,
                self.cli.json,
                self.dumper.make_dump_target("unc")?,
            )
        } else {
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    start_logger(LevelFilter::INFO);

    let mut cli = Cli::parse();
    let raw_config = fs::read_to_string(&cli.config_file)?;
    let config: Config = toml::from_str(&raw_config)?;

    if cli.json && !cli.parse {
        tracing::info!("command-line option json implies parse");
        cli.parse = true;
    }
    if cli.annotate_only && (!cli.parse_local || !cli.dump || !cli.no_connect) {
        tracing::info!(
            "command-line option annotate-only implies options parse-local, dump and no-connect"
        );
        cli.parse_local = true;
        cli.dump = true;
        cli.no_connect = true;
    }

    fs::create_dir_all(&config.cache_dir)?;
    let lookup = Rc::new(RefCell::new(Lookup::new()));

    if cli.parse_local {
        parse_local(
            lookup.clone(),
            &config.cache_dir,
            cli.annotate_only,
            cli.dump,
        )?;
    }

    if cli.no_connect {
        tracing::info!("not connecting to RPC");
        return Ok(());
    }

    let rpc_url = config.rpc_url.parse()?;
    let provider = ProviderBuilder::new().on_http(rpc_url);

    let block_count = cli.block_count.get();
    let single_shot = cli.single_shot;
    let mut from_block = cli.from_block.get();
    let mut to_block = from_block + block_count - 1;
    let mut app = App::new(cli, config, provider, lookup.clone())?;
    loop {
        app.cycle(from_block, to_block).await?;
        if single_shot {
            tracing::info!("done");
            return Ok(());
        }

        tracing::info!("last checked {}", to_block);
        from_block = to_block + 1;
        to_block = from_block + block_count - 1;
    }
}
