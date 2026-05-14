//! Concurrent symbolication safety test.
//!
//! Spawns several threads that simultaneously call
//! `symbolicated_report()` and asserts they all complete without
//! deadlock and produce non-empty reports. Validates that the
//! per-process address cache and the platform symbolicator are
//! safe under concurrent reader load.

#![cfg(feature = "symbolicate")]

use std::thread;
use std::time::{Duration, Instant};

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

#[inline(never)]
fn workload() {
    for _ in 0..200 {
        let _v: Vec<u8> = Vec::with_capacity(128);
    }
}

#[test]
fn concurrent_reports_terminate() {
    // Run the workload on the main thread, then call
    // `symbolicated_report()` once from main to flush main's
    // arena into the global table. Without this primer, the
    // worker threads (which each have their own empty TLS arena)
    // would see an empty global table because main's events were
    // stuck in main's arena below the 512-event flush threshold.
    workload();
    let primer = GLOBAL.symbolicated_report();
    assert!(!primer.is_empty(), "primer report should observe workload");

    let started = Instant::now();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            thread::spawn(|| {
                let report = GLOBAL.symbolicated_report();
                assert!(!report.is_empty());
                report.len()
            })
        })
        .collect();

    let mut sizes = Vec::new();
    for h in handles {
        sizes.push(h.join().expect("symbolicator panicked under contention"));
    }

    assert!(
        started.elapsed() < Duration::from_secs(30),
        "symbolicator deadlocked or stalled under concurrent access"
    );

    // Each worker thread's `symbolicated_report()` call first
    // flushes the calling thread's own arena into the global
    // aggregation table, so row counts grow monotonically as
    // workers race through their reports. We do not assert
    // exact agreement (that would race with the flushes); we
    // only assert that every report saw at least the primer
    // size and that no report shrank, both of which would
    // indicate concurrency bugs.
    let primer_size = primer.len();
    let max_size = *sizes.iter().max().unwrap();
    for &n in &sizes {
        assert!(
            n >= primer_size,
            "concurrent report ({n}) below primer ({primer_size}); table shrank?"
        );
        assert!(
            n <= max_size,
            "concurrent report ({n}) above max ({max_size}); impossible"
        );
    }
}
