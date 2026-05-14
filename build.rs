//! mod-alloc build script.
//!
//! Approved exception per `.dev/DIRECTIVES.md` section 2.1. The
//! crate does not generally use `build.rs`. This script exists
//! solely to detect whether the user's toolchain has frame
//! pointers enabled when the `backtraces` feature is on, and to
//! emit a `cargo:warning=` directive if not.
//!
//! Frame pointers are a prerequisite for the inline FP walker
//! shipped in `v0.9.1`. Without them the walker returns empty or
//! shallow traces on release builds; a build-time hint saves
//! users from debugging that downstream.
//!
//! This script never fails the build. The walker degrades
//! gracefully at runtime if FP support is absent.

use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");

    let backtraces_on = env::var_os("CARGO_FEATURE_BACKTRACES").is_some();
    if !backtraces_on {
        return;
    }

    let rustflags = env::var("CARGO_ENCODED_RUSTFLAGS")
        .or_else(|_| env::var("RUSTFLAGS"))
        .unwrap_or_default();

    let has_fp = rustflags.contains("force-frame-pointers=yes")
        || rustflags.contains("force-frame-pointers=y")
        || rustflags.contains("force-frame-pointers=on");

    if !has_fp {
        println!(
            "cargo:warning=mod-alloc: the `backtraces` feature is enabled but \
             RUSTFLAGS does not include `-C force-frame-pointers=yes`. The inline \
             FP walker requires frame pointers; without them traces will be empty \
             or shallow on release builds. Add to .cargo/config.toml: [build] \
             rustflags = [\"-C\", \"force-frame-pointers=yes\"]"
        );
    }
}
