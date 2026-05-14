//! Per-allocation overhead micro-benchmark.
//!
//! Runs a tight allocate/deallocate loop and reports the average
//! nanoseconds per allocation observed end-to-end. The number
//! includes the cost of `System` itself; the `ModAlloc` overhead is
//! the difference between this number and a System-only baseline.
//!
//! REPS section 6 sets a target of <50ns of `ModAlloc` overhead per
//! allocation on x86_64. This bench is a sanity-check, not a
//! rigorous benchmark; for precise measurement, prefer a tool that
//! controls CPU pinning and isolates noise.
//!
//! Run with: `cargo run --release --example bench_overhead`

use std::time::Instant;

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

const WARMUP: usize = 50_000;
const N: usize = 1_000_000;
const SIZE: usize = 64;

fn main() {
    for _ in 0..WARMUP {
        let v: Vec<u8> = Vec::with_capacity(SIZE);
        std::hint::black_box(&v);
    }

    GLOBAL.reset();
    let start = Instant::now();
    for _ in 0..N {
        let v: Vec<u8> = Vec::with_capacity(SIZE);
        std::hint::black_box(&v);
    }
    let elapsed = start.elapsed();

    let snap = GLOBAL.snapshot();
    let per_cycle_ns = elapsed.as_nanos() as f64 / N as f64;

    println!("bench_overhead:");
    println!("  iterations:           {N}");
    println!("  allocation size:      {SIZE} bytes");
    println!("  elapsed:              {elapsed:?}");
    println!("  per alloc+dealloc:    {per_cycle_ns:.1} ns");
    println!();
    println!("counter snapshot after run:");
    println!("  alloc_count:   {}", snap.alloc_count);
    println!("  total_bytes:   {}", snap.total_bytes);
    println!("  current_bytes: {}", snap.current_bytes);
    println!("  peak_bytes:    {}", snap.peak_bytes);
}
