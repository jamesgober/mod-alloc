# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.5] - 2026-05-18

Tier 2 (`backtraces`) perf optimisation. The per-allocation cost
of inline backtrace capture dropped roughly **32x** on a Windows
x86_64 dev host (2,051 ns → 56.9 ns total cycle, ≈ 11 ns of
Tier 2 overhead on top of Tier 1's 45.5 ns). Both tiers now
clear the REPS section 6 targets with significant headroom.

### Changed

- **Removed the per-thread arena layer.** Captured events go
  straight from `record_event` into the global aggregation
  table — `table::record`'s steady-state matching path is two
  atomic operations, so the arena's batching added cost
  (one TLS lookup + a 72-byte memcpy into an arena slot +
  periodic 512-event synchronous flush) without a real benefit
  once the bucket was warm.
- **Inlining hints on the hot path.** `table::record` and
  `current_stack_bounds` are now `#[inline]` / `#[inline(always)]`
  so thin-LTO stitches the full capture path
  (`record_event → walk → hash_frames → record`) into the
  calling `GlobalAlloc::alloc` body with no cross-module
  function-call boundaries.
- **Split `ensure_init` into fast/slow paths.** The steady-state
  inline accessor is three atomic loads; the first-event-per-
  process slow path (raw-page allocation + CAS publish) is
  out-of-line via `#[cold] #[inline(never)]`.

### Removed

- `src/backtrace/arena.rs` (and the per-thread `ARENA`
  thread-local). Dead after the bypass. The
  `flush_current_thread()` call inside `table::call_sites_report`
  is removed too — there is nothing to flush.
- `ENTRIES_PER_ARENA`, `ArenaState`, `ArenaSlot`, and the
  one arena unit test (`record_and_flush_round_trip`) went with
  the module.

### Notes

- mod-alloc skipped a `v0.9.5` release for the milestone of the
  same name (the dev-bench consumer-side swap, which shipped as
  `dev-bench v0.9.7` instead). `v0.9.5` here is mod-alloc's own
  next published version, carrying the Tier 2 perf pass.
- No public API changes. No new external dependencies. MSRV
  unchanged (`1.75`). The `ModAlloc::call_sites()`
  /  `symbolicated_report()` / `dhat_json_string()` /
  `write_dhat_json()` / `dhat_compat::*` surfaces all behave
  identically; only the internal capture pipeline shrank.
- Bench numbers in `README.md` updated to the post-optimisation
  measurements.

[0.9.5]: https://github.com/jamesgober/mod-alloc/compare/v0.9.4...v0.9.5

## [0.9.4] - 2026-05-18

### Added

- **`dhat_compat` module — drop-in replacement for `dhat-rs`.**
  Behind the existing `dhat-compat` cargo feature. Mirrors
  `dhat-rs`'s public surface method-for-method so consumers
  (notably `dev-bench`) can swap dhat for mod-alloc with a
  one-line import change (`use mod_alloc::dhat_compat as dhat;`).
- **`dhat_compat::Alloc`** — unit-struct global allocator
  matching `dhat::Alloc`'s usage pattern
  (`static A: Alloc = Alloc;`). Forwards every `GlobalAlloc`
  call to a process-wide static `ModAlloc` so `HeapStats::get()`
  and `Profiler` see the live counters.
- **`dhat_compat::Profiler` + `ProfilerBuilder`** — RAII handle
  that writes a DHAT JSON report on drop. `new_heap()`,
  `new_ad_hoc()`, `builder()`. Builder methods: `ad_hoc()`,
  `testing()`, `file_name()`, `trim_backtraces()`, `build()`.
- **`dhat_compat::HeapStats`** — six-field stats mirroring
  `dhat::HeapStats` exactly (including the
  `u64 total_*` / `usize curr_* max_*` asymmetry).
  `HeapStats::get()` snapshots the installed allocator.
- **`dhat_compat::AdHocStats` + `ad_hoc_event(weight)`** — ad-hoc
  mode counters with the same shape as `dhat`'s. Two atomic ops
  per event; no allocation.
- **`live_count` and `peak_live_count` fields on `AllocStats`.**
  Wired into `record_alloc` / `record_dealloc` to track
  currently-alive block counts. Backs `HeapStats::curr_blocks`
  and `max_blocks`. `record_realloc` does not touch them
  (a realloc is the same block from a count perspective —
  matches dhat's accounting).
- **Ad-hoc JSON writer** in `src/dhat_compat/ad_hoc_writer.rs`.
  Emits `dhatFileVersion: 2`, `mode: "ad-hoc"` files alongside
  the existing heap-mode writer.
- **`MIGRATING_FROM_DHAT.md`** project-root guide covering the
  one-line swap, API surface mapping, behavioural differences,
  and rollback steps.
- New tests:
  - `tests/dhat_compat_surface.rs` — 7 tests covering the swap
    pattern, live-block tracking, drop-time file write,
    testing-mode skip, ad-hoc event accumulation,
    `trim_backtraces` over-cap clamp, and `Profiler::new_heap`
    construction.
  - `src/dhat_compat/{mod,profiler,stats,ad_hoc_writer}.rs`
    unit tests cover unit-struct size, default construction,
    builder configuration, ad-hoc counter math, and JSON
    rendering.
  - In-file `src/lib.rs` tests gained
    `live_counters_track_alive_blocks` and
    `record_realloc_does_not_touch_live_count` to lock the
    new counter semantics.
- **`examples/dhat_drop_in.rs`** showing the one-line dhat-rs
  swap pattern.

### Changed

- **`AllocStats` gained two fields** (`live_count`,
  `peak_live_count`). This is a 0.x-window minor-breaking
  change for callers constructing `AllocStats` via struct
  literal — they must initialise the new fields. Callers using
  `ModAlloc::snapshot()` or `Profiler::stop()` are unaffected.
  Existing in-tree tests and the smoke test were updated to
  pass the new fields.
- **`record_alloc` performs two extra atomic ops**
  (`live_count.fetch_add`, `peak_live_count.fetch_max`).
  `record_dealloc` performs one extra (`live_count.fetch_sub`).
  Both use `Relaxed` ordering matching the existing counters.
  Measured added cost on the alloc hot path is under 3 ns —
  the new atomics dual-issue alongside the existing
  `current_bytes` updates.
- Module-level rustdoc in `src/lib.rs` updated for v0.9.4.

### Documented gaps from dhat-rs

- Backtrace depth capped at 8 frames (Tier 2 walker limit);
  `trim_backtraces` is accepted for API parity but silently
  clamps.
- Drop-time JSON write errors are swallowed silently — matches
  dhat-rs's behaviour.
- Double-Profiler construction is a no-op instead of a panic
  (dhat-rs panics); documented "last writer wins" on the JSON
  file.
- `dhat::assert!` / `dhat::assert_eq!` / `dhat::assert_ne!`
  macros are not yet ported. Use `HeapStats::get()` directly
  in test assertions until they ship.

### Migration note

This release is the unblock for projects whose MSRV target
forces them off `dhat = "0.3"` (which today requires
Rust 1.85+ through its `backtrace → addr2line` chain).
`mod-alloc` holds MSRV 1.75 and provides the same core
profiling surface.

## [0.9.3] - 2026-05-18

### Added

- **`dhat-compat` cargo feature wired up.** Previously a no-op
  placeholder shipped since v0.9.0, the feature now emits the
  per-call-site report as DHAT-compatible JSON
  (`dhatFileVersion: 2`, `mode: "rust-heap"`) that the upstream
  `dh_view.html` viewer shipped with Valgrind loads directly. No
  new external dependencies; the JSON writer is hand-rolled.
- **`ModAlloc::dhat_json_string() -> String`** renders the report
  as a JSON string. Allocates; call from outside the allocator
  hook.
- **`ModAlloc::write_dhat_json(path)`** writes the rendered JSON
  to `path` (mirrors `dhat-rs`'s `dhat-heap.json` convention).
  Returns `std::io::Result<()>`.
- **Frame-string formatting cfg-splits on `symbolicate`.** Without
  the `symbolicate` feature, frame strings carry raw hex
  addresses with `<unresolved>` placeholders. With `symbolicate`,
  the JSON carries function names plus (where the platform
  supports it) source file and line, with `[inlined]` flags on
  inlined expansions.
- **Frame-table deduplication.** The internal builder keeps a
  `HashMap<String, u32>` so identical frame strings reused across
  call sites share a single `ftbl` entry. Index 0 is reserved for
  the literal `"[root]"`.
- New tests:
  - `tests/dhat_json_shape.rs`: validates the top-level keys are
    present, the document starts/ends with object braces, a
    workload produces at least one program point, and
    `write_dhat_json` round-trips byte-for-byte with
    `dhat_json_string`.
  - `src/dhat_json/writer.rs` unit tests cover JSON string
    escaping (quote, backslash, newline, low control bytes,
    multibyte UTF-8 pass-through).
  - `src/dhat_json/frames.rs` unit tests cover frame-table
    interning and the raw frame-string format.
- **`examples/dhat_json.rs`** drops a `dhat-heap.json` file in
  the current working directory for inspection in the upstream
  viewer.

### Changed

- Module-level rustdoc in `src/lib.rs` updated to mention Tier 3
  DHAT JSON output.

### Notes

- The viewer hides time-and-lifetime columns (`tl`, `mb`, `mbk`,
  etc.) automatically because `mod-alloc` emits `bklt: false`. We
  do not track per-allocation lifetimes today and will not
  fabricate values just to populate columns.
- `eb` and `ebk` (at-end bytes / blocks) are emitted as `0`. The
  JSON can be written at any point during execution, so there is
  no meaningful "end" snapshot from our perspective.

## [0.9.2] - 2026-05-14

### Added

- **`symbolicate` cargo feature.** Turns the raw return addresses
  captured by v0.9.1's backtrace path into
  `(function, file, line)` tuples at report-generation time.
  Available on Linux, macOS, *BSD via `addr2line` + `object`;
  Windows via `pdb`. All four resolver crates plus
  `rustc-demangle` are pulled in only when the feature is on; the
  default build remains zero-runtime-dep.
- **`ModAlloc::symbolicated_report() -> Vec<SymbolicatedCallSite>`**.
  Drains the per-call-site table and resolves each frame against
  the running binary's debug info. Cached per-address across
  calls; allocates, so safe to call only from outside the
  allocator hook.
- **`SymbolicatedCallSite`** and **`SymbolicatedFrame`** public
  types behind `#[cfg(feature = "symbolicate")]`. `frames` is a
  `Vec` in stack-frame order with `inlined: bool` marking
  expansions from a single physical return address.
- **Self-binary discovery** via `std::env::current_exe`, cached
  in a `OnceLock<Option<PathBuf>>`. Falls back to unresolved
  frames if the binary path cannot be determined.
- **Per-process address cache** in `symbolicate::report` keyed by
  raw `u64` address. Cache miss runs the platform symbolicator
  once; subsequent calls reuse the cached `Vec<SymbolicatedFrame>`.
- **Approved external-dep exception** in `.dev/DIRECTIVES.md`
  section 2.2 documenting the `symbolicate` feature's deps
  (`addr2line`, `object`, `rustc-demangle`, `pdb`, plus a
  pinned `uuid = "=1.10.0"` to hold MSRV 1.75 against `pdb`'s
  latest transitive `uuid` 1.x).
- New tests:
  - `tests/symbolicate_self.rs`: self-symbolication of a known
    in-test-binary function; downgrades to shape-only when
    debug info is unavailable.
  - `tests/symbolicate_concurrent.rs`: 8 threads calling
    `symbolicated_report()` simultaneously, asserts no deadlock
    and consistent row count.
  - `src/symbolicate/self_binary.rs` unit tests for path
    resolution and caching.
- **`examples/symbolicate.rs`** prints top-10 call sites sorted
  by total bytes with resolved function names plus inlined
  frames where available.
- CI matrix gains explicit `backtraces` + `symbolicate` feature
  steps with the FP flag set.

### Changed

- Module-level rustdoc in `src/lib.rs` updated to mention Tier 2
  symbolication.
- The `symbolicate` feature implies `backtraces` (which in turn
  implies `std`). Activating it alone is sufficient.
- Linux/macOS produce richer output than Windows: DWARF inlining
  info is more complete than PDB's `S_INLINESITE` decoding (the
  latter is deferred for v0.9.3+). Asymmetry is documented and
  not gated.

### Limitations (Windows path)

- No source file / line on Windows yet. PDB exposes line info
  via `module.line_program()` but threading it through the
  index is non-trivial; deferred to a later release.
- No inlined-frame expansion on Windows. PDB `S_INLINESITE`
  records are not yet decoded.
- Best-effort address-to-RVA translation: without the module's
  load base we mask `address` to 32 bits and binary-search the
  RVA index. Exact for non-ASLR builds; usable but approximate
  for ASLR builds.
- C++ frames remain mangled (only Rust mangling is decoded via
  `rustc-demangle`). Adding `cpp_demangle` is a follow-up if
  there's real demand.

## [0.9.1] - 2026-05-14

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
- **Walker reads are no longer `read_volatile`.** The walker reads
  the current thread's own stack memory after the four
  bounds/alignment/range/monotonicity checks; no other thread can
  mutate that memory mid-walk. Plain reads let the compiler
  schedule the loads and unblock register-allocation
  opportunities. Same observable behaviour, with a small per-walk
  speed-up.
- **Table matching path no longer spins on `wait_published`.** The
  init thread now uses `fetch_add` (instead of `store`) for the
  bucket's `count` and `total_bytes` fields. Concurrent matching
  writers can land their increments at any moment without being
  clobbered, so the matching path becomes two `fetch_add` calls
  with no spin loop. Readers (`call_sites_report`) still gate on
  `frame_count > 0` for sample-frame coherence.
- CI: ASAN job sets `ASAN_OPTIONS=detect_stack_use_after_return=0`
  so the stack-bounds test runs against the real stack rather
  than ASAN's fake-stack heap allocation. The walker is
  unaffected either way; the test just needed a real-stack
  context to assert against.
- CI: added a defensive "Verify toolchain is fully installed"
  step after `dtolnay/rust-toolchain@stable` to heal the
  occasional macOS runner-image case where `cargo` resolves to
  `rustup-init`.

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

[Unreleased]: https://github.com/jamesgober/mod-alloc/compare/v0.9.5...HEAD
[0.9.4]: https://github.com/jamesgober/mod-alloc/compare/v0.9.3...v0.9.4
[0.9.3]: https://github.com/jamesgober/mod-alloc/compare/v0.9.2...v0.9.3
[0.9.2]: https://github.com/jamesgober/mod-alloc/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/jamesgober/mod-alloc/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/jamesgober/mod-alloc/compare/v0.1.0...v0.9.0
[0.1.0]: https://github.com/jamesgober/mod-alloc/releases/tag/v0.1.0
