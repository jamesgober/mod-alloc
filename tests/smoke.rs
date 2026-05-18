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
    let s = AllocStats {
        alloc_count: 5,
        total_bytes: 100,
        peak_bytes: 80,
        current_bytes: 40,
        live_count: 2,
        peak_live_count: 3,
    };
    let t = s;
    assert_eq!(s, t);
}
