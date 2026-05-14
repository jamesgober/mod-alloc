//! Symbolicated per-call-site report demo.
//!
//! Run with:
//!   cargo run --release --features symbolicate --example symbolicate
//!
//! Requires frame pointers in the build (`.cargo/config.toml`
//! already sets this for in-crate runs) and debug info present in
//! the binary (the default `cargo run --release` build keeps
//! enough info for `addr2line` / `pdb` to resolve our own
//! functions).

#[cfg(feature = "symbolicate")]
#[global_allocator]
static GLOBAL: mod_alloc::ModAlloc = mod_alloc::ModAlloc::new();

#[cfg(feature = "symbolicate")]
#[inline(never)]
fn alloc_small() {
    let v: Vec<u8> = Vec::with_capacity(64);
    std::hint::black_box(&v);
}

#[cfg(feature = "symbolicate")]
#[inline(never)]
fn alloc_medium() {
    let v: Vec<u8> = Vec::with_capacity(1024);
    std::hint::black_box(&v);
}

#[cfg(feature = "symbolicate")]
#[inline(never)]
fn alloc_large() {
    let v: Vec<u8> = Vec::with_capacity(64 * 1024);
    std::hint::black_box(&v);
}

#[cfg(feature = "symbolicate")]
fn main() {
    for _ in 0..500 {
        alloc_small();
    }
    for _ in 0..100 {
        alloc_medium();
    }
    for _ in 0..10 {
        alloc_large();
    }

    let mut report = GLOBAL.symbolicated_report();
    report.sort_by_key(|s| std::cmp::Reverse(s.total_bytes));

    let snap = GLOBAL.snapshot();
    println!("Process-wide:");
    println!(
        "  {} allocations, {} total bytes, peak {} bytes",
        snap.alloc_count, snap.total_bytes, snap.peak_bytes
    );
    println!();

    println!("Top call sites by total bytes:");
    println!(
        "{:>2}  {:>10}  {:>14}  top frame",
        "#", "count", "total_bytes"
    );
    for (rank, site) in report.iter().take(10).enumerate() {
        let top = &site.frames[0];
        let name = top.function.as_deref().unwrap_or("<unresolved>");
        let loc = match (top.file.as_ref(), top.line) {
            (Some(f), Some(l)) => format!("  {}:{}", f.display(), l),
            _ => String::new(),
        };
        println!(
            "{rank:>2}  {count:>10}  {bytes:>14}  {name}{loc}",
            rank = rank,
            count = site.count,
            bytes = site.total_bytes,
            name = name,
            loc = loc,
        );
        // Show inlined expansions if any.
        for inlined in site.frames.iter().skip(1).take_while(|f| f.inlined) {
            let n = inlined.function.as_deref().unwrap_or("<unresolved>");
            println!("        inlined: {n}");
        }
    }
}

#[cfg(not(feature = "symbolicate"))]
fn main() {
    eprintln!(
        "this example requires the `symbolicate` feature; run with \
         `cargo run --features symbolicate --example symbolicate`"
    );
}
