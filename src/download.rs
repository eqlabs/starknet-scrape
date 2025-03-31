use alloy::primitives::FixedBytes;
use eyre::anyhow;
use num_bigint::BigUint;
use serde::Deserialize;
use tokio::{
    task,
    time::{Duration, sleep},
};

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::blob_util::parse_str_to_blob_data;
use crate::transform::Transformer;

/// `MAX_RETRIES` is the maximum number of retries on failed blob retrieval.
const MAX_RETRIES: usize = 5;
/// The interval in seconds to wait before retrying to fetch a blob.
const FAILED_FETCH_RETRY_INTERVAL_S: u64 = 10;

#[derive(Deserialize)]
struct JsonResponse {
    commitment: String,
    data: String,
}

pub struct Downloader {
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
                    let status = response.status();
                    let status_code = status.as_u16();
                    // 10x statuses aren't really expected, but just
                    // in case of a strange server...
                    if status_code < 200 {
                        return Err(anyhow!("{} got status {}", url, status));
                    }

                    if status_code >= 400 {
                        tracing::warn!("attempt {}: GET error status: {:?}", attempt, status);
                        sleep(Duration::from_secs(FAILED_FETCH_RETRY_INTERVAL_S)).await;
                    } else {
                        return Ok(response);
                    }
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
        let text = response.text().await?;
        let json_response = match serde_json::from_str::<JsonResponse>(&text) {
            Ok(rsp) => rsp,
            Err(e) => {
                tracing::warn!("URL {} has invalid JSON: {} ({:?})", url, text, e);
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
