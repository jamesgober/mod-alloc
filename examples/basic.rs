//! Install `ModAlloc` as the global allocator and report stats.
//!
//! Run with: `cargo run --release --example basic`

use mod_alloc::{ModAlloc, Profiler};

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

fn main() {
    let p = Profiler::start();

    let v: Vec<u64> = (0..1_000).collect();
    let sum: u64 = v.iter().sum();
    drop(v);

    let mut owned: Vec<String> = Vec::with_capacity(100);
    for i in 0..100 {
        owned.push(format!("item-{i}"));
    }
    drop(owned);

    let delta = p.stop();
    println!("Profiler delta (alloc/total/current = delta; peak = absolute):");
    println!("  alloc_count:   {}", delta.alloc_count);
    println!("  total_bytes:   {}", delta.total_bytes);
    println!("  current_bytes: {}", delta.current_bytes);
    println!("  peak_bytes:    {}", delta.peak_bytes);

    let snap = GLOBAL.snapshot();
    println!();
    println!("Process-wide snapshot:");
    println!("  alloc_count:   {}", snap.alloc_count);
    println!("  total_bytes:   {}", snap.total_bytes);
    println!("  current_bytes: {}", snap.current_bytes);
    println!("  peak_bytes:    {}", snap.peak_bytes);

    println!();
    println!("(workload checksum: {sum})");
}
