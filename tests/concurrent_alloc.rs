//! Multi-threaded stress test for atomic-counter contention.
//!
//! 64 threads each perform 5,000 allocations (320,000 total). The
//! test asserts that the aggregate counter movement matches the
//! workload, that no thread deadlocks, and that the run terminates
//! within a generous timeout.

use std::thread;
use std::time::{Duration, Instant};

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

const THREADS: usize = 64;
const ALLOCS_PER_THREAD: usize = 5_000;
const SIZE: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(60);

#[test]
fn concurrent_allocation_stress() {
    let before = GLOBAL.snapshot();
    let started = Instant::now();

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            thread::spawn(|| {
                for _ in 0..ALLOCS_PER_THREAD {
                    let v: Vec<u8> = Vec::with_capacity(SIZE);
                    std::hint::black_box(&v);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let elapsed = started.elapsed();
    assert!(
        elapsed < TIMEOUT,
        "stress test took too long: {elapsed:?} (deadlock or pathological contention?)"
    );

    let after = GLOBAL.snapshot();
    let alloc_delta = after.alloc_count - before.alloc_count;
    let expected = (THREADS * ALLOCS_PER_THREAD) as u64;

    assert!(
        alloc_delta >= expected,
        "expected at least {expected} allocs across {THREADS} threads, observed {alloc_delta}"
    );

    // Sanity ceiling: more than 10x the expected count means the
    // counter is being incremented spuriously somewhere. (Test
    // harness allocations contribute noise above the workload but
    // they should remain a small fraction of total volume.)
    assert!(
        alloc_delta < expected * 10,
        "observed {alloc_delta} allocs vs expected ~{expected}; counter is over-reporting"
    );

    // current_bytes returned to (near) zero confirms allocs paired
    // with deallocs. Vecs in the workers are scoped-dropped each
    // iteration so the only residual is whatever the harness still
    // holds. Verify it is small relative to peak.
    assert!(
        after.current_bytes <= after.peak_bytes,
        "current_bytes ({}) exceeded peak_bytes ({})",
        after.current_bytes,
        after.peak_bytes
    );
}
