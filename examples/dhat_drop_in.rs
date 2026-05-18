//! dhat-rs drop-in swap demo.
//!
//! Compare to dhat-rs's introductory example: change only the
//! import line and this code runs against mod-alloc.
//!
//! Run with:
//!   cargo run --release --features dhat-compat --example dhat_drop_in
//!
//! Drops `dhat-heap.json` in the current working directory on
//! exit. Open it in `dh_view.html` (shipped with Valgrind) to
//! inspect the per-call-site report.

#[cfg(feature = "dhat-compat")]
use mod_alloc::dhat_compat as dhat;

#[cfg(feature = "dhat-compat")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[cfg(feature = "dhat-compat")]
#[inline(never)]
fn alloc_small() {
    let v: Vec<u8> = Vec::with_capacity(64);
    std::hint::black_box(&v);
}

#[cfg(feature = "dhat-compat")]
#[inline(never)]
fn alloc_medium() {
    let v: Vec<u8> = Vec::with_capacity(1024);
    std::hint::black_box(&v);
}

#[cfg(feature = "dhat-compat")]
fn main() {
    let _profiler = dhat::Profiler::new_heap();

    for _ in 0..500 {
        alloc_small();
    }
    for _ in 0..100 {
        alloc_medium();
    }

    let stats = dhat::HeapStats::get();
    println!(
        "total_blocks: {}, total_bytes: {}, max_bytes: {}, curr_blocks: {}",
        stats.total_blocks, stats.total_bytes, stats.max_bytes, stats.curr_blocks,
    );
    println!("_profiler drops at end of main → writes dhat-heap.json");
}

#[cfg(not(feature = "dhat-compat"))]
fn main() {
    eprintln!(
        "this example requires the `dhat-compat` feature; run with \
         `cargo run --features dhat-compat --example dhat_drop_in`"
    );
}
