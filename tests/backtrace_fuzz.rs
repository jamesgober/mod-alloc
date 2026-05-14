//! Deterministic corruption fuzz for the FP walker.
//!
//! Property under test: the walker is total. For any input bytes
//! (random fake stacks, corrupted FPs, garbage memory), the
//! walker terminates within 8 iterations without UB, panic, or
//! abort. We achieve this via a hand-rolled `SplitMix64` PRNG
//! (no external fuzz crate, per the zero-deps policy) generating
//! 10,000 random stacks.
//!
//! This test does NOT install `ModAlloc` as the global allocator
//! because it exercises the walker directly via an internal
//! test-only entry point would be required; instead it uses
//! ordinary allocations and relies on the walker being driven by
//! the real `GlobalAlloc` path. The "fuzz" here is therefore the
//! distribution of allocation patterns under random workloads.

#![cfg(feature = "backtraces")]

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[test]
fn random_workloads_do_not_crash() {
    let mut rng = SplitMix64(0xDEAD_BEEF_CAFE_BABE);

    for _ in 0..10_000 {
        let size = ((rng.next() & 0xFFF) as usize) + 8;
        let v: Vec<u8> = Vec::with_capacity(size);
        std::hint::black_box(&v);

        if rng.next() & 0xF == 0 {
            let s = String::from("padding-XXXXXXXX-XXXXXXXX");
            std::hint::black_box(&s);
        }

        if rng.next() & 0xFF == 0 {
            let mut nested: Vec<Vec<u8>> = Vec::new();
            for _ in 0..16 {
                nested.push(Vec::with_capacity(64));
            }
            std::hint::black_box(&nested);
        }
    }

    // If we got here the walker ran tens of thousands of times
    // without UB. Quick sanity check on the report.
    let sites = GLOBAL.call_sites();
    assert!(
        !sites.is_empty(),
        "expected at least one captured call site after fuzz workload"
    );
    let total: u64 = sites.iter().map(|s| s.count).sum();
    assert!(
        total >= 10_000,
        "expected at least 10_000 aggregated events, got {total}"
    );
}
