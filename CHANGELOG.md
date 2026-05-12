# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
implemented — using `ModAlloc` as `#[global_allocator]` in 0.1.0
will not work. Real implementation lands in `0.9.x` along with:

- Per-thread arena-based tracking to avoid contention.
- Inline frame-pointer-based backtrace capture (x86_64 + aarch64).
- DHAT-compatible JSON output.
- Statistical validation suite.

[Unreleased]: https://github.com/jamesgober/mod-alloc/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jamesgober/mod-alloc/releases/tag/v0.1.0
