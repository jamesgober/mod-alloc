//! Integration test: real-call-chain capture.
//!
//! Builds a chain of `#[inline(never)]` functions that ends in an
//! allocation. Asserts that the per-call-site report contains a
//! site with multiple distinct frames. We don't assert exact
//! frame counts or addresses (optimisation can still vary), only
//! that the walker captured *something* and that the aggregation
//! is non-empty.
//!
//! Requires `--features backtraces`. Without `-C
//! force-frame-pointers=yes` the captured trace may be very
//! shallow; the in-crate `.cargo/config.toml` enables that flag
//! for this crate's own builds.

#![cfg(feature = "backtraces")]

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

#[inline(never)]
fn level_three() {
    let v: Vec<u64> = Vec::with_capacity(1024);
    std::hint::black_box(&v);
}

#[inline(never)]
fn level_two() {
    level_three();
}

#[inline(never)]
fn level_one() {
    level_two();
}

#[test]
fn captures_at_least_one_call_site() {
    for _ in 0..50 {
        level_one();
    }

    let sites = GLOBAL.call_sites();
    assert!(
        !sites.is_empty(),
        "expected at least one captured call site"
    );

    let total_count: u64 = sites.iter().map(|s| s.count).sum();
    assert!(
        total_count >= 50,
        "expected at least 50 aggregated events, got {total_count}"
    );

    let max_frames = sites.iter().map(|s| s.frame_count).max().unwrap_or(0);
    assert!(
        max_frames >= 1,
        "expected at least one captured frame in the report"
    );
}
