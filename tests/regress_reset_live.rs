//! Regression: `reset()` with live allocations must not make the
//! counters wrap.
//!
//! In 1.0.1 `reset()` zeroed `current_bytes` and `live_count`; freeing
//! any older block then wrapped them, the next allocation overflowed
//! inside `GlobalAlloc::alloc` (a panic in debug builds) and release
//! builds reported a peak near `u64::MAX`.

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

// Far above anything this test allocates.
const SANE: u64 = 1 << 40;

#[test]
fn reset_with_live_allocations_keeps_counters_sane() {
    let older: Vec<u8> = vec![7; 4096];
    std::hint::black_box(&older);
    GLOBAL.reset();

    let after_reset = GLOBAL.snapshot();
    assert!(after_reset.alloc_count < 16, "{after_reset:?}");
    assert!(after_reset.current_bytes >= 4096, "{after_reset:?}");
    assert!(after_reset.peak_bytes >= after_reset.current_bytes);

    drop(older);
    let newer: Vec<u8> = vec![1; 1024];
    std::hint::black_box(&newer);
    let s = GLOBAL.snapshot();
    assert!(s.current_bytes < SANE, "current_bytes wrapped: {s:?}");
    assert!(s.live_count < SANE, "live_count wrapped: {s:?}");
    assert!(s.peak_bytes < SANE, "peak_bytes wrapped: {s:?}");
    assert!(s.peak_live_count < SANE, "peak_live_count wrapped: {s:?}");
    assert!(s.alloc_count >= 1);
    assert!(s.peak_bytes >= s.current_bytes);
    drop(newer);
}
