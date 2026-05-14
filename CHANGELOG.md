# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
