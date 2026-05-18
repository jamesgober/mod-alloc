//! Minimal ad-hoc-mode JSON writer.
//!
//! Companion to `super::dhat_json` (heap mode). Emits a
//! `dhatFileVersion: 2`, `mode: "ad-hoc"` document with a single
//! program point carrying the accumulated event count and unit
//! sum from [`super::stats::ad_hoc_event`].
//!
//! Schema rationale: the upstream DHAT viewer renders ad-hoc
//! profiles using the same `pps`/`ftbl` infrastructure as heap
//! profiles, with `mode` driving column labels. Without call-site
//! capture for ad-hoc events (ad-hoc events are reported by user
//! code, not by the allocator hook), we emit a single rolled-up
//! program point with `fs: []`.

use std::io;
use std::path::Path;

use super::stats::AdHocStats;

pub(super) fn write_ad_hoc(path: &Path) -> io::Result<()> {
    let stats = AdHocStats::get();
    let body = render(&stats);
    std::fs::write(path, body)
}

fn render(stats: &AdHocStats) -> String {
    let mut out = String::with_capacity(256);
    let pid = std::process::id();

    out.push('{');
    out.push_str("\"dhatFileVersion\":2,");
    out.push_str("\"mode\":\"ad-hoc\",");
    out.push_str("\"verb\":\"Occurred\",");
    out.push_str("\"bklt\":false,");
    out.push_str("\"bkacc\":false,");
    out.push_str("\"bu\":\"unit\",");
    out.push_str("\"bsu\":\"units\",");
    out.push_str("\"bksu\":\"events\",");
    out.push_str("\"tu\":\"instrs\",");
    out.push_str("\"Mtu\":\"Minstr\",");
    out.push_str("\"tuth\":0,");

    out.push_str("\"cmd\":");
    push_cmd(&mut out);
    out.push(',');

    out.push_str("\"pid\":");
    push_u64(&mut out, pid as u64);
    out.push(',');

    out.push_str("\"tg\":0,");
    out.push_str("\"te\":0,");

    out.push_str("\"pps\":[{");
    out.push_str("\"tb\":");
    push_u64(&mut out, stats.total_units);
    out.push(',');
    out.push_str("\"tbk\":");
    push_u64(&mut out, stats.total_events);
    out.push(',');
    out.push_str("\"eb\":0,\"ebk\":0,");
    out.push_str("\"fs\":[]");
    out.push_str("}],");

    out.push_str("\"ftbl\":[\"[root]\"]");
    out.push('}');

    out
}

fn push_u64(out: &mut String, n: u64) {
    use std::fmt::Write as _;
    let _ = write!(out, "{n}");
}

fn push_cmd(out: &mut String) {
    out.push('"');
    let mut first = true;
    for arg in std::env::args_os() {
        if !first {
            out.push(' ');
        }
        first = false;
        let s = arg.to_string_lossy();
        for ch in s.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    use std::fmt::Write as _;
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
    }
    if first {
        out.push_str("<unknown>");
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_emits_required_keys() {
        let stats = AdHocStats {
            total_events: 3,
            total_units: 42,
        };
        let s = render(&stats);
        assert!(s.starts_with('{') && s.ends_with('}'));
        for fragment in [
            "\"dhatFileVersion\":2",
            "\"mode\":\"ad-hoc\"",
            "\"verb\":\"Occurred\"",
            "\"tb\":42",
            "\"tbk\":3",
            "\"[root]\"",
        ] {
            assert!(s.contains(fragment), "missing {fragment} in {s}");
        }
    }
}
