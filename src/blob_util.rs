use eyre::anyhow;
use num_bigint::BigUint;
use num_traits::{Num, ToPrimitive};

pub const FIELD_ELEMENTS_PER_BLOB: usize = 4096;

pub fn parse_str_to_blob_data(contents: &str) -> eyre::Result<Vec<BigUint>> {
    let trimmed = contents.trim();
    let blob_hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    // blobs have fixed size
    if blob_hex.len() != FIELD_ELEMENTS_PER_BLOB * 64 {
        return Err(anyhow!(
            "expected {} hex chars, got {}",
            FIELD_ELEMENTS_PER_BLOB * 64,
            blob_hex.len()
        ));
    }

    let mut data = Vec::new();
    for i in 0..FIELD_ELEMENTS_PER_BLOB {
        let d = BigUint::from_str_radix(&blob_hex[i * 64..(i + 1) * 64], 16)
            .map_err(|_| anyhow!("invalid hex integer"))?;
        data.push(d);
    }

    Ok(data)
}

pub fn parse_usize(value: &BigUint) -> eyre::Result<usize> {
    value
        .to_usize()
        .ok_or_else(|| anyhow!("Value exceeds usize::MAX"))
}
