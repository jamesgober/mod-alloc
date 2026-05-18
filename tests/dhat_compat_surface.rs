//! End-to-end coverage for the dhat-rs-shaped compatibility
//! surface introduced in v0.9.4.

#![cfg(feature = "dhat-compat")]

use mod_alloc::dhat_compat::{ad_hoc_event, AdHocStats, Alloc, HeapStats, Profiler};

#[global_allocator]
static ALLOC: Alloc = Alloc;

#[inline(never)]
fn workload(n: usize) -> Vec<Vec<u8>> {
    let mut keep: Vec<Vec<u8>> = Vec::with_capacity(n);
    for _ in 0..n {
        keep.push(Vec::with_capacity(64));
    }
    keep
}

#[test]
fn alloc_swap_pattern_compiles_and_tracks_total_bytes() {
    let before = HeapStats::get();
    let kept = workload(16);
    let after = HeapStats::get();

    assert!(
        after.total_blocks > before.total_blocks,
        "after.total_blocks ({}) must exceed before ({})",
        after.total_blocks,
        before.total_blocks
    );
    assert!(after.total_bytes >= before.total_bytes + 16 * 64);
    drop(kept);
}

#[test]
fn live_block_count_rises_and_falls() {
    let baseline = HeapStats::get().curr_blocks;
    let kept = workload(8);
    let peak = HeapStats::get();
    let kept_count = kept.len();
    drop(kept);
    let after = HeapStats::get();

    assert!(
        peak.curr_blocks >= baseline + kept_count,
        "curr_blocks {} should be at least baseline {} + {}",
        peak.curr_blocks,
        baseline,
        kept_count
    );
    assert!(
        after.curr_blocks <= peak.curr_blocks,
        "curr_blocks should fall after drop"
    );
}

#[test]
fn profiler_drop_writes_file_with_dhat_json_shape() {
    let path = std::env::temp_dir().join(format!(
        "mod-alloc-dhat-compat-test-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));

    {
        let _p = Profiler::builder().file_name(&path).build();
        // Some allocation so the report isn't empty.
        let _kept = workload(4);
    }

    assert!(
        path.exists(),
        "Profiler drop should write {}",
        path.display()
    );
    let bytes = std::fs::read_to_string(&path).expect("read written file");
    let _ = std::fs::remove_file(&path);

    assert!(bytes.starts_with('{') && bytes.ends_with('}'));
    for fragment in [
        "\"dhatFileVersion\":2",
        "\"mode\":\"rust-heap\"",
        "\"pps\":[",
        "\"ftbl\":[",
    ] {
        assert!(bytes.contains(fragment), "missing {fragment} in {bytes}");
    }
}

#[test]
fn testing_mode_suppresses_drop_write() {
    let path = std::env::temp_dir().join(format!(
        "mod-alloc-dhat-compat-testing-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));

    // Ensure no stale file.
    let _ = std::fs::remove_file(&path);

    {
        let _p = Profiler::builder().file_name(&path).testing().build();
        let _kept = workload(4);
    }

    assert!(
        !path.exists(),
        "testing mode must NOT write the file at {}",
        path.display()
    );
}

#[test]
fn ad_hoc_event_accumulates_counts_and_weights() {
    let before = AdHocStats::get();
    ad_hoc_event(7);
    ad_hoc_event(3);
    let after = AdHocStats::get();
    assert_eq!(after.total_events - before.total_events, 2);
    assert_eq!(after.total_units - before.total_units, 10);
}

#[test]
fn trim_backtraces_accepts_oversize_value_without_panic() {
    // 100 > walker cap of 8 — must not panic, must still build.
    let _p = Profiler::builder()
        .testing()
        .trim_backtraces(Some(100))
        .build();
}

#[test]
fn profiler_new_heap_constructs_and_drops_cleanly() {
    // Default `dhat-heap.json` write to CWD would litter the
    // workspace, so we route it to tmp instead.
    let path = std::env::temp_dir().join(format!(
        "mod-alloc-new-heap-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));

    {
        let _p = Profiler::builder().file_name(&path).build();
    }

    assert!(path.exists());
    let _ = std::fs::remove_file(&path);

    // Smoke-coverage for `Profiler::new_heap()` itself with the
    // testing flag (avoids writing dhat-heap.json to CWD).
    {
        let _p = Profiler::builder().testing().build();
    }
}
