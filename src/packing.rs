use eyre::anyhow;
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};

use std::ops::{BitAnd, Shr};

pub struct PackConst {
    pub top_mask: BigUint,
    pub short_top_mask: BigUint,
    pub class_flag_mask: BigUint,
    pub nonce_mask: BigUint,
    pub short_nonce_mask: BigUint,
    pub nonce_shift: usize,
    pub short_nonce_shift: usize,
    pub update_count_mask: BigUint,
    pub update_count_shift: usize,
    pub short_update_count_mask: BigUint,
    pub short_flag_mask: BigUint,
    pub one: BigUint,
    pub two: BigUint,
}

impl PackConst {
    pub fn unpack_contract_update(&self, packed: &BigUint) -> eyre::Result<(bool, u64, u64)> {
        let short_flag = !self.short_flag_mask.clone().bitand(packed).is_zero();
        let top_mask = if short_flag {
            self.short_top_mask.clone()
        } else {
            self.top_mask.clone()
        };
        let top = top_mask.bitand(packed);
        if !top.is_zero() {
            return Err(anyhow!("Extra high bits"));
        }

        let class_flag = !self.class_flag_mask.clone().bitand(packed).is_zero();
        let nonce_mask = if short_flag {
            self.short_nonce_mask.clone()
        } else {
            self.nonce_mask.clone()
        };
        let nonce_high = nonce_mask.bitand(packed);
        let nonce_shift = if short_flag {
            self.short_nonce_shift
        } else {
            self.nonce_shift
        };
        let nonce = nonce_high.shr(nonce_shift);
        let update_count_mask = if short_flag {
            self.short_update_count_mask.clone()
        } else {
            self.update_count_mask.clone()
        };
        let update_count_high = update_count_mask.bitand(packed);
        let update_count = update_count_high.shr(self.update_count_shift);
        Ok((
            class_flag,
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
        let two: BigUint = 2u32.to_biguint().unwrap();
        let top_mask_low: BigUint = one.clone().shl(127) - one.clone();
        let class_flag_mask = one.clone().shl(128);
        let nonce_mask_low: BigUint = one.clone().shl(64) - one.clone();
        let update_count_mask = nonce_mask_low.clone();
        PackConst {
            top_mask: top_mask_low.shl(129),
            short_top_mask: BigUint::ZERO, // not used
            class_flag_mask,
            nonce_mask: nonce_mask_low.shl(64),
            short_nonce_mask: BigUint::ZERO, // not used
            nonce_shift: 64,
            short_nonce_shift: 0, // not used
            update_count_mask,
            update_count_shift: 0,
            short_update_count_mask: BigUint::ZERO, // not used
            short_flag_mask: BigUint::ZERO,         // never matches
            one,
            two,
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
                pack_const.unpack_contract_update(&u).unwrap();
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
        let two: BigUint = 2u32.to_biguint().unwrap();
        let top_mask_low: BigUint = one.clone().shl(126) - one.clone();
        let short_top_mask_low: BigUint = one.clone().shl(182) - one.clone();
        let nonce_mask_low: BigUint = one.clone().shl(64) - one.clone();
        let update_count_mask_low = nonce_mask_low.clone();
        let short_update_count_mask_low: BigUint = 255.to_biguint().unwrap();
        PackConst {
            top_mask: top_mask_low.shl(130),
            short_top_mask: short_top_mask_low.shl(74),
            nonce_mask: nonce_mask_low.clone().shl(66),
            short_nonce_mask: nonce_mask_low.shl(10),
            nonce_shift: 66,
            short_nonce_shift: 10,
            update_count_mask: update_count_mask_low.shl(2),
            short_update_count_mask: short_update_count_mask_low.shl(2),
            update_count_shift: 2,
            // https://docs.starknet.io/architecture-and-concepts/network-architecture/data-availability/
            // says 2 but that's already incorrect for the 0x1
            // contract of the first compressed blob (eth block
            // 21282183), which has the second-lowest bit set but no
            // class hash (while the lowest bit is clear & number of
            // updates < 256)
            class_flag_mask: one.clone(),
            short_flag_mask: two.clone(),
            one,
            two,
        }
    }

    mod tests {
        #[rstest::rstest]
        #[case::first(46, (false, 0, 11))]
        #[case::zero_storage_updates(3074, (false, 3, 0))]
        #[case::long(0x5b8, (false, 0, 366))]
        fn wild_example(#[case] input: u64, #[case] expected: (bool, u64, u64)) {
            use num_bigint::ToBigUint;

            use super::make_pack_const;

            let pack_const = make_pack_const();
            let u = input.to_biguint().unwrap();
            let (class_flag, nonce, update_count) = pack_const.unpack_contract_update(&u).unwrap();
            assert_eq!(expected.0, class_flag);
            assert_eq!(expected.1, nonce);
            assert_eq!(expected.2, update_count);
        }
    }
}
