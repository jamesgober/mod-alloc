# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Tier 2: inline backtrace capture (`backtraces` feature).** Each
  tracked allocation, zero-init allocation, and reallocation
  captures up to 8 frames of its call site via inline
  frame-pointer walking. Available on `x86_64` and `aarch64`.
  Other architectures compile but capture is a no-op.
- **`ModAlloc::call_sites()`** drains the per-call-site
  aggregation table into a `Vec<CallSiteStats>`. Each row carries
  raw return addresses (top of stack first), the number of
  allocations attributed to the site, and the total bytes.
  Symbolication ships in v0.9.2.
- **`CallSiteStats`** public type behind the `backtraces` feature.
- **Per-thread arena** (64 KB OS-page region per thread, 512
  events per flush) and **global aggregation table** (4,096
  buckets by default, ~384 KB) allocated through raw
  `mmap` / `VirtualAlloc` so the backtrace path never recurses
  into `ModAlloc::alloc` for its own state.
- **`MOD_ALLOC_BUCKETS` env var** to override the
  aggregation-table size at process start. Value is rounded up to
  the next power of two and clamped to `[64, 1_048_576]`.
- **`build.rs`** (one-off approved exception): warns at compile
  time when `RUSTFLAGS` is missing `-C force-frame-pointers=yes`
  and the `backtraces` feature is on. See
  `.dev/DIRECTIVES.md` section 2.1 for the documented exception.
- **`.cargo/config.toml`** in the crate root enables frame
  pointers for the crate's own builds so the test suite and
  examples produce useful traces. Downstream consumers must
  enable the flag in their own builds.
- New tests:
  - `tests/backtrace_real_chain.rs`: captures from a deeply
    nested `#[inline(never)]` call chain.
  - `tests/backtrace_fuzz.rs`: SplitMix64-driven random workload
    proving the walker is total under varied allocation patterns
    (10,000 iterations).
  - `tests/backtrace_concurrent.rs`: 32-thread aggregation
    stress test.
  - `src/backtrace/*` unit tests cover hash determinism, walker
    safety checks (null, alignment, out-of-range, non-monotonic,
    max-frame cap), arena round-trip, table claim races, and
    stack-bounds discovery.
- **`examples/backtraces.rs`** demonstrates installing
  `ModAlloc`, exercising a few distinct call paths, and printing
  the top sites by total bytes.
- **CI: AddressSanitizer nightly job.** A dedicated job in
  `.github/workflows/ci.yml` runs the test suite under
  `-Zsanitizer=address` on Linux x86_64 to catch any UB in the
  unsafe FP-walker path that survives the in-walker safety
  checks.

### Changed

- `GlobalAlloc::alloc`, `alloc_zeroed`, and `realloc` invoke
  `backtrace::record_event` after the existing counter update
  when the `backtraces` feature is on. `dealloc` does not capture
  (matches dhat: call sites describe who allocated, not who
  freed).
- Per maintainer guidance, realloc captures all events including
  shrinks, matching dhat's per-event accounting. Documented in
  the rustdoc.
- CI workflow runs the `backtraces` test suite with
  `RUSTFLAGS="-C force-frame-pointers=yes"` so traces are
  meaningful on hosted runners.

### Design notes

- **Reentrancy on the backtrace path.** The walker reads memory
  inside the cached stack bounds (which are queried via
  `GetCurrentThreadStackLimits` on Windows,
  `pthread_getattr_np` on Linux, `pthread_get_stackaddr_np` on
  Darwin / BSD). All reads are pointer-aligned and in-range; no
  page faults are possible. The existing `IN_ALLOC` reentrancy
  guard from v0.9.0 catches any pathological allocation
  triggered transitively from inside the backtrace path (e.g.
  libc lazy-init during the first `pthread_getattr_np`).
- **No `HashMap` in the hot path.** The global aggregation table
  is a fixed-size open-addressed array allocated once via raw OS
  pages, with atomic per-bucket CAS for claim and linear probing
  for index collisions. Hash collisions on the 64-bit FxHash are
  a documented limitation (different sites with identical hashes
  get conflated).
- **Bucket publish protocol.** Each bucket uses a two-phase
  claim: CAS on `hash` first (Release), then write
  `sample_frames`, then store `frame_count` with Release. Readers
  gate on `frame_count > 0` after observing a non-zero hash;
  this prevents torn reads of the sample frames.

### Migration

The default build (Tier 1 only) is unchanged. Existing callers
need no edits.

Users opting in to the `backtraces` feature must add
`-C force-frame-pointers=yes` to their build configuration. The
included `build.rs` emits a `cargo:warning=` at compile time if
this is missing.

## [0.9.0] - 2026-05-13

### Added

- `unsafe impl GlobalAlloc for ModAlloc`. Installing `ModAlloc` as
  `#[global_allocator]` now records every alloc, dealloc, realloc,
  and `alloc_zeroed` event into the four Tier 1 counters
  (`alloc_count`, `total_bytes`, `peak_bytes`, `current_bytes`).
- Lock-free counter updates on the hot path using `AtomicU64` with
  `Relaxed` ordering, plus `fetch_max` for the peak high-water
  mark.
- Thread-local reentrancy guard. The allocator hook is recursion
  safe: if any code transitively triggered from inside the tracking
  path attempts to allocate, the nested call bypasses tracking and
  forwards directly to the System allocator. The flag is
  `const`-initialised so TLS access on the hot path does not
  allocate.
- Lazy `Profiler` registration via a process-wide `AtomicPtr`
  handle that the `GlobalAlloc` impl populates on first call.
  `Profiler::start()` and `Profiler::stop()` snapshot the installed
  allocator without requiring an explicit registration step.
- New integration tests under `tests/`:
  - `counters_accuracy.rs`: single-thread counter correctness.
  - `concurrent_alloc.rs`: 64-thread x 5,000-allocation stress test.
  - `profiler_delta.rs`: Profiler delta math.
  - `reentrancy.rs`: reentrancy-guard smoke test.
- `examples/bench_overhead.rs`: per-allocation overhead
  micro-benchmark.

### Changed

- `ModAlloc::snapshot` now returns the running counter values from
  the live `GlobalAlloc` path. In v0.1.0 it always returned zeros.
- `ModAlloc::reset` zeroes the counters. Documented caveat:
  resetting while allocations are outstanding can cause
  `current_bytes` to wrap on subsequent deallocations; reset before
  any workload begins for clean accounting.
- `Profiler::stop` returns deltas for `alloc_count`, `total_bytes`,
  and `current_bytes`. `peak_bytes` is the absolute high-water mark
  observed during the profiling window (peak-delta has no
  meaningful semantic). The rustdoc on `Profiler::stop` documents
  this difference explicitly.
- `examples/basic.rs` now installs `ModAlloc` as
  `#[global_allocator]` and prints real counter values.
- Module-level rustdoc in `src/lib.rs` updated to describe counter
  semantics, the installation pattern, and the v0.9.0 status.

### Design notes

- **Per-thread arena deferred to v0.9.1.** The original ROADMAP
  entry for v0.9.0 envisaged a 64KB per-thread arena with periodic
  global aggregation. v0.9.0 ships with direct atomic increments
  on four shared counters instead. Per-thread buffering becomes
  load-bearing in v0.9.1 when backtrace state (32-64 bytes per
  allocation) would otherwise serialise on the global path; for
  four `u64` counters the indirection is not warranted yet. See
  `.dev/DESIGN.md` section 2 for the full rationale.
- **`backtraces` and `dhat-compat` features are no-ops in v0.9.0.**
  They remain defined in `Cargo.toml` so build matrices stay green
  and downstream callers can opt in once the features ship. Real
  implementations land in v0.9.1 (Tier 2: inline backtrace
  capture) and v0.9.3 (Tier 3: DHAT-compatible JSON output).

### Migration

The public API surface is unchanged. Callers using the v0.1.0
placeholder API (`ModAlloc::new`, `snapshot`, `reset`, `Profiler`,
`AllocStats`) continue to compile and behave identically when
`ModAlloc` is not installed as the global allocator. Callers that
install it now see real counter values where v0.1.0 returned zeros.

## [0.1.0] - 2026-05-11

### Added

- Initial crate skeleton.
- `ModAlloc` struct (the global allocator wrapper) with `new`,
  `snapshot`, `reset` methods. Placeholder implementation forwards
  to System allocator without tracking.
- `AllocStats` struct: alloc_count, total_bytes, peak_bytes,
  current_bytes.
- `Profiler` for scoped delta capture: `start` / `stop`.
- Feature flags: `std` (default), `counters` (default), `backtraces`,
  `dhat-compat`.
- Smoke tests.

### Note

This is the name-claim release. The `GlobalAlloc` trait is not yet
implemented; using `ModAlloc` as `#[global_allocator]` in 0.1.0
will not work. Real implementation lands in `0.9.x` along with:

- Per-thread arena-based tracking to avoid contention.
- Inline frame-pointer-based backtrace capture (x86_64 + aarch64).
- DHAT-compatible JSON output.
- Statistical validation suite.

[Unreleased]: https://github.com/jamesgober/mod-alloc/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/jamesgober/mod-alloc/compare/v0.1.0...v0.9.0
[0.1.0]: https://github.com/jamesgober/mod-alloc/releases/tag/v0.1.0
