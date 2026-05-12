# mod-alloc — Project Specification (REPS)

> Rust Engineering Project Specification.
> Normative language follows RFC 2119.

## 1. Purpose

`mod-alloc` MUST provide a global-allocator wrapper that tracks
allocations and deallocations. Output MUST be machine-readable so
consumers (`dev-bench`, manual benchmarks, CI gates) can act on the
measurements programmatically.

## 2. Core capabilities

### Tier 1: Counters (feature `counters`, default)

- Total allocation count
- Total bytes allocated
- Peak resident bytes
- Current resident bytes (allocations minus deallocations)

MUST be lock-free in the hot path. Per-thread state with periodic
aggregation to global atomics. Target overhead: <50ns per allocation
on x86_64.

### Tier 2: Call-site backtraces (feature `backtraces`, opt-in)

- 4-8 frames per allocation
- Frame-pointer-based capture on x86_64 and aarch64 initially
- No symbolication at capture time (raw addresses only)
- Symbolication deferred to report-generation time
- Per-call-site aggregation in the report

MUST NOT pull in the `backtrace` crate or any external symbolication
library. Inline capture via `extern "C"` or assembly intrinsics.

### Tier 3: DHAT-compatible output (feature `dhat-compat`)

- Emit JSON in the format consumed by the official DHAT viewer.
- Allows existing DHAT tooling to work with our output.
- Spec: https://valgrind.org/docs/manual/dh-manual.html

## 3. API surface

```rust
pub struct ModAlloc { /* private */ }
impl ModAlloc {
    pub const fn new() -> Self;
    pub fn snapshot(&self) -> AllocStats;
    pub fn reset(&self);
}

unsafe impl GlobalAlloc for ModAlloc { /* ... */ }

pub struct AllocStats {
    pub alloc_count: u64,
    pub total_bytes: u64,
    pub peak_bytes: u64,
    pub current_bytes: u64,
}

pub struct Profiler { /* private */ }
impl Profiler {
    pub fn start() -> Self;
    pub fn stop(self) -> AllocStats;
}
```

## 4. Recursion safety

The allocator hook MUST NOT itself allocate. All scratch storage
comes from pre-allocated buffers or direct `mmap`. Reentrancy
detection via thread-local flag.

## 5. Determinism

Given identical inputs and identical program state, allocation
counts MUST be deterministic across runs. (Backtrace addresses
WILL vary across runs due to ASLR; this is acceptable.)

## 6. Performance targets

- Tier 1 overhead: <50ns per allocation on x86_64
- Tier 2 overhead: <200ns per allocation with backtraces
- Profiler::start/stop: <1μs

## 7. Dependencies

MUST NOT have runtime dependencies outside `std`. Platform syscalls
(mmap, etc.) declared inline as `extern "C"`.

## 8. Stability

Through `0.9.x` the public API MAY shift. The `1.0` release pins the
API and the wire format of DHAT-compatible output.

## 9. Out of scope

- Replacing the system allocator (use `mimalloc` or `jemallocator`).
- Memory leak detection beyond peak/current tracking (use AddressSanitizer).
- Non-x86_64/non-aarch64 architectures in initial release (add by demand).
- Allocation source tracking via Rust source-level instrumentation
  (cargo features etc.) — too invasive.
