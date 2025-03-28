use eyre::anyhow;
use num_bigint::{BigUint, ToBigUint};

use std::collections::BTreeMap;

pub const START_INDEX: usize = 128;

pub struct Lookup {
    pub global_start_index: BigUint,
    scratchpad: BTreeMap<usize, BigUint>,
    // indices are shifted down by START_INDEX
    table: Vec<BigUint>,
}

impl Lookup {
    pub fn new() -> Self {
        Self {
            scratchpad: BTreeMap::new(),
            table: Vec::new(),
            global_start_index: START_INDEX.to_biguint().unwrap(),
        }
    }

    pub fn record(&mut self, index: usize, value: &BigUint) -> eyre::Result<()> {
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
        let scratchpad = std::mem::take(&mut self.scratchpad);
        let mut first = true;
        for (index, value) in scratchpad.into_iter() {
            if index - START_INDEX != self.table.len() {
                return Err(if first {
                    anyhow!("lookup table not complete before {}", index)
                } else {
                    anyhow!("index {} not consecutive", index)
                });
            }

            self.table.push(value);
            first = false;
        }

        tracing::debug!("lookup table expanded to {} entries", self.table.len());
        Ok(())
    }

    pub fn is_on(&self) -> bool {
        !self.table.is_empty()
    }

    pub fn get(&self, index: usize) -> eyre::Result<BigUint> {
        let idx = index
            .checked_sub(START_INDEX)
            .ok_or_else(|| anyhow!("index {} is too small", index))?;

        if idx >= self.table.len() {
            return Err(anyhow!("index {} not found", index));
        }

        Ok(self.table[idx].clone())
    }

    pub fn get_scratchpad_size(&self) -> usize {
        self.scratchpad.len()
    }

    pub fn get_table_size(&self) -> usize {
        self.table.len()
    }
}
