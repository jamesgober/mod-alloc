//! Per-call-site report demo.
//!
//! Run with:
//!   cargo run --release --features backtraces --example backtraces
//!
//! Requires frame pointers for useful output. The in-crate
//! `.cargo/config.toml` enables `-C force-frame-pointers=yes` for
//! this crate's own builds; downstream users opting into the
//! `backtraces` feature must enable the flag in their own build.

#[cfg(feature = "backtraces")]
#[global_allocator]
static GLOBAL: mod_alloc::ModAlloc = mod_alloc::ModAlloc::new();

#[cfg(feature = "backtraces")]
#[inline(never)]
fn alloc_small() {
    let v: Vec<u8> = Vec::with_capacity(64);
    std::hint::black_box(&v);
}

#[cfg(feature = "backtraces")]
#[inline(never)]
fn alloc_medium() {
    let v: Vec<u8> = Vec::with_capacity(1024);
    std::hint::black_box(&v);
}

#[cfg(feature = "backtraces")]
#[inline(never)]
fn alloc_large() {
    let v: Vec<u8> = Vec::with_capacity(64 * 1024);
    std::hint::black_box(&v);
}

#[cfg(feature = "backtraces")]
fn main() {
    for _ in 0..1_000 {
        alloc_small();
    }
    for _ in 0..100 {
        alloc_medium();
    }
    for _ in 0..10 {
        alloc_large();
    }

    let snap = GLOBAL.snapshot();
    println!("Process-wide snapshot:");
    println!("  alloc_count:   {}", snap.alloc_count);
    println!("  total_bytes:   {}", snap.total_bytes);
    println!("  current_bytes: {}", snap.current_bytes);
    println!("  peak_bytes:    {}", snap.peak_bytes);
    println!();

    let mut sites = GLOBAL.call_sites();
    sites.sort_by_key(|s| std::cmp::Reverse(s.total_bytes));

    println!("Top 10 call sites by total bytes:");
    println!(
        "{:>10}  {:>14}  {:>4}  {:>18}",
        "count", "total_bytes", "frm", "top frame"
    );
    for (rank, site) in sites.iter().take(10).enumerate() {
        println!(
            "{rank:>2}: {count:>6}  {bytes:>14}  {frm:>4}  {top:#018x}",
            rank = rank,
            count = site.count,
            bytes = site.total_bytes,
            frm = site.frame_count,
            top = site.frames[0],
        );
    }
}

#[cfg(not(feature = "backtraces"))]
fn main() {
    eprintln!(
        "this example requires the `backtraces` feature; run with \
         `cargo run --features backtraces --example backtraces`"
    );
}
