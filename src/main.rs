use alloy::{
    consensus::transaction::TxEip4844Variant,
    primitives::{FixedBytes, address},
    providers::{Provider, ProviderBuilder},
    rpc::types::Filter,
    sol_types::SolEvent,
};
use clap::Parser;
use eyre::{ContextCompat, anyhow};
use num_bigint::BigUint;
use num_traits::{Num, ToPrimitive};
use serde::Deserialize;
use tokio::{
    task,
    time::{Duration, sleep},
};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use std::fs;
use std::io::{BufRead, BufReader, LineWriter, Write};
use std::path::PathBuf;

use starknet_scrape::{
    blob_util::parse_str_to_blob_data,
    config::{Cli, Config},
    decomp::Decompressor,
    eth::StarknetCore::LogStateUpdate,
    packing::{
        v0_13_1::make_pack_const as make_pack_const1, v0_13_3::make_pack_const as make_pack_const3,
    },
    parser::StateUpdateParser,
    transform::Transformer,
};

/// `MAX_RETRIES` is the maximum number of retries on failed blob retrieval.
const MAX_RETRIES: usize = 5;
/// The interval in seconds to wait before retrying to fetch a blob.
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

fn parse_local(cache_dir: &PathBuf, dump: bool) -> eyre::Result<()> {
    let cache_dir = fs::canonicalize(cache_dir)?;
    let seq_mask = cache_dir.join("*.seq");
    for raw_entry in glob::glob(seq_mask.to_str().context("invalid cache dir")?)? {
        let mut entry = raw_entry?;
        let file = fs::File::open(&entry)?;
        // it isn't strictly necessary to load the file first, but it
        // does separate input from parsing errors; also, the other
        // (and primary) caller of uncond_parse has the data in
        // memory...
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

        let cond_target = if dump {
            entry.set_extension("unc");
            Some(entry)
        } else {
            None
        };
        uncond_parse(elements, cond_target)?;
    }

    Ok(())
}

fn uncond_parse(seq: Vec<BigUint>, dump_target: Option<PathBuf>) -> eyre::Result<()> {
    if seq.len() == 0 {
        return Err(anyhow!("empty sequence"));
    }

    // This isn't really a _guaranteed_ check for the compressed
    // format, but the update would have to be pretty much empty to
    // have all high header bits clear... Maybe we should lock the
    // format change the first time we encounter it?
    let (seq, unpacker) = if seq[0].to_usize().is_some() {
        (seq, make_pack_const1())
    } else {
        let (unc, tail_size) = Decompressor::decompress(seq.into_iter())?;
        tracing::debug!(
            "{} zeroes after decompressed sequence of {} words",
            tail_size,
            unc.len()
        );

        if let Some(ref target) = dump_target {
            uncond_dump(&unc, target)?;
        }

        (unc, make_pack_const3())
    };

    let anno_dump: Box<dyn Write> = if let Some(mut target) = dump_target {
        target.set_extension("anno");
        let file = fs::File::create(target)?;
        Box::new(LineWriter::new(file))
    } else {
        Box::new(std::io::empty())
    };

    let (_state_diff, tail_size) = StateUpdateParser::parse(seq.into_iter(), unpacker, anno_dump)?;
    tracing::debug!("{} zeroes after parsed blob", tail_size);
    Ok(())
}

fn uncond_dump(seq: &Vec<BigUint>, target: &PathBuf) -> eyre::Result<()> {
    tracing::debug!("dumping {:?}...", target);
    let file = fs::File::create(target)?;
    let mut writer = LineWriter::new(file);
    for el in seq.iter() {
        writeln!(writer, "{:#x}", el)?;
    }

    Ok(())
}

struct App<P> {
    cli: Cli,
    provider: P,
    filter_base: Filter,
    downloader: Downloader,
    dumper: Dumper,
}

impl<P> App<P>
where
    P: Provider,
{
    pub fn new(cli: Cli, config: Config, provider: P) -> eyre::Result<Self> {
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
        })
    }

    pub async fn cycle(&mut self, from_block: u64, to_block: u64) -> eyre::Result<()> {
        let filter = self
            .filter_base
            .clone()
            .from_block(from_block)
            .to_block(to_block);
        let logs = self.provider.get_logs(&filter).await?;
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
            let opt_outer = self.provider.get_transaction_by_hash(tx_hash).await?;
            let outer = opt_outer.context("logged tx not found")?;
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

    fn cond_parse(&mut self, seq: Vec<BigUint>) -> eyre::Result<()> {
        if self.cli.parse {
            // dumping uncompressed sequences isn't supported while
            // fetching because pruning them isn't supported
            uncond_parse(seq, None)
        } else {
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    start_logger(LevelFilter::INFO);

    let cli = Cli::parse();
    let raw_config = fs::read_to_string(&cli.config_file)?;
    let config: Config = toml::from_str(&raw_config)?;

    fs::create_dir_all(&config.cache_dir)?;

    if cli.parse_local {
        parse_local(&config.cache_dir, cli.dump)?;
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
    let mut app = App::new(cli, config, provider)?;
    loop {
        app.cycle(from_block, to_block).await?;
        if single_shot {
            tracing::info!("done");
            return Ok(());
        }

        tracing::info!("last processed {}", to_block);
        from_block = to_block + 1;
        to_block = from_block + block_count - 1;
    }
}

struct Dumper {
    dump: bool,
    cache_dir: PathBuf,
    prune: bool,
    doomed: Option<PathBuf>,
    cur_block_no: Option<u64>,
    cur_block_repeat: u32, // 0 when cur_block_no not set
}

impl Dumper {
    pub fn new(dump: bool, cache_dir: PathBuf, prune: bool) -> Self {
        Self {
            dump,
            cache_dir,
            prune,
            doomed: None,
            cur_block_no: None,
            cur_block_repeat: 0,
        }
    }

    pub fn set_block_no(&mut self, cur_block_no: u64) -> eyre::Result<()> {
        if let Some(last_block_no) = self.cur_block_no {
            if cur_block_no < last_block_no {
                return Err(anyhow!(
                    "block {} followed by {}",
                    cur_block_no,
                    last_block_no
                ));
            } else if cur_block_no == last_block_no {
                // some blocks (e.g. 19433007, 19433041) have multiple
                // update transactions (w/ different blobs), therefore
                // not all transformed blobs can be identified simply
                // by the block number
                self.cur_block_repeat += 1;
                return Ok(());
            } else {
                self.cur_block_repeat = 0;
            }
        }

        self.cur_block_no = Some(cur_block_no);
        Ok(())
    }

    pub fn make_dump_target(&self, ext: &str) -> eyre::Result<PathBuf> {
        let block_no = self
            .cur_block_no
            .ok_or_else(|| anyhow!("internal error: Dumper.cur_block_no not set"))?;
        let sub_block = if self.cur_block_repeat > 0 {
            format!("-{}", self.cur_block_repeat)
        } else {
            String::new()
        };
        let name = format!("{}{}.{}", block_no, sub_block, ext);
        Ok(self.cache_dir.join(name))
    }

    pub fn cond_dump(&mut self, seq: &Vec<BigUint>) -> eyre::Result<()> {
        if self.dump {
            let seq_path = self.make_dump_target("seq")?;
            uncond_dump(seq, &seq_path)?;

            if self.prune {
                if let Some(doomed) = &self.doomed {
                    fs::remove_file(doomed)?;
                }

                self.doomed = Some(seq_path);
            }
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct JsonResponse {
    commitment: String,
    data: String,
}

struct Downloader {
    client: reqwest::Client,
    blob_url_base: String,
    transformer: Transformer,
    save: bool,
    cache_dir: PathBuf,
    prune: bool,
    doomed: Option<PathBuf>,
}

impl Downloader {
    pub fn new(
        client: reqwest::Client,
        blob_url_base: String,
        save: bool,
        cache_dir: PathBuf,
        prune: bool,
    ) -> Self {
        let transformer = Transformer::new();
        Self {
            client,
            blob_url_base,
            transformer,
            save,
            cache_dir,
            prune,
            doomed: None,
        }
    }

    async fn repeat_get(&mut self, url: &str) -> eyre::Result<reqwest::Response> {
        for attempt in 1..=MAX_RETRIES {
            match self.client.get(url).send().await {
                Ok(response) => {
                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!("attempt {}: GET error: {:?}", attempt, e);
                    sleep(Duration::from_secs(FAILED_FETCH_RETRY_INTERVAL_S)).await;
                }
            }
        }

        Err(anyhow!("can't get blob"))
    }

    pub async fn download(&mut self, blob_hash: &FixedBytes<32>) -> eyre::Result<Vec<BigUint>> {
        let url = format!(
            "{}0x{}",
            self.blob_url_base,
            hex::encode(blob_hash.as_slice())
        );
        let response = self.repeat_get(&url).await?;
        let status = response.status();
        let text = response.text().await?;
        let json_response = match serde_json::from_str::<JsonResponse>(&text) {
            Ok(rsp) => rsp,
            Err(e) => {
                tracing::warn!(
                    "URL {} got {} with invalid JSON: {} ({:?})",
                    url,
                    status,
                    text,
                    e
                );
                return Err(e.into());
            }
        };
        if self.save {
            let target_name = format!("{}.blob", json_response.commitment);
            let target_path = self.cache_dir.join(target_name);
            let mut target = fs::File::create(&target_path)?;
            target.write_all(json_response.data.as_bytes())?;
            if self.prune {
                if let Some(doomed) = &self.doomed {
                    fs::remove_file(doomed)?;
                }

                self.doomed = Some(target_path);
            }
        }

        // copying thousands of constants is inefficient - but so
        // is locking access to them...
        let transformer = self.transformer.clone();
        let words = parse_str_to_blob_data(&json_response.data)?;
        let transformed = task::spawn_blocking(move || transformer.transform(&words)).await?;
        Ok(transformed)
    }
}
