//! Profiler delta-math correctness.
//!
//! Installs `ModAlloc` as the global allocator, runs a known
//! workload between `Profiler::start` and `Profiler::stop`, and
//! asserts the returned delta reflects that workload.

use mod_alloc::{ModAlloc, Profiler};

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

const K: usize = 200;
const SIZE: usize = 512;

#[test]
fn profiler_captures_delta() {
    // Warm up so the global handle is registered before we read it.
    let _warm: Vec<u8> = Vec::with_capacity(8);

    let p = Profiler::start();
    let before = GLOBAL.snapshot();

    let mut vecs: Vec<Vec<u8>> = Vec::with_capacity(K);
    for _ in 0..K {
        let mut v: Vec<u8> = Vec::with_capacity(SIZE);
        v.push(1);
        vecs.push(v);
    }

    let after_workload = GLOBAL.snapshot();
    let direct_delta_count = after_workload.alloc_count - before.alloc_count;

    drop(vecs);

    let delta = p.stop();

    assert!(
        delta.alloc_count >= K as u64,
        "Profiler delta alloc_count ({}) should capture at least {K} workload allocs",
        delta.alloc_count
    );

    assert!(
        delta.alloc_count >= direct_delta_count,
        "Profiler delta ({}) should be at least the direct-snapshot delta ({})",
        delta.alloc_count,
        direct_delta_count
    );

    assert!(
        delta.total_bytes >= (K * SIZE) as u64,
        "Profiler delta total_bytes ({}) should cover the {} byte workload",
        delta.total_bytes,
        K * SIZE
    );

    // peak_bytes is absolute, not a delta. It must be at least as
    // large as any current_bytes value we observed during the
    // window.
    assert!(
        delta.peak_bytes >= delta.current_bytes,
        "peak_bytes ({}) must be >= current_bytes delta ({})",
        delta.peak_bytes,
        delta.current_bytes
    );
}
