//! Inline FxHash variant for frame-array hashing.
//!
//! FxHash is the algorithm rustc uses internally. Fast, no
//! external dependencies, good enough for bucket-index calculation
//! on small fixed-size inputs. Not cryptographic.
//!
//! The hash reserves the value `0` for "empty bucket" semantics in
//! the aggregation table; the function maps any input that would
//! naturally hash to 0 onto 1 instead.

const K: u64 = 0x517c_c1b7_2722_0a95;

/// Hash a prefix of a frame array.
///
/// `count` is the number of valid frames in `frames`. The function
/// reads `frames[0..count]` and ignores the rest.
#[inline]
pub(crate) fn hash_frames(frames: &[u64; 8], count: usize) -> u64 {
    let n = if count > 8 { 8 } else { count };
    let mut h: u64 = 0;
    let mut i = 0;
    while i < n {
        h = (h.rotate_left(5) ^ frames[i]).wrapping_mul(K);
        i += 1;
    }
    if h == 0 {
        1
    } else {
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let frames = [0x1111, 0x2222, 0x3333, 0x4444, 0, 0, 0, 0];
        assert_eq!(hash_frames(&frames, 4), hash_frames(&frames, 4));
    }

    #[test]
    fn frame_count_affects_hash() {
        let frames = [0x1111, 0x2222, 0x3333, 0x4444, 0, 0, 0, 0];
        let h2 = hash_frames(&frames, 2);
        let h3 = hash_frames(&frames, 3);
        assert_ne!(h2, h3);
    }

    #[test]
    fn never_returns_zero() {
        let zero_frames = [0u64; 8];
        assert_eq!(hash_frames(&zero_frames, 0), 1);
        assert_eq!(hash_frames(&zero_frames, 8), 1);
    }

    #[test]
    fn distinct_inputs_distinct_outputs() {
        let mut seen = std::collections::HashSet::new();
        for i in 0u64..1000 {
            let frames = [i, i.wrapping_mul(7), i.wrapping_add(31), 0, 0, 0, 0, 0];
            let h = hash_frames(&frames, 3);
            seen.insert(h);
        }
        // Collisions can happen but should be rare across 1000 inputs.
        assert!(seen.len() > 990, "too many hash collisions: {}", seen.len());
    }

    #[test]
    fn count_clamps_to_eight() {
        let frames = [
            0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666, 0x7777, 0x8888,
        ];
        assert_eq!(hash_frames(&frames, 8), hash_frames(&frames, 99));
    }
}
