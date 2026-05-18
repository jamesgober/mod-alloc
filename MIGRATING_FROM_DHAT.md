# Migrating from dhat-rs

This guide walks through swapping `dhat` for `mod-alloc` in a
Rust project. The compatibility surface lives behind the
`dhat-compat` cargo feature and mirrors `dhat-rs`'s public API
method-for-method, so the migration is typically a one-line
import change.

## Why migrate

`dhat` (the Rust crate) is excellent but its dependency chain
(`backtrace 0.3.76` → `addr2line 0.25.1`) forces consumers onto
MSRV `1.85+`. Projects with a broader MSRV target — anything
that needs to support stable Rust from before mid-2025 — pay
a real cost for that.

`mod-alloc` provides equivalent core profiling with:

- **MSRV 1.75.** Verified by CI on every push.
- **Zero external dependencies on the alloc hot path.** Optional
  symbolication crates only activate when you opt in to the
  `symbolicate` feature.
- **Inline frame-pointer walking** for backtrace capture, with
  no `backtrace` crate dependency.

The trade-off is fewer supported architectures (`x86_64` +
`aarch64` for Tier 2 capture; other targets still run with
counter-only Tier 1). Most production Rust deploys are one of
the supported two.

## The one-line swap

Take typical `dhat`-using code:

```rust
use dhat;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    // ... your work ...

    let stats = dhat::HeapStats::get();
    println!("total bytes: {}", stats.total_bytes);
}
```

Change exactly one line — the import — and the rest works:

```rust
use mod_alloc::dhat_compat as dhat;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    // ... your work ...

    let stats = dhat::HeapStats::get();
    println!("total bytes: {}", stats.total_bytes);
}
```

`Cargo.toml` swaps too:

```toml
# Before
[dependencies]
dhat = "0.3"

# After
[dependencies]
mod-alloc = { version = "0.9", features = ["dhat-compat"] }
```

## API surface mapping

| dhat-rs                       | mod-alloc `dhat_compat`          | Notes                           |
|-------------------------------|----------------------------------|---------------------------------|
| `dhat::Alloc`                 | `Alloc`                          | unit struct, same usage         |
| `dhat::Profiler`              | `Profiler`                       | RAII; writes JSON on drop       |
| `dhat::Profiler::new_heap()`  | `Profiler::new_heap()`           | identical                       |
| `dhat::Profiler::new_ad_hoc()`| `Profiler::new_ad_hoc()`         | identical                       |
| `dhat::Profiler::builder()`   | `Profiler::builder()`            | identical                       |
| `dhat::ProfilerBuilder`       | `ProfilerBuilder`                | same builder methods            |
| `.ad_hoc()`                   | `.ad_hoc()`                      | identical                       |
| `.testing()`                  | `.testing()`                     | identical                       |
| `.file_name(p)`               | `.file_name(p)`                  | identical                       |
| `.trim_backtraces(n)`         | `.trim_backtraces(n)`            | clamped to walker cap (8)       |
| `.build()`                    | `.build()`                       | identical                       |
| `dhat::HeapStats`             | `HeapStats`                      | identical fields                |
| `dhat::HeapStats::get()`      | `HeapStats::get()`               | identical                       |
| `dhat::AdHocStats`            | `AdHocStats`                     | identical fields                |
| `dhat::AdHocStats::get()`     | `AdHocStats::get()`              | identical                       |
| `dhat::ad_hoc_event(w)`       | `ad_hoc_event(w)`                | identical                       |
| `dhat::assert!`               | _not yet shipped_                | use `HeapStats::get()` directly |
| `dhat::assert_eq!`            | _not yet shipped_                | use `HeapStats::get()` directly |
| `dhat::assert_ne!`            | _not yet shipped_                | use `HeapStats::get()` directly |

The three assertion macros are the only intentional gap. They
require a stored snapshot comparator that hasn't been ported yet
— if you need them, file an issue or use `HeapStats::get()` in
your own assertions.

## Behavioural differences

These are documented divergences. None affect the common
profiling workflow.

### 1. Backtrace depth cap

dhat uses the `backtrace` crate which captures the full call
stack (potentially hundreds of frames). mod-alloc's inline
frame-pointer walker captures up to 8 frames per allocation.

`ProfilerBuilder::trim_backtraces(Some(n))` is accepted for
API parity but silently clamped:
- `Some(n)` where `n <= 8` produces up to `n` frames
- `Some(n)` where `n > 8` produces up to 8 frames
- `None` produces up to 8 frames (the default)

For most call-site grouping, 8 frames is more than enough; the
upstream `dh_view.html` viewer collapses identical inner stacks
regardless.

### 2. Drop-time file-write errors swallowed

dhat-rs swallows IO errors from `Profiler::drop` silently
(`Drop` can't propagate `?`). mod-alloc does the same. If you
need error visibility, write the JSON explicitly via
`ModAlloc::write_dhat_json` from the underlying allocator
type before the Profiler drops.

### 3. Double-Profiler is a no-op, not a panic

dhat-rs panics if more than one `Profiler` is alive at a time.
mod-alloc treats the second construction as a no-op and
reports "last writer wins" on the JSON file. Real-world code
rarely hits this, and the no-op behaviour is friendlier in test
harnesses.

### 4. JSON byte-identity not guaranteed

Both crates emit `dhatFileVersion: 2` documents that the
upstream `dh_view.html` viewer loads. Field ordering,
float-formatting details, and certain decorative fields
(`tg`, `te`, etc.) may differ between the two. The viewer
doesn't care; downstream tooling that does byte-level
comparison may.

### 5. Frame string format

mod-alloc emits `0x{addr:x}: <unresolved>` without the
`symbolicate` feature, or `0x{addr:x}: <function> ({file}:{line})`
with it. dhat emits richer strings out of the box thanks to its
mandatory `backtrace`/`addr2line` deps. Enable `symbolicate`
to match dhat's resolution quality:

```toml
mod-alloc = { version = "0.9", features = ["dhat-compat", "symbolicate"] }
```

## Build requirements

`mod-alloc` Tier 2 capture requires frame pointers. Add to your
project's `.cargo/config.toml`:

```toml
[build]
rustflags = ["-C", "force-frame-pointers=yes"]
```

The crate's `build.rs` emits a `cargo:warning=` at compile time
if `RUSTFLAGS` is missing this. Without it, the walker degrades
gracefully (shallow traces) but doesn't crash.

For maximally informative traces, rebuild `std` with frame
pointers:

```bash
RUSTFLAGS="-C force-frame-pointers=yes" \
  cargo +nightly -Z build-std=std test
```

dhat uses libunwind and doesn't need this flag — but you also
inherit dhat's MSRV 1.85+ requirement. Pick your trade.

## Viewing the report

Identical to dhat-rs: open the produced `dhat-heap.json` in
`dh_view.html` (shipped with Valgrind). The viewer renders both
crates' output the same way.

## Rolling back

The `dhat_compat` module is purely additive. Reverting:

1. Change the import back: `use dhat;`
2. Swap the dep: `dhat = "0.3"` instead of
   `mod-alloc = { ... features = ["dhat-compat"] }`
3. Rebuild.

No public API in your own code changes.

## Reporting gaps

If you hit a dhat feature mod-alloc doesn't cover, file an
issue at <https://github.com/jamesgober/mod-alloc/issues>.
The compatibility surface is a living target — if dev-bench or
your project needs an additional dhat method ported, it gets
prioritised.
