<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <strong>mod-alloc</strong>
    <br>
    <sup><sub>ALLOCATION PROFILING FOR RUST</sub></sup>
</h1>

<p align="center">
    <a href="https://crates.io/crates/mod-alloc"><img alt="crates.io" src="https://img.shields.io/crates/v/mod-alloc.svg"></a>
    <a href="https://crates.io/crates/mod-alloc"><img alt="downloads" src="https://img.shields.io/crates/d/mod-alloc.svg"></a>
    <a href="https://docs.rs/mod-alloc"><img alt="docs.rs" src="https://docs.rs/mod-alloc/badge.svg"></a>
    <img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.75%2B-blue.svg?style=flat-square" title="Rust Version">
    <a href="https://github.com/jamesgober/mod-alloc/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/jamesgober/mod-alloc/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
    Allocation counters, peak resident, and call-site grouping.<br>
    Zero external dependencies in the hot path.
</p>

---

## What it does

`mod-alloc` is a global-allocator wrapper that tracks every
allocation and deallocation. It answers:

- **How many allocations did this code path make?**
- **How many total bytes were allocated?**
- **What was the peak resident memory?**
- **Which call-sites caused the most allocations?** (with `backtraces` feature)

Designed as a lean replacement for `dhat` with:

- **MSRV 1.75** (vs dhat's 1.85+)
- **Zero external dependencies** in the hot path (no `backtrace` crate)
- **Lower overhead** per allocation via purpose-built inline capture
- **DHAT-compatible output** so existing viewer tools work (via the `dhat-compat` feature)

## Quick start

```rust
use mod_alloc::{Profiler, ModAlloc};

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

fn main() {
    let p = Profiler::start();

    let v: Vec<u64> = (0..1000).collect();
    drop(v);

    let stats = p.stop();
    println!("Allocations: {}", stats.alloc_count);
    println!("Total bytes: {}", stats.total_bytes);
    println!("Peak bytes:  {}", stats.peak_bytes);
}
```

## Feature flags

```toml
[dependencies]
mod-alloc = "0.9"                                                      # counters only (default)
mod-alloc = { version = "0.9", features = ["backtraces"] }             # + call-site capture (lands in v0.9.1)
mod-alloc = { version = "0.9", features = ["dhat-compat"] }            # + DHAT-format output (lands in v0.9.3)
```

## Why a new allocation profiler

`dhat` is the de-facto standard but its dependency chain
(`backtrace` 0.3.76 → `addr2line` 0.25.1) locks consumers at Rust
1.85. For projects with broader MSRV targets, this is a real cost.

`mod-alloc` provides the same core capability with inline backtrace
capture (frame-pointer-based, x86_64 + aarch64 initially) and no
external dependencies. The trade: fewer architectures supported in
v1.0; we add ARM32, RISC-V, etc. based on demand.

## Status

`v0.9.0` ships Tier 1 (counters). Installing `ModAlloc` as
`#[global_allocator]` tracks every allocation, deallocation,
reallocation, and zero-init allocation against four lock-free
atomic counters. Per-allocation overhead measures under 50 ns on
x86_64 (`cargo run --release --example bench_overhead`). Tier 2
(inline backtrace capture) lands in `v0.9.1`. Tier 3
(DHAT-compatible JSON output) lands in `v0.9.3`. The `1.0` release
freezes the public API and the wire format.

## Minimum supported Rust version

`1.75`, pinned in `Cargo.toml` and verified by CI.

## License

Apache-2.0. See [LICENSE](LICENSE).



<!-- COPYRIGHT
---------------------------------->
<div align="center">
  <br>
  <h2></h2>
  Copyright &copy; 2026 James Gober.
</div>