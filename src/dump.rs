use eyre::anyhow;
use num_bigint::BigUint;

use std::fs;
use std::io::{LineWriter, Write};
use std::path::PathBuf;

pub fn uncond_dump(seq: &Vec<BigUint>, target: &PathBuf) -> eyre::Result<()> {
    tracing::debug!("dumping {:?}...", target);
    let file = fs::File::create(target)?;
    let mut writer = LineWriter::new(file);
    for el in seq.iter() {
        writeln!(writer, "{:#x}", el)?;
    }

    Ok(())
}

pub struct Dumper {
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
