//! Regression: a `ModAlloc` used directly must never become the
//! instance that `Profiler` reads.
//!
//! In 1.0.1 the first `ModAlloc` to serve an allocation published its
//! own address, so a short-lived local made `Profiler::start` read a
//! dead stack frame (undefined behaviour, reported by Miri). This test
//! binary has no `ModAlloc` global allocator, so the profiler must
//! report zeros.

use mod_alloc::{ModAlloc, Profiler};
use std::alloc::{GlobalAlloc, Layout};

#[inline(never)]
fn use_local_allocator() -> u64 {
    let a = ModAlloc::new();
    let layout = Layout::from_size_align(64, 8).unwrap();
    // SAFETY: `layout` has non-zero size; the block is freed with the
    // same layout on the same allocator.
    unsafe {
        let p = a.alloc(layout);
        assert!(!p.is_null());
        a.dealloc(p, layout);
    }
    a.snapshot().alloc_count
}

#[inline(never)]
fn scribble() -> u64 {
    let junk = [0xAAu8; 512];
    std::hint::black_box(&junk);
    junk.iter().map(|&b| b as u64).sum()
}

#[test]
fn profiler_ignores_dead_local_allocator() {
    // The local keeps its own counters.
    assert_eq!(use_local_allocator(), 1);
    let _ = scribble();
    let p = Profiler::start();
    let _ = scribble();
    let s = p.stop();
    assert_eq!(s.alloc_count, 0, "{s:?}");
    assert_eq!(s.total_bytes, 0, "{s:?}");
    assert_eq!(s.peak_bytes, 0, "{s:?}");
    assert_eq!(s.current_bytes, 0, "{s:?}");
}
