// ported from compression.cairo

use eyre::{ContextCompat, anyhow};
use num_bigint::{BigUint, ToBigUint};
use num_traits::{ToPrimitive, Zero};

use std::ops::Shl;

use crate::blob_util::parse_usize;

const HEADER_ELM_N_BITS: usize = 20;

const MAX_N_BITS_PER_FELT: usize = 251;

const TOTAL_N_BUCKETS: usize = 7;

fn unpack_felt(
    packed: BigUint,
    elm_bound: &BigUint,
    n_elms: usize,
) -> eyre::Result<(Vec<BigUint>, BigUint)> {
    let mut packed = packed;
    let mut out = Vec::new();
    // not clear if needed, but compression.cairo has it...
    if n_elms == 0 {
        let res = if packed.is_zero() {
            Ok((out, BigUint::ZERO))
        } else {
            Err(anyhow!("unpacking leaves set bits"))
        };
        return res;
    }

    let mut i = 0;
    while i < n_elms {
        let elm = packed.clone() % elm_bound.clone();
        out.push(elm);
        packed /= elm_bound.clone();
        i += 1;
    }

    Ok((out, packed))
}

fn unpack_header(packed: BigUint) -> eyre::Result<Vec<usize>> {
    let elm_bound = 1u32.to_biguint().unwrap().shl(HEADER_ELM_N_BITS);
    let (elements, rest) = unpack_felt(packed, &elm_bound, 9)?;
    if !rest.is_zero() {
        return Err(anyhow!("high header bits set"));
    }

    let mut iter = elements.into_iter();
    let version = iter.next().expect("9 elements");
    if !version.is_zero() {
        return Err(anyhow!("invalid compression version"));
    }

    let sizes = iter
        .map(|el| parse_usize(el).expect("HEADER_ELM_N_BITS must fit usize"))
        .collect();
    Ok(sizes)
}

fn make_bucket_bounds() -> Vec<BigUint> {
    let one: BigUint = 1u32.to_biguint().unwrap();
    let powers = [252, 125, 83, 62, 31, 15];
    powers.into_iter().map(|p| one.clone().shl(p)).collect()
}

fn get_n_elms_per_felt(elm_bound: usize) -> usize {
    if elm_bound < 2 {
        return MAX_N_BITS_PER_FELT;
    }

    let prev = elm_bound - 1;
    let n_bits_per_elm = usize::BITS - prev.leading_zeros(); // log2_ceil(elm_bound);
    MAX_N_BITS_PER_FELT / (n_bits_per_elm as usize)
}

fn extend_with_repeats(src_and_dst: &mut Vec<BigUint>, indices: &Vec<BigUint>) {
    for big_idx in indices.iter() {
        // caller ensures indices are actually indices, i.e. small enough
        let idx = big_idx.to_usize().unwrap();
        src_and_dst.push(src_and_dst[idx].clone());
    }
}

// from sequencer
fn get_bucket_offsets(bucket_lengths: &[usize]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(bucket_lengths.len());
    let mut current = 0;
    for &length in bucket_lengths {
        offsets.push(current);
        current += length;
    }

    offsets
}

pub struct Decompressor<I> {
    pub current: I,
    pub sizes: Vec<usize>, // 8 items (header w/o version)
}

impl<I> Decompressor<I>
where
    I: Iterator<Item = BigUint>,
{
    pub fn decompress(iter: I) -> eyre::Result<(Vec<BigUint>, usize)> {
        let mut iter = iter;
        let header = iter.next().context("nothing to decompress")?;
        let sizes = unpack_header(header)?;
        let mut decompressor = Self {
            current: iter,
            sizes,
        };

        let mut all_values = Vec::new();
        decompressor.unpack_unique_values(&mut all_values)?;
        let n_unique_values = all_values.len();
        let uncompressed_len = decompressor.sizes[0];
        let n_repeating_values = decompressor.sizes[7];
        if uncompressed_len != (n_unique_values + n_repeating_values) {
            return Err(anyhow!(
                "uncompressed length {}, {} unique values, {} repeating values",
                uncompressed_len,
                n_unique_values,
                n_repeating_values,
            ));
        }

        decompressor.unpack_repeating_values(&mut all_values)?;
        let bucket_index_per_elm = decompressor.unpack_bucket_index_per_elm()?;
        let data = decompressor.reconstruct_data(&all_values, &bucket_index_per_elm);
        let n = decompressor.check_zero_tail()?;
        Ok((data, n))
    }

    fn unpack_unique_values(&mut self, decompressed_dst: &mut Vec<BigUint>) -> eyre::Result<()> {
        let bucket_bounds = make_bucket_bounds();

        // not including the largest bucket (which doesn't do any
        // packing)
        let pack_counts: [usize; 5] = [2, 3, 4, 8, 16];

        self.copy_largest_bucket(decompressed_dst)?;
        for i in 0..5 {
            self.unpack_felts(
                self.sizes[i + 2],
                &bucket_bounds[i + 1],
                pack_counts[i],
                decompressed_dst,
            )?;
        }

        Ok(())
    }

    fn unpack_repeating_values(&mut self, src_and_dst: &mut Vec<BigUint>) -> eyre::Result<()> {
        let pointers = self.unpack_repeating_value_pointers(src_and_dst.len())?;
        extend_with_repeats(src_and_dst, &pointers);
        Ok(())
    }

    fn unpack_repeating_value_pointers(&mut self, n_unique_values: usize) -> eyre::Result<Vec<BigUint>> {
        let n_repeating_values = self.sizes[7];
        let pointer_bound = n_unique_values.to_biguint().unwrap();
        let n_elms_per_felt = get_n_elms_per_felt(n_unique_values);
        let mut pointers = Vec::new();
        self.unpack_felts(n_repeating_values, &pointer_bound, n_elms_per_felt, &mut pointers)?;
        Ok(pointers)
    }

    fn copy_largest_bucket(&mut self, decompressed_dst: &mut Vec<BigUint>) -> eyre::Result<()> {
        let n_elms = self.sizes[1];
        for i in 0..n_elms {
            let el = self
                .current
                .next()
                .with_context(|| format!("large element {} of {} not found", i, n_elms))?;
            decompressed_dst.push(el);
        }

        Ok(())
    }

    fn unpack_bucket_index_per_elm(&mut self) -> eyre::Result<Vec<BigUint>> {
        let total_n_buckets = TOTAL_N_BUCKETS.to_biguint().unwrap();
        let n_elms_per_felt = 83; // get_n_elms_per_felt(TOTAL_N_BUCKETS);
        let mut bucket_index_per_elm = Vec::new();
        self.unpack_felts(self.sizes[0], &total_n_buckets, n_elms_per_felt, &mut bucket_index_per_elm)?;
        Ok(bucket_index_per_elm)
    }

    fn reconstruct_data(&mut self, all_values: &Vec<BigUint>, bucket_index_per_elm: &Vec<BigUint>) -> Vec<BigUint> {
        // input includes repeated values count but that's just a
        // placeholder - the offset after the last segment (AKA the
        // total count) is neither needed nor included in
        // offset_trackers
        let mut offset_trackers = get_bucket_offsets(&self.sizes[1..=7]);
        let mut data = Vec::new();
        for bucket_index in bucket_index_per_elm.iter() {
            // unpack_bucket_index_per_elm ensures indices are
            // actually indices, i.e. small enough
            let idx = bucket_index.to_usize().unwrap();
            let offset = &mut offset_trackers[idx];
            let val = &all_values[*offset];
            *offset += 1;
            data.push(val.clone());
        }

        data
    }

    fn unpack_felts(
        &mut self,
        n_elms: usize,
        elm_bound: &BigUint,
        n_elms_per_felt: usize,
        decompressed_dst: &mut Vec<BigUint>,
    ) -> eyre::Result<()> {
        let n_full_felts = n_elms / n_elms_per_felt;
        let n_remaining_elms = n_elms % n_elms_per_felt;
        for _ in 0..n_full_felts {
            self.unpack_felts_given_n_packed_felts(elm_bound, n_elms_per_felt, decompressed_dst)?;
        }

        if n_remaining_elms > 0 {
            self.unpack_felts_given_n_packed_felts(elm_bound, n_remaining_elms, decompressed_dst)?;
        }

        Ok(())
    }

    fn unpack_felts_given_n_packed_felts(
        &mut self,
        elm_bound: &BigUint,
        n_elms_per_felt: usize,
        decompressed_dst: &mut Vec<BigUint>,
    ) -> eyre::Result<()> {
        let felt = self
            .current
            .next()
            .context("iterator finished before going through sizes")?;
        let (mut elements, rest) = unpack_felt(felt, elm_bound, n_elms_per_felt)?;
        if !rest.is_zero() {
            return Err(anyhow!("high bits set"));
        }

        decompressed_dst.append(&mut elements);
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::get_n_elms_per_felt;

    #[test]
    fn test_dyn_packing_count() {
        assert_eq!(get_n_elms_per_felt(2_usize.pow(15) - 1), 16);
        assert_eq!(get_n_elms_per_felt(7), 83);
    }
}
