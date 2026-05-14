//! Single-threaded counter accuracy.
//!
//! Installs `ModAlloc` as the global allocator for this test
//! binary, performs a known number of allocations, and asserts that
//! the recorded counters move in the expected direction.
//!
//! Each integration test file is compiled as its own binary, which
//! lets each file own its `#[global_allocator]` without conflicting
//! with other tests.

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

const N: usize = 200;
const SIZE: usize = 1024;

#[test]
fn counters_track_allocations_accurately() {
    let before = GLOBAL.snapshot();

    let mut vecs: Vec<Vec<u8>> = Vec::with_capacity(N);
    for _ in 0..N {
        let mut v: Vec<u8> = Vec::with_capacity(SIZE);
        v.push(0);
        vecs.push(v);
    }

    let after_alloc = GLOBAL.snapshot();
    let alloc_delta = after_alloc.alloc_count - before.alloc_count;
    let bytes_delta = after_alloc.total_bytes - before.total_bytes;

    assert!(
        alloc_delta >= N as u64,
        "expected at least {N} allocations, observed delta = {alloc_delta}"
    );
    assert!(
        bytes_delta >= (N * SIZE) as u64,
        "expected at least {} bytes allocated, observed delta = {bytes_delta}",
        N * SIZE
    );

    let current_before_drop = after_alloc.current_bytes;
    let peak_before_drop = after_alloc.peak_bytes;
    assert!(peak_before_drop >= current_before_drop);

    drop(vecs);

    let after_drop = GLOBAL.snapshot();

    assert!(
        after_drop.current_bytes < current_before_drop,
        "current_bytes should decrease after drop ({} -> {})",
        current_before_drop,
        after_drop.current_bytes
    );

    assert_eq!(
        after_drop.peak_bytes, peak_before_drop,
        "peak_bytes must not decrease after deallocation"
    );

    assert_eq!(
        after_drop.alloc_count, after_alloc.alloc_count,
        "alloc_count must not change on dealloc"
    );
}
