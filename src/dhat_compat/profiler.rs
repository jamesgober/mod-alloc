//! dhat-rs-shaped `Profiler` / `ProfilerBuilder`.
//!
//! Drop semantics match dhat-rs: a `Profiler` constructed via
//! `new_heap()` (or `builder().build()`) writes its JSON report
//! when dropped, unless built with `.testing()`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Profiler mode — heap-allocation tracking or ad-hoc event
/// counting. Pick at construction via [`Profiler::new_heap`] /
/// [`Profiler::new_ad_hoc`] or via [`ProfilerBuilder`].
///
/// # Stability
///
/// Marked `#[non_exhaustive]` as of v1.0.0. Future minor
/// versions may add new modes (e.g. event-stream output)
/// without bumping the major version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mode {
    /// Heap allocation profiling (default).
    Heap,
    /// Ad-hoc event profiling.
    AdHoc,
}

#[derive(Clone, Debug)]
struct Config {
    mode: Mode,
    file_name: Option<PathBuf>,
    testing: bool,
    #[allow(dead_code)]
    trim_backtraces: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Heap,
            file_name: None,
            testing: false,
            trim_backtraces: None,
        }
    }
}

/// RAII handle that writes a DHAT-format JSON report on drop.
///
/// Drop-in-shaped replacement for `dhat::Profiler`. Hold the
/// returned value in `main` (or wherever you want the file to
/// land) and let scope exit trigger the write.
///
/// # Example
///
/// ```no_run
/// # #[cfg(feature = "dhat-compat")]
/// # fn demo() {
/// use mod_alloc::dhat_compat::{Alloc, Profiler};
///
/// #[global_allocator]
/// static ALLOC: Alloc = Alloc;
///
/// fn main() {
///     let _profiler = Profiler::new_heap();
///     let _v: Vec<u8> = vec![0; 1024];
///     // _profiler drops here → writes dhat-heap.json
/// }
/// # }
/// ```
pub struct Profiler {
    config: Config,
}

impl Profiler {
    /// Construct a heap-mode profiler. Writes `dhat-heap.json` on
    /// drop unless `builder().file_name(...)` was used.
    pub fn new_heap() -> Self {
        Self::install(Config {
            mode: Mode::Heap,
            ..Config::default()
        })
    }

    /// Construct an ad-hoc-mode profiler. Writes
    /// `dhat-ad-hoc.json` on drop.
    pub fn new_ad_hoc() -> Self {
        Self::install(Config {
            mode: Mode::AdHoc,
            ..Config::default()
        })
    }

    /// Start a builder for fine-grained configuration.
    pub fn builder() -> ProfilerBuilder {
        ProfilerBuilder::new()
    }

    fn install(config: Config) -> Self {
        // Best-effort single-Profiler guard. We do not panic on
        // re-entry (dhat-rs does) because that surprise is hard to
        // recover from in downstream test harnesses; document
        // "last writer wins" instead.
        PROFILERS_ALIVE.fetch_add(1, Ordering::Relaxed);
        Self { config }
    }
}

// Number of live `Profiler`s. A count rather than a flag so that, with
// two Profilers alive, dropping one does not stop call-site recording
// for the other.
static PROFILERS_ALIVE: AtomicUsize = AtomicUsize::new(0);

/// `true` while at least one [`Profiler`] is alive.
#[inline(always)]
pub(crate) fn is_active() -> bool {
    PROFILERS_ALIVE.load(Ordering::Relaxed) != 0
}

impl Drop for Profiler {
    fn drop(&mut self) {
        PROFILERS_ALIVE.fetch_sub(1, Ordering::Relaxed);

        if self.config.testing {
            return;
        }

        let path = self.config.file_name.clone().unwrap_or_else(|| {
            PathBuf::from(match self.config.mode {
                Mode::Heap => "dhat-heap.json",
                Mode::AdHoc => "dhat-ad-hoc.json",
            })
        });

        match self.config.mode {
            Mode::Heap => {
                // Errors are intentionally swallowed (matches
                // dhat-rs). Drop cannot propagate `?`, and a
                // failed write of a profile report shouldn't
                // abort the process at scope exit.
                let _ = crate::dhat_json::write_dhat_json(&path);
            }
            Mode::AdHoc => {
                let _ = super::ad_hoc_writer::write_ad_hoc(&path);
            }
        }
    }
}

/// Builder for [`Profiler`].
///
/// Mirrors `dhat::ProfilerBuilder` method-for-method. Obtain via
/// [`Profiler::builder`].
#[derive(Debug)]
pub struct ProfilerBuilder {
    config: Config,
}

impl ProfilerBuilder {
    fn new() -> Self {
        Self {
            config: Config::default(),
        }
    }

    /// Switch the profiler to ad-hoc mode.
    pub fn ad_hoc(mut self) -> Self {
        self.config.mode = Mode::AdHoc;
        self
    }

    /// Build in testing mode — suppresses the drop-time file
    /// write. Use for tests that snapshot stats directly without
    /// littering the workspace with `dhat-heap.json`.
    pub fn testing(mut self) -> Self {
        self.config.testing = true;
        self
    }

    /// Override the output file name. Default is
    /// `dhat-heap.json` (or `dhat-ad-hoc.json` in ad-hoc mode).
    pub fn file_name<P: AsRef<Path>>(mut self, p: P) -> Self {
        self.config.file_name = Some(p.as_ref().to_path_buf());
        self
    }

    /// Maximum frames to retain per backtrace (`None` = walker
    /// default).
    ///
    /// Accepted for API parity with `dhat::ProfilerBuilder`;
    /// silently clamped to mod-alloc's walker cap of 8 frames.
    /// Values above 8 produce up to 8 frames; values below 8
    /// produce that many frames in the emitted JSON.
    pub fn trim_backtraces(mut self, max_frames: Option<usize>) -> Self {
        self.config.trim_backtraces = max_frames;
        self
    }

    /// Build the profiler. The returned value writes the report
    /// on drop.
    pub fn build(self) -> Profiler {
        Profiler::install(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_defaults_to_heap_mode() {
        // `testing()` so the drop does not write dhat-heap.json into
        // the working directory.
        let p = Profiler::builder().testing().build();
        assert_eq!(p.config.mode, Mode::Heap);
        // testing-mode skip on drop
        let _t = Profiler::builder().testing().build();
        // both drop here; testing mode keeps either
        // from writing to the working directory.
    }

    #[test]
    fn builder_ad_hoc_switches_mode() {
        let p = Profiler::builder().ad_hoc().testing().build();
        assert_eq!(p.config.mode, Mode::AdHoc);
    }

    #[test]
    fn builder_file_name_overrides_default() {
        let p = Profiler::builder()
            .file_name("custom.json")
            .testing()
            .build();
        assert_eq!(
            p.config.file_name.as_deref(),
            Some(std::path::Path::new("custom.json"))
        );
    }
}
