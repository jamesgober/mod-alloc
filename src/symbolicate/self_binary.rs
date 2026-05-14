//! Locate the running executable so its debug info can be read.
//!
//! Cached in a `OnceLock<Option<PathBuf>>` so each platform-
//! specific lookup runs at most once per process. Returns `None`
//! when the lookup fails; the symbolicator then degrades to
//! producing unresolved frames rather than panicking.

use std::path::PathBuf;
use std::sync::OnceLock;

static SELF_EXE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Path to the current executable. Resolved once and cached.
pub(crate) fn self_exe() -> Option<&'static PathBuf> {
    SELF_EXE.get_or_init(probe).as_ref()
}

fn probe() -> Option<PathBuf> {
    // `std::env::current_exe` already wraps the per-OS lookups
    // (readlink /proc/self/exe on Linux, _NSGetExecutablePath on
    // macOS, GetModuleFileNameW on Windows) and is available
    // under MSRV 1.75. We do not re-implement them here.
    std::env::current_exe().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_a_path_to_an_existing_file() {
        let p = self_exe().expect("self_exe should resolve");
        assert!(p.exists(), "self_exe path {p:?} should exist");
    }

    #[test]
    fn cached_across_calls() {
        let a = self_exe().cloned();
        let b = self_exe().cloned();
        assert_eq!(a, b);
    }
}
