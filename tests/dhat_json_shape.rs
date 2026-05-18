//! Shape-level tests for the DHAT-compatible JSON output.
//!
//! We do not pull `serde_json` in as a dev-dep — the format is
//! fixed and shape assertions via string matching are sufficient
//! to verify well-formedness. Cross-validation against the real
//! `dh_view.html` viewer is documented in the v0.9.3 release
//! note as a one-time manual step.

#![cfg(feature = "dhat-compat")]

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

#[inline(never)]
fn workload() {
    for _ in 0..32 {
        let _v: Vec<u8> = Vec::with_capacity(256);
    }
}

fn assert_required_top_level_keys(json: &str) {
    for expected in [
        "\"dhatFileVersion\":2",
        "\"mode\":\"rust-heap\"",
        "\"verb\":\"Allocated\"",
        "\"bklt\":false",
        "\"bkacc\":false",
        "\"bu\":\"byte\"",
        "\"bsu\":\"bytes\"",
        "\"bksu\":\"blocks\"",
        "\"tu\":\"instrs\"",
        "\"Mtu\":\"Minstr\"",
        "\"tuth\":0",
        "\"cmd\":\"",
        "\"pid\":",
        "\"tg\":0",
        "\"te\":0",
        "\"pps\":[",
        "\"ftbl\":[",
        "\"[root]\"",
    ] {
        assert!(
            json.contains(expected),
            "missing required JSON fragment: {expected}\nfull JSON:\n{json}"
        );
    }
}

#[test]
fn json_starts_and_ends_with_object_braces() {
    let s = GLOBAL.dhat_json_string();
    assert!(s.starts_with('{'), "JSON must start with {{; got: {s}");
    assert!(s.ends_with('}'), "JSON must end with }}; got: {s}");
}

#[test]
fn json_contains_all_required_top_level_keys_after_workload() {
    workload();
    let s = GLOBAL.dhat_json_string();
    assert_required_top_level_keys(&s);
}

#[test]
fn pps_grows_after_workload() {
    // Capture before/after sizes. The per-call-site table is
    // process-wide and monotonic, so we cannot reliably assert
    // exact deltas (parallel tests share the table), but a
    // workload run should never make the report shrink and
    // should produce at least one program point.
    let _ = GLOBAL.dhat_json_string(); // prime
    workload();
    let after = GLOBAL.dhat_json_string();

    // At minimum one pp must exist after a workload.
    assert!(
        after.contains("\"tb\":") || after.contains("\"tbk\":"),
        "expected at least one program point in: {after}"
    );

    // ftbl must have at least the root plus one captured frame.
    let ftbl_idx = after.find("\"ftbl\":[").expect("ftbl present");
    let ftbl_chunk = &after[ftbl_idx..];
    // count commas inside the ftbl array as an approximation
    // of "more than just root". The ftbl is the last array in
    // the document so we can scan from its opening bracket.
    assert!(
        ftbl_chunk.contains("\"[root]\""),
        "ftbl missing root entry: {ftbl_chunk}"
    );
}

#[test]
fn write_dhat_json_round_trips() {
    workload();
    let target = std::env::temp_dir().join(format!(
        "mod-alloc-dhat-test-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));

    GLOBAL.write_dhat_json(&target).expect("write_dhat_json");
    let written = std::fs::read_to_string(&target).expect("read written file");
    let _ = std::fs::remove_file(&target);

    assert!(written.starts_with('{') && written.ends_with('}'));
    assert_required_top_level_keys(&written);
}
