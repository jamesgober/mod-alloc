//! Regression: `dhat_compat::HeapStats::get` counts from the start of
//! the profiling session, as dhat-rs does.
//!
//! In 1.0.1 it counted from process start, so the usual dhat-rs test
//! pattern failed after the import change.

#![cfg(feature = "dhat-compat")]

use mod_alloc::dhat_compat::{Alloc, HeapStats, Profiler};

#[global_allocator]
static ALLOC: Alloc = Alloc;

#[test]
fn heap_stats_cover_only_the_profiling_session() {
    // Allocation history from before the session.
    let earlier: Vec<u8> = vec![0; 100 * 1024];
    std::hint::black_box(&earlier);
    drop(earlier);

    let _p = Profiler::builder().testing().build();
    let b: Box<[u8; 10]> = Box::new([1; 10]);
    std::hint::black_box(&b);
    let s = HeapStats::get();
    // The exact figures are 1 block and 10 bytes; the ranges allow
    // for an allocation made by the test harness on another thread.
    assert!((1..10).contains(&s.total_blocks), "{s:?}");
    assert!((10..1000).contains(&s.total_bytes), "{s:?}");
    assert!(s.max_bytes < 50 * 1024, "{s:?}");
    assert!(s.max_bytes >= s.curr_bytes, "{s:?}");
    drop(b);
}
