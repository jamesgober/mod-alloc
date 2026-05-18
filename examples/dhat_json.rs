//! DHAT-compatible JSON output demo.
//!
//! Run with:
//!   cargo run --release --features dhat-compat --example dhat_json
//!
//! Or with symbolicated frames in the JSON:
//!   cargo run --release --features symbolicate,dhat-compat --example dhat_json
//!
//! Drops a `dhat-heap.json` file in the current working directory.
//! Load that file in `dh_view.html` (shipped with Valgrind) to
//! inspect the per-call-site report visually.

#[cfg(feature = "dhat-compat")]
#[global_allocator]
static GLOBAL: mod_alloc::ModAlloc = mod_alloc::ModAlloc::new();

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
#[inline(never)]
fn alloc_large() {
    let v: Vec<u8> = Vec::with_capacity(64 * 1024);
    std::hint::black_box(&v);
}

#[cfg(feature = "dhat-compat")]
fn main() -> std::io::Result<()> {
    for _ in 0..500 {
        alloc_small();
    }
    for _ in 0..100 {
        alloc_medium();
    }
    for _ in 0..10 {
        alloc_large();
    }

    let snap = GLOBAL.snapshot();
    println!(
        "captured {} allocations, {} total bytes, peak {} bytes",
        snap.alloc_count, snap.total_bytes, snap.peak_bytes
    );

    let path = std::path::Path::new("dhat-heap.json");
    GLOBAL.write_dhat_json(path)?;

    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    println!("wrote {}", abs.display());
    println!("open it in dh_view.html to inspect.");
    Ok(())
}

#[cfg(not(feature = "dhat-compat"))]
fn main() {
    eprintln!(
        "this example requires the `dhat-compat` feature; run with \
         `cargo run --features dhat-compat --example dhat_json`"
    );
}
