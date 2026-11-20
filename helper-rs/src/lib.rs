//! 7TV Emotes Helper: fetches 7TV emotes for the SevenTVEmotes World of Warcraft addon.
//!
//! Rust port of the original Python helper (v0.5.1, in the git history). Same files, same formats,
//! same helper_state.json schema, so it can take over an existing install.

// Shared with the web pack builder (compiles without the "native" feature, including to WebAssembly).
pub mod bundle;
pub mod images;
pub mod luasv;
pub mod pack;
pub mod seventv;

// The Windows helper app.
#[cfg(feature = "native")]
pub mod draw;
#[cfg(feature = "native")]
pub mod installer;
#[cfg(feature = "native")]
pub mod instance;
#[cfg(feature = "native")]
pub mod library;
#[cfg(feature = "native")]
pub mod logger;
#[cfg(feature = "native")]
pub mod paths;
#[cfg(feature = "native")]
pub mod service;
#[cfg(feature = "native")]
pub mod status_board;
#[cfg(feature = "native")]
pub mod updater;
#[cfg(feature = "native")]
pub mod wow;

#[cfg(all(windows, feature = "native"))]
pub mod tray;
#[cfg(all(windows, feature = "native"))]
pub mod win;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Seconds since the Unix epoch (Python's time.time()). Not available in WebAssembly builds.
#[cfg(feature = "native")]
pub fn now() -> f64 {
    system_time_secs(std::time::SystemTime::now())
}

pub fn system_time_secs(t: std::time::SystemTime) -> f64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

#[cfg(feature = "native")]
#[doc(hidden)]
pub mod testutil {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A temporary directory removed on drop (tests only; avoids a dependency).
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "seventv-test-{}-{}-{}",
                tag,
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
