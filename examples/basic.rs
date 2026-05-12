//! Minimal example: snapshot allocation counters.
//!
//! Run with: `cargo run --example basic`

use mod_alloc::{ModAlloc, Profiler};

fn main() {
    let alloc = ModAlloc::new();
    let s0 = alloc.snapshot();
    println!("Initial: {s0:?}");

    let p = Profiler::start();
    let stats = p.stop();
    println!("Profiler delta: {stats:?}");
}
