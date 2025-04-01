use eyre::{ContextCompat, WrapErr, anyhow};
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use crate::blob_util::parse_usize;
use crate::lookup::Lookup;
use crate::packing::PackConst;
use crate::state_diff::{BlockRange, ClassDeclaration, ContractUpdate, StateDiff, StorageUpdate};

enum LookupUsageState {
    Off,
    One,
    Expand,
    On,
}

pub struct StateUpdateParser<I> {
    current: I,
    lookup_usage_state: LookupUsageState,
    lookup: Rc<RefCell<Lookup>>,
    pack_const: PackConst,
    anno_dump: Box<dyn Write>,
    range: BlockRange,
}

impl<I> StateUpdateParser<I>
where
    I: Iterator<Item = BigUint>,
{
    pub fn parse(
        iter: I,
        unpacker: PackConst,
        lookup: Rc<RefCell<Lookup>>,
        anno_dump: Box<dyn Write>,
    ) -> eyre::Result<StateDiff> {
        let mut parser = Self {
            current: iter,
            lookup_usage_state: LookupUsageState::Off,
            lookup,
            pack_const: unpacker,
            anno_dump,
            range: Default::default(),
        };
        let contract_updates = parser.parse_contract_updates()?;
        let class_declarations = parser.parse_class_declarations()?;
        let n = parser.check_zero_tail()?;
        Ok(StateDiff {
            contract_updates,
            class_declarations,
            range: parser.range,
            tail_size: n,
        })
    }

    fn parse_contract_updates(&mut self) -> eyre::Result<Vec<ContractUpdate>> {
        let raw_num_contracts: BigUint = self
            .current
            .next()
            .context("Missing number of contract updates")?;
        writeln!(self.anno_dump, "{}", raw_num_contracts)?;
        let num_contracts =
            parse_usize(&raw_num_contracts).context("Parsing number of contract updates")?;
        (0..num_contracts)
            .map(|i| {
                self.parse_contract_update()
                    .with_context(|| format!("contract {} of {}", i, num_contracts))
            })
            .collect()
    }

    fn parse_contract_update(&mut self) -> eyre::Result<ContractUpdate> {
        let address = self.current.next().context("Missing contract address")?;
        writeln!(self.anno_dump, "a: {:#x}", address)?;
        if address.is_zero() {
            // majin-blob has a break on this condition, but hopefully
            // it doesn't happen on correct data...
            return Err(anyhow!("Zero address"));
        }

        let addr = match self.lookup_usage_state {
            LookupUsageState::Off => {
                if address == self.pack_const.one {
                    self.lookup_usage_state = LookupUsageState::One;
                }
                address
            }
            LookupUsageState::One => {
                if address == self.pack_const.two {
                    // switching to stateful compression
                    self.lookup_usage_state = LookupUsageState::Expand;
                    address
                } else if self.lookup.borrow().is_on() && (address > self.pack_const.two) {
                    // even if a statefully-compressed block didn't
                    // change 0x2's storage (and therefore its state
                    // diff doesn't contain the 0x2 address), it
                    // should still decompress its repeated storage
                    // keys and addresses (including this one)
                    let lookup = self.lookup.borrow();
                    let index = parse_usize(&address).context("Casting compressed address")?;
                    let addr = lookup.get(index)?;
                    self.lookup_usage_state = LookupUsageState::On;
                    addr
                } else {
                    self.lookup_usage_state = LookupUsageState::Off;
                    address
                }
            }
            LookupUsageState::Expand => {
                return Err(anyhow!(
                    "contract address encountered in unexpected lookup state Expand"
                ));
            }
            LookupUsageState::On => {
                let lookup = self.lookup.borrow();
                let index = parse_usize(&address).context("Casting compressed address")?;
                lookup.get(index)?
            }
        };

        let packed = self
            .current
            .next()
            .context("Missing contract packed data")?;
        let (class_flag, nonce, update_count) = self.pack_const.unpack_contract_update(&packed)?;
        writeln!(
            self.anno_dump,
            "{:#b} -> n: {}, c: {}, f: {}",
            packed, nonce, update_count, class_flag as i32
        )?;
        let new_class_hash = if class_flag {
            let hash = self.current.next().context("Missing new class hash")?;
            writeln!(self.anno_dump, "h: {:#x}", hash)?;
            Some(hash)
        } else {
            None
        };

        let storage_updates = (0..update_count)
            .map(|i| {
                self.parse_storage_update()
                    .with_context(|| format!("storage update {} of {}", i, update_count))
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        match self.lookup_usage_state {
            LookupUsageState::Expand => {
                let mut lookup = self.lookup.borrow_mut();
                lookup.expand()?;
                self.lookup_usage_state = LookupUsageState::On;
            }
            _ => (),
        }

        Ok(ContractUpdate {
            address: addr,
            nonce,
            new_class_hash,
            storage_updates,
        })
    }

    fn parse_storage_update(&mut self) -> eyre::Result<StorageUpdate> {
        let mut key = self.current.next().context("Missing storage address")?;
        writeln!(self.anno_dump, "k: {:#x}", key)?;
        let value = self.current.next().context("Missing storage value")?;
        writeln!(self.anno_dump, "v: {:#x}", value)?;
        match self.lookup_usage_state {
            LookupUsageState::Off => (),
            LookupUsageState::One => {
                let seq_no = key.to_u64().context("Casting 0x1 key")?;
                if let Some(old) = self.range.min_seq_no {
                    if seq_no < old {
                        tracing::warn!("0x1 keys not ordered: {} before {}", old, seq_no);
                        self.range.min_seq_no = Some(seq_no);
                    }
                } else {
                    self.range.min_seq_no = Some(seq_no);
                }
                if let Some(old) = self.range.max_seq_no {
                    if seq_no > old {
                        self.range.max_seq_no = Some(seq_no);
                    }
                } else {
                    self.range.max_seq_no = Some(seq_no);
                }
            }
            LookupUsageState::Expand => {
                if key.is_zero() {
                    tracing::debug!("global counter = {}", value);
                } else {
                    let mut lookup = self.lookup.borrow_mut();
                    let index = parse_usize(&value).context("Casting 0x2 value")?;
                    lookup.record(index, &key)?;
                }
            }
            LookupUsageState::On => {
                let lookup = self.lookup.borrow();
                if key >= lookup.global_start_index {
                    let index = parse_usize(&key).context("Casting compressed key")?;
                    key = lookup.get(index)?;
                }
            }
        }

        Ok(StorageUpdate { key, value })
    }

    fn parse_class_declarations(&mut self) -> eyre::Result<Vec<ClassDeclaration>> {
        let raw_num_decls: BigUint = self
            .current
            .next()
            .context("Missing number of class declarations")?;
        writeln!(self.anno_dump, "{}", raw_num_decls)?;
        let num_decls =
            parse_usize(&raw_num_decls).context("Parsing number of class declarations")?;
        (0..num_decls)
            .map(|i| {
                self.parse_class_declaration()
                    .with_context(|| format!("declaration {} of {}", i, num_decls))
            })
            .collect()
    }

    fn parse_class_declaration(&mut self) -> eyre::Result<ClassDeclaration> {
        let class_hash = self.current.next().context("Missing class hash")?;
        let compiled_class_hash = self.current.next().context("Missing compiled class hash")?;
        Ok(ClassDeclaration {
            class_hash,
            compiled_class_hash,
        })
    }

    fn check_zero_tail(&mut self) -> eyre::Result<usize> {
        let mut n = 0;
        while let Some(el) = self.current.next() {
            n += 1;
            if !el.is_zero() {
                return Err(anyhow!("Extra tail"));
            }
        }
        Ok(n)
    }
}
