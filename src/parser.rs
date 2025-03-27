use eyre::{ContextCompat, WrapErr, anyhow};
use num_bigint::BigUint;
use num_traits::Zero;

use std::io::Write;

use crate::blob_util::parse_usize;
use crate::packing::PackConst;

#[derive(Debug)]
pub struct StorageUpdate {
    pub key: BigUint,
    pub value: BigUint,
}

#[derive(Debug)]
pub struct ContractUpdate {
    pub address: BigUint,
    pub nonce: u64,
    pub new_class_hash: Option<BigUint>, // Some only if class updated
    pub storage_updates: Vec<StorageUpdate>,
}

#[derive(Debug)]
pub struct ClassDeclaration {
    pub class_hash: BigUint,
    pub compiled_class_hash: BigUint,
}

#[derive(Debug)]
pub struct StateDiff {
    pub contract_updates: Vec<ContractUpdate>,
    pub class_declarations: Vec<ClassDeclaration>,
}

pub struct StateUpdateParser<I> {
    pub current: I,
    pub pack_const: PackConst,
    pub anno_dump: Box<dyn Write>,
}

impl<I> StateUpdateParser<I>
where
    I: Iterator<Item = BigUint>,
{
    pub fn parse(
        iter: I,
        unpacker: PackConst,
        anno_dump: Box<dyn Write>,
    ) -> eyre::Result<(StateDiff, usize)> {
        let mut parser = Self {
            current: iter,
            pack_const: unpacker,
            anno_dump,
        };
        let contract_updates = parser.parse_contract_updates()?;
        let class_declarations = parser.parse_class_declarations()?;
        let n = parser.check_zero_tail()?;
        Ok((
            StateDiff {
                contract_updates,
                class_declarations,
            },
            n,
        ))
    }

    fn parse_contract_updates(&mut self) -> eyre::Result<Vec<ContractUpdate>> {
        let raw_num_contracts: BigUint = self
            .current
            .next()
            .context("Missing number of contract updates")?;
        writeln!(self.anno_dump, "{}", raw_num_contracts)?;
        let num_contracts =
            parse_usize(raw_num_contracts).context("Parsing number of contract updates")?;
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

        if address == self.pack_const.two {
            // should be switching to stateful compression here...
            return Err(anyhow!("0x2 contract present"));
        }

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

        Ok(ContractUpdate {
            address,
            nonce,
            new_class_hash,
            storage_updates,
        })
    }

    fn parse_storage_update(&mut self) -> eyre::Result<StorageUpdate> {
        let key = self.current.next().context("Missing storage address")?;
        writeln!(self.anno_dump, "k: {:#x}", key)?;
        let value = self.current.next().context("Missing storage value")?;
        writeln!(self.anno_dump, "v: {:#x}", value)?;
        Ok(StorageUpdate { key, value })
    }

    fn parse_class_declarations(&mut self) -> eyre::Result<Vec<ClassDeclaration>> {
        let raw_num_decls: BigUint = self
            .current
            .next()
            .context("Missing number of class declarations")?;
        writeln!(self.anno_dump, "{}", raw_num_decls)?;
        let num_decls =
            parse_usize(raw_num_decls).context("Parsing number of class declarations")?;
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
