use eyre::anyhow;
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};

use std::ops::{BitAnd, Shr};

pub struct PackConst {
    pub top_mask: BigUint,
    pub class_flag_mask: BigUint,
    pub nonce_mask: BigUint,
    pub nonce_shift: usize,
    pub update_count_mask: BigUint,
    pub update_count_shift: usize,
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
        let update_count_high = self.update_count_mask.clone().bitand(packed);
        let update_count = update_count_high.shr(self.update_count_shift);
        Ok((
            class_flag_bit,
            nonce.to_u64().expect("bitmasked"),
            update_count.to_u64().expect("bitmasked"),
        ))
    }
}

pub mod v0_13_1 {
    use num_bigint::{BigUint, ToBigUint};

    use std::ops::Shl;

    use super::PackConst;

    pub fn make_pack_const() -> PackConst {
        let one: BigUint = 1u32.to_biguint().unwrap();
        let top_mask_low: BigUint = one.clone().shl(127) - one.clone();
        let class_flag_mask = one.clone().shl(128);
        let nonce_mask_low: BigUint = one.clone().shl(64) - one.clone();
        let update_count_mask = nonce_mask_low.clone();
        PackConst {
            top_mask: top_mask_low.shl(129),
            class_flag_mask,
            nonce_mask: nonce_mask_low.shl(64),
            nonce_shift: 64,
            update_count_mask,
            update_count_shift: 0,
        }
    }

    #[cfg(test)]
    mod tests {
        use num_bigint::BigUint;
        use std::str::FromStr;

        use super::make_pack_const;

        #[test]
        fn spec_example() {
            let pack_const = make_pack_const();
            let u = BigUint::from_str("18446744073709551617").unwrap();
            let (class_flag_bit, nonce, update_count) =
                pack_const.unpack_contract_update(u).unwrap();
            assert!(!class_flag_bit);
            assert_eq!(nonce, 1);
            assert_eq!(update_count, 1);
        }
    }
}

pub mod v0_13_3 {
    use num_bigint::{BigUint, ToBigUint};

    use std::ops::Shl;

    use super::PackConst;

    pub fn make_pack_const() -> PackConst {
        let one: BigUint = 1u32.to_biguint().unwrap();
        let top_mask_low: BigUint = one.clone().shl(126) - one.clone();
        let nonce_mask_low: BigUint = one.clone().shl(64) - one.clone();
        let update_count_mask_low = nonce_mask_low.clone();
        PackConst {
            top_mask: top_mask_low.shl(130),
            nonce_mask: nonce_mask_low.shl(66),
            nonce_shift: 66,
            update_count_mask: update_count_mask_low.shl(2),
            update_count_shift: 2,
            // https://docs.starknet.io/architecture-and-concepts/network-architecture/data-availability/
            // says 2 but that's already incorrect for the 0x1
            // contract of the first compressed blob (eth block
            // 21282183), which has the second-lowest bit set but no
            // class hash (while the lowest bit is clear & number of
            // updates < 256)
            class_flag_mask: one,
        }
    }
}
