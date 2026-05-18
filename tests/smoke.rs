use mod_alloc::{AllocStats, ModAlloc, Profiler};

#[test]
fn smoke_alloc_constructs() {
    let _ = ModAlloc::new();
}

#[test]
fn smoke_snapshot_initial_zeros() {
    let a = ModAlloc::new();
    let s = a.snapshot();
    assert_eq!(s.alloc_count, 0);
    assert_eq!(s.total_bytes, 0);
    assert_eq!(s.peak_bytes, 0);
    assert_eq!(s.current_bytes, 0);
}

#[test]
fn smoke_reset() {
    let a = ModAlloc::new();
    a.reset();
    let s = a.snapshot();
    assert_eq!(s.alloc_count, 0);
}

#[test]
fn smoke_profiler_round_trip() {
    let p = Profiler::start();
    let stats: AllocStats = p.stop();
    assert_eq!(stats.alloc_count, 0);
}

#[test]
fn smoke_stats_copy_and_eq() {
    // `AllocStats` is `#[non_exhaustive]` as of v1.0.0; construct
    // via `Default` and mutate named fields. This is the supported
    // API surface — direct snapshot consumers get their stats from
    // `ModAlloc::snapshot()` or `Profiler::stop()`.
    let mut s = AllocStats::default();
    s.alloc_count = 5;
    s.total_bytes = 100;
    s.peak_bytes = 80;
    s.current_bytes = 40;
    s.live_count = 2;
    s.peak_live_count = 3;

    let t = s;
    assert_eq!(s, t);
    assert_eq!(s.alloc_count, 5);
    assert_eq!(s.peak_live_count, 3);
}
