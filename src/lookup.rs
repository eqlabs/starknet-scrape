use eyre::{ContextCompat, anyhow};
use num_bigint::{BigUint, ToBigUint};
use redb::{Database, ReadableTableMetadata, TableDefinition, TableError, WriteTransaction};

use std::collections::BTreeMap;
use std::path::PathBuf;

pub const START_INDEX: u64 = 128;

const PHASE_CHANGE: TableDefinition<&str, u64> = TableDefinition::new("phase_change");
const STATEFUL_COMPRESSION_START: &str = "stateful";
const STATEFUL_COMPRESSION_CREST: &str = "crest";

const LOOKUP_TABLE: TableDefinition<u64, &[u8] /* BigUint */> =
    TableDefinition::new("lookup_table");

pub struct Lookup {
    pub global_start_index: BigUint,
    scratchpad: BTreeMap<u64, BigUint>,
    cur_block_no: Option<u64>,
    db: Database,
}

impl Lookup {
    pub fn new(db_file: &PathBuf) -> eyre::Result<Self> {
        let db = Database::create(db_file)?;
        Ok(Self {
            global_start_index: START_INDEX.to_biguint().unwrap(),
            scratchpad: BTreeMap::new(),
            cur_block_no: None,
            db,
        })
    }

    pub fn set_block_no(&mut self, cur_block_no: u64) {
        self.cur_block_no = Some(cur_block_no)
    }

    pub fn record(&mut self, index: u64, value: &BigUint) -> eyre::Result<()> {
        if index < START_INDEX {
            return Err(anyhow!("index {} too small", index));
        }

        if let Some(old) = self.scratchpad.insert(index, value.clone()) {
            // reject invalid input data
            self.scratchpad.insert(index, old);
            Err(anyhow!("index repeated"))
        } else {
            Ok(())
        }
    }

    // clears scratchpad (even) on error
    pub fn expand(&mut self) -> eyre::Result<()> {
        if let Some(crest) = self.get_crest()? {
            let block_no = self.get_cur_block_no()?;
            if block_no <= crest {
                tracing::info!(
                    "stateful compression mapping had already been persisted up to block {} and won't be updated for block {} again",
                    crest,
                    block_no
                );
                // could be checked to be the same, though...
                self.scratchpad.clear();
                Ok(())
            } else {
                self.do_expand()
            }
        } else {
            self.do_expand()
        }
    }

    fn do_expand(&mut self) -> eyre::Result<()> {
        let scratchpad = std::mem::take(&mut self.scratchpad);
        let mut first = true;
        let mut sz = self.get_table_size()?;
        let mut txn = self.db.begin_write()?;
        for (index, value) in scratchpad.into_iter() {
            if index - START_INDEX != sz {
                return Err(if first {
                    anyhow!("lookup table not complete before {}", index)
                } else {
                    anyhow!("index {} not consecutive", index)
                });
            }

            Self::set_expansion(&mut txn, index, value)?;
            sz += 1;
            first = false;
        }

        self.set_stateful_compression(&mut txn)?;
        txn.commit()?;

        tracing::debug!("lookup table expanded to {} entries", sz);
        Ok(())
    }

    pub fn is_on(&self) -> eyre::Result<bool> {
        let txn = self.db.begin_read()?;
        match txn.open_table(PHASE_CHANGE) {
            Err(TableError::TableDoesNotExist(_)) => {
                return Ok(false);
            }
            Err(err) => {
                return Err(err.into());
            }
            Ok(phase_change) => {
                if let Some(found) = phase_change.get(STATEFUL_COMPRESSION_START)? {
                    let start_no = found.value();
                    let block_no = self.get_cur_block_no()?;
                    if block_no < start_no {
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
                }
            }
        };

        let empty = match txn.open_table(LOOKUP_TABLE) {
            Err(TableError::TableDoesNotExist(_)) => true,
            Err(err) => {
                return Err(err.into());
            }
            Ok(table) => table.is_empty()?,
        };
        Ok(!empty)
    }

    pub fn get(&self, index: u64) -> eyre::Result<BigUint> {
        if index < START_INDEX {
            return Err(anyhow!("index {} is too small", index));
        }

        let txn = self.db.begin_read()?;
        let table = txn.open_table(LOOKUP_TABLE)?;
        if let Some(found) = table.get(index)? {
            let bytes = found.value();
            let n = BigUint::from_bytes_be(bytes);
            Ok(n)
        } else {
            Err(anyhow!("index {} not found", index))
        }
    }

    pub fn get_scratchpad_size(&self) -> usize {
        self.scratchpad.len()
    }

    pub fn get_table_size(&self) -> eyre::Result<u64> {
        let txn = self.db.begin_read()?;
        let l = match txn.open_table(LOOKUP_TABLE) {
            Err(TableError::TableDoesNotExist(_)) => 0,
            Err(err) => {
                return Err(err.into());
            }
            Ok(table) => table.len()?,
        };
        Ok(l)
    }

    fn get_cur_block_no(&self) -> eyre::Result<u64> {
        self.cur_block_no.context("Lookup.cur_block_no not set")
    }

    fn get_crest(&self) -> eyre::Result<Option<u64>> {
        let txn = self.db.begin_read()?;
        let opt_crest = match txn.open_table(PHASE_CHANGE) {
            Err(TableError::TableDoesNotExist(_)) => None,
            Err(err) => {
                return Err(err.into());
            }
            Ok(phase_change) => {
                if let Some(found) = phase_change.get(STATEFUL_COMPRESSION_CREST)? {
                    let crest = found.value();
                    Some(crest)
                } else {
                    None
                }
            }
        };
        Ok(opt_crest)
    }

    fn set_stateful_compression(&self, txn: &mut WriteTransaction) -> eyre::Result<()> {
        let block_no = self.get_cur_block_no()?;
        let mut phase_change = txn.open_table(PHASE_CHANGE)?;
        let updated = {
            let opt_old = phase_change.insert(STATEFUL_COMPRESSION_CREST, block_no)?;
            opt_old.is_none()
        };
        if updated {
            phase_change.insert(STATEFUL_COMPRESSION_START, block_no)?;
            // not asserting STATEFUL_COMPRESSION_START wasn't set
            // before because older code actually did set it earlier
        }

        Ok(())
    }

    fn set_expansion(txn: &mut WriteTransaction, index: u64, value: BigUint) -> eyre::Result<()> {
        let bytes = value.to_bytes_be();
        let mut table = txn.open_table(LOOKUP_TABLE)?;
        let opt_old = table.insert(index, bytes.as_slice())?;
        // caller ensures indices are in order, IOW they don't repeat
        assert!(opt_old.is_none());
        Ok(())
    }
}
