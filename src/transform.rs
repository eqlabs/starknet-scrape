use num_bigint::{BigUint, ToBigUint};

use std::str::FromStr;

use crate::blob_util::FIELD_ELEMENTS_PER_BLOB;

#[derive(Clone)]
struct TransConst {
    pub bls_modulus: BigUint,
    pub generator: BigUint,
    pub two: BigUint,
}

impl Default for TransConst {
    fn default() -> Self {
        let bls_modulus =
            "52435875175126190479447740508185965837690552500527637822603658699938581184513";
        let generator =
            "39033254847818212395286706435128746857159659164139250548781411570340225835782";
        Self {
            bls_modulus: BigUint::from_str(bls_modulus).unwrap(),
            generator: BigUint::from_str(generator).unwrap(),
            two: 2u32.to_biguint().unwrap(),
        }
    }
}

#[derive(Clone)]
pub struct Transformer {
    big_const: TransConst,
    points: Vec<BigUint>,
}

impl Transformer {
    pub fn new() -> Self {
        let big_const = Default::default();
        let points = (0..FIELD_ELEMENTS_PER_BLOB)
            .map(|i| {
                let s = i as u16;
                let r = s.reverse_bits();
                Self::gen_exp_mod(&big_const, r / 16)
            })
            .collect();
        Self { big_const, points }
    }

    fn gen_exp_mod(big_const: &TransConst, exponent: u16) -> BigUint {
        let exp = exponent.to_biguint().unwrap();
        big_const.generator.modpow(&exp, &big_const.bls_modulus)
    }

    pub fn transform(&self, arr: &Vec<BigUint>) -> Vec<BigUint> {
        self.ifft(arr, &self.points)
    }

    fn ifft(&self, arr: &Vec<BigUint>, xs: &Vec<BigUint>) -> Vec<BigUint> {
        // Base case: return immediately if the array length is 1
        if arr.len() == 1 {
            return arr.clone();
        }

        let n = arr.len() / 2;
        let mut res0 = Vec::with_capacity(n);
        let mut res1 = Vec::with_capacity(n);
        let mut new_xs = Vec::with_capacity(n);

        for i in (0..2 * n).step_by(2) {
            let a = &arr[i];
            let b = &arr[i + 1];
            let x = &xs[i];

            res0.push(self.div_mod((a + b).into(), self.big_const.two.clone()));
            // Handle subtraction to avoid underflow
            let diff = if b > a {
                self.big_const.bls_modulus.clone() - (b - a)
            } else {
                a - b
            };
            res1.push(self.div_mod(diff, self.big_const.two.clone() * x));

            let sq: BigUint = (x * x).into();
            new_xs.push(sq % self.big_const.bls_modulus.clone());
        }

        // Recursive calls
        let merged_res0 = self.ifft(&res0, &new_xs);
        let merged_res1 = self.ifft(&res1, &new_xs);

        // Merging the results
        let mut merged = Vec::with_capacity(arr.len());
        // FIXME
        for i in 0..n {
            merged.push(merged_res0[i].clone());
            merged.push(merged_res1[i].clone());
        }
        merged
    }

    fn div_mod(&self, a: BigUint, b: BigUint) -> BigUint {
        let e = self.big_const.bls_modulus.clone() - self.big_const.two.clone();
        let pow = b.modpow(&e, &self.big_const.bls_modulus);
        a * pow % self.big_const.bls_modulus.clone()
    }
}
