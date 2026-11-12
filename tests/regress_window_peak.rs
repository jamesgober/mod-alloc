//! Regression: `Profiler::stop().peak_bytes` is the peak of the
//! profiling window, not of the whole process.
//!
//! In 1.0.1 a window that allocated 1 KiB after a 64 MiB block had
//! been freed reported a 64 MiB peak.

use mod_alloc::{ModAlloc, Profiler};

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

const BIG: usize = 64 << 20;

#[test]
fn window_peak_ignores_earlier_high_water_mark() {
    let big: Vec<u8> = vec![0; BIG];
    std::hint::black_box(&big);
    drop(big);
    assert!(GLOBAL.snapshot().peak_bytes >= BIG as u64);

    let p = Profiler::start();
    let small: Vec<u8> = vec![0; 1024];
    std::hint::black_box(&small);
    drop(small);
    let s = p.stop();
    assert!(
        s.peak_bytes < (BIG as u64) / 2,
        "window peak should not include the earlier 64 MiB block: {s:?}"
    );

    // Nested windows each keep their own peak.
    let outer = Profiler::start();
    let mid: Vec<u8> = vec![0; 8 << 20];
    std::hint::black_box(&mid);
    drop(mid);
    let inner = Profiler::start();
    let tiny: Vec<u8> = vec![0; 16];
    std::hint::black_box(&tiny);
    drop(tiny);
    let si = inner.stop();
    let so = outer.stop();
    assert!(
        so.peak_bytes >= 8 << 20,
        "outer window lost its peak: {so:?}"
    );
    assert!(
        si.peak_bytes < 4 << 20,
        "inner window saw the outer peak: {si:?}"
    );
}
