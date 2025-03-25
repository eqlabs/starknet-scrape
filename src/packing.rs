use eyre::anyhow;
use num_bigint::{BigUint, ToBigUint};
use num_traits::{ToPrimitive, Zero};

use std::ops::{BitAnd, Shl, Shr};

pub struct PackConst {
    pub top_mask: BigUint,
    pub class_flag_mask: BigUint,
    pub nonce_mask: BigUint,
    pub nonce_shift: usize,
    pub update_count_mask: BigUint,
}

impl Default for PackConst {
    fn default() -> Self {
        let one: BigUint = 1u32.to_biguint().unwrap();
        let top_mask_low: BigUint = one.clone().shl(127) - one.clone();
        let class_flag_mask = one.clone().shl(128);
        let nonce_mask_low: BigUint = one.clone().shl(64) - one.clone();
        let update_count_mask = nonce_mask_low.clone();
        Self {
            top_mask: top_mask_low.shl(129),
            class_flag_mask,
            nonce_mask: nonce_mask_low.shl(64),
            nonce_shift: 64,
            update_count_mask,
        }
    }
}

impl PackConst {
    pub fn unpack_contract_update(&self, packed: BigUint) -> eyre::Result<(bool, u64, u64)> {
        let top = self.top_mask.clone().bitand(packed.clone());
        if !top.is_zero() {
            return Err(anyhow!("Extra high bits"));
        }

        let class_flag_bit = !self
            .class_flag_mask
            .clone()
            .bitand(packed.clone())
            .is_zero();
        let nonce_high = self.nonce_mask.clone().bitand(packed.clone());
        let nonce = nonce_high.shr(self.nonce_shift);
        let update_count = self.update_count_mask.clone().bitand(packed);
        Ok((
            class_flag_bit,
            nonce.to_u64().expect("bitmasked"),
            update_count.to_u64().expect("bitmasked"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::PackConst;
    use num_bigint::BigUint;
    use std::str::FromStr;

    #[test]
    fn spec_example() {
        let pack_const: PackConst = Default::default();
        let u = BigUint::from_str("18446744073709551617").unwrap();
        let (class_flag_bit, nonce, update_count) = pack_const.unpack_contract_update(u).unwrap();
        assert!(!class_flag_bit);
        assert_eq!(nonce, 1);
        assert_eq!(update_count, 1);
    }
}
