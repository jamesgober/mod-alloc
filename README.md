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

`mod-alloc` is a `#[global_allocator]` wrapper that tracks every
allocation and answers four questions for the code that runs while
it is installed:

- **How many allocations did this code path make?**
- **How many total bytes did it allocate?**
- **What was the peak resident memory?**
- **Which call sites did most of the allocating?** (with the
  `backtraces` feature)

The allocation hot path is `std`-only. No `backtrace`, no `libc`,
no `gimli` in the alloc path. Inline frame-pointer walking on
`x86_64` and `aarch64` for call-site capture; raw `mmap` /
`VirtualAlloc` for the per-thread arena and the global
aggregation table.

The opt-in `symbolicate` feature pulls in pure-Rust `addr2line` +
`object` + `pdb` + `rustc-demangle` for offline report
generation; the alloc hot path stays untouched.

## Quick start

```rust
use mod_alloc::{Profiler, ModAlloc};

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

fn main() {
    let p = Profiler::start();

    let v: Vec<u64> = (0..1_000).collect();
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
mod-alloc = "1"                                           # Tier 1: counters (default)
mod-alloc = { version = "1", features = ["backtraces"] }  # Tier 2: call-site capture
mod-alloc = { version = "1", features = ["symbolicate"] } # + function/file/line names
mod-alloc = { version = "1", features = ["dhat-compat"] } # DHAT JSON output + dhat-rs drop-in
```

| Feature       | What it adds                                                        | Since   |
|---------------|---------------------------------------------------------------------|---------|
| `counters`    | Four lock-free counters via `GlobalAlloc` (default)                 | `0.9.0` |
| `backtraces`  | Inline FP walk + per-call-site aggregation (~11 ns overhead)        | `0.9.1` |
| `symbolicate` | Resolve raw addresses to `(function, file, line)`                   | `0.9.2` |
| `dhat-compat` | DHAT JSON output + `dhat_compat` drop-in surface for `dhat-rs`      | `0.9.3` |

## Backtraces

Enabling the `backtraces` feature requires frame pointers in the
caller's build:

```toml
# .cargo/config.toml
[build]
rustflags = ["-C", "force-frame-pointers=yes"]
```

The crate's `build.rs` emits a `cargo:warning=` at compile time if
`RUSTFLAGS` is missing this. Without it the walker degrades
gracefully (returns shallow or empty traces) but does not crash.

The aggregation-table size is configurable at process start:

```bash
MOD_ALLOC_BUCKETS=16384 ./your-binary
```

Default is 4,096 buckets (~384 KB). Range `[64, 1_048_576]`,
rounded up to the next power of two.

## Drop-in replacement for dhat-rs

With the `dhat-compat` feature, mod-alloc exposes a
`dhat_compat` module that mirrors dhat-rs's public surface
method-for-method. Migrate by changing a single import line:

```rust
use mod_alloc::dhat_compat as dhat;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();
    // ... work ...
    // _profiler drops here → writes dhat-heap.json
}
```

The full mapping (and documented gaps) lives in
[`MIGRATING_FROM_DHAT.md`](MIGRATING_FROM_DHAT.md). The
motivating benefit: mod-alloc holds MSRV 1.75 while dhat-rs
forces 1.85+ through its `backtrace → addr2line` chain.

## DHAT-compatible JSON output

With the `dhat-compat` feature enabled, the per-call-site report
serialises to the JSON shape understood by the upstream
`dh_view.html` viewer shipped with Valgrind:

```rust
GLOBAL.write_dhat_json("dhat-heap.json")?;
```

Combine with `symbolicate` for resolved function names plus file
and line where available:

```toml
mod-alloc = { version = "0.9", features = ["symbolicate", "dhat-compat"] }
```

The JSON writer is hand-rolled — no `serde` / `serde_json`
dependency. See [`examples/dhat_json.rs`](examples/dhat_json.rs)
for a complete walk-through.

## Performance

Measured per allocation, end to end, on a Windows x86_64 dev host
with `cargo run --release --example bench_overhead`:

| Build                                  | Per alloc + dealloc cycle |
|----------------------------------------|--------------------------:|
| Tier 1 only (`counters`, default)      |        **45.5 ns**        |
| Tier 1 + Tier 2 (`backtraces`, v0.9.5) |        **56.9 ns**        |
| Tier 1 + Tier 2 (`backtraces`, v0.9.4) |       **~2,050 ns**       |

Both tiers comfortably clear the REPS section 6 targets (Tier 1
<50 ns, Tier 2 <200 ns of additional overhead). v0.9.5 dropped
Tier 2 overhead from ~2,000 ns to ~11 ns by removing the
per-thread arena layer and writing each captured event straight
to the global aggregation table — `table::record`'s steady-state
matching path is two atomic operations, so the arena's batching
no longer paid for itself.

## Why a new allocation profiler

`dhat` is the de facto standard for allocation profiling in Rust,
but its dependency chain (`backtrace 0.3.76` → `addr2line 0.25.1`)
forces consumers to MSRV `1.85`+. For projects with a broader MSRV
target, that cost is real.

`mod-alloc` provides the same core capability with inline
backtrace capture (frame-pointer-based, `x86_64` + `aarch64`) and
no external dependencies. The trade-off is fewer architectures
supported in `1.0`; ARM32, RISC-V, and others land based on
demand.

## Status

| Milestone                                  | Version    | State                          |
|--------------------------------------------|------------|--------------------------------|
| Name-claim placeholder                     | `v0.1.0`   | shipped                        |
| Real `GlobalAlloc` + Tier 1 counters       | `v0.9.0`   | shipped                        |
| Tier 2: inline backtrace capture           | `v0.9.1`   | shipped                        |
| Symbolication for reports                  | `v0.9.2`   | shipped                        |
| Tier 3: DHAT-compatible JSON output        | `v0.9.3`   | shipped                        |
| dhat-rs drop-in compat surface             | `v0.9.4`   | shipped                        |
| `dev-bench` swap (consumer side)           | n/a        | shipped as `dev-bench v0.9.7`  |
| Tier 2 perf optimisation                   | `v0.9.5`   | shipped                        |
| **Stable API**                             | **`v1.0.0`** | **shipped**                  |

`v1.0.0` freezes the public API and the wire format. Breaking
changes after `1.0` require a major version bump per Semantic
Versioning.

## Out of scope

- Replacing the system allocator. Use `mimalloc` or
  `jemallocator` for that.
- Use-after-free / double-free detection. Use AddressSanitizer.
- Source-level instrumentation (build.rs, proc macros). The one
  build.rs in this crate exists solely to detect missing frame
  pointers at compile time.

## Minimum supported Rust version

`1.75`, pinned in `Cargo.toml` and verified by CI on every push.

## License

Apache-2.0. See [LICENSE](LICENSE).



<!-- COPYRIGHT
---------------------------------->
<div align="center">
  <br>
  <h2></h2>
  Copyright &copy; 2026 James Gober.
</div>
