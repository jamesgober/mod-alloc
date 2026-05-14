//! Multi-thread backtrace stress test.
//!
//! 32 worker threads each performing 1,000 allocations across
//! distinct call paths. Verifies the global aggregation table
//! handles concurrent claim races, that the bucket count and
//! aggregated event total are sane, and that the table does not
//! deadlock.

#![cfg(feature = "backtraces")]

use std::thread;
use std::time::{Duration, Instant};

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

const THREADS: usize = 32;
const ALLOCS_PER_THREAD: usize = 1_000;
const TIMEOUT: Duration = Duration::from_secs(60);

#[inline(never)]
fn path_a() {
    let _v: Vec<u64> = Vec::with_capacity(32);
}

#[inline(never)]
fn path_b() {
    let _v: Vec<u64> = Vec::with_capacity(64);
}

#[inline(never)]
fn path_c() {
    let _v: Vec<u64> = Vec::with_capacity(128);
}

#[inline(never)]
fn dispatch(i: usize) {
    match i % 3 {
        0 => path_a(),
        1 => path_b(),
        _ => path_c(),
    }
}

#[test]
fn concurrent_backtrace_capture_terminates_and_aggregates() {
    let started = Instant::now();

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            thread::spawn(move || {
                for i in 0..ALLOCS_PER_THREAD {
                    dispatch(t * ALLOCS_PER_THREAD + i);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let elapsed = started.elapsed();
    assert!(elapsed < TIMEOUT, "stress test took too long: {elapsed:?}");

    let sites = GLOBAL.call_sites();
    assert!(!sites.is_empty(), "expected at least one site");

    let total: u64 = sites.iter().map(|s| s.count).sum();
    let expected = (THREADS * ALLOCS_PER_THREAD) as u64;
    assert!(
        total >= expected,
        "expected at least {expected} events captured, got {total}"
    );
}
