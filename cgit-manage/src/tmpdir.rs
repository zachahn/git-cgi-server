//! A throwaway directory for tests.
//!
//! The crate has no dependencies, and one small struct is cheaper than adding
//! a temp-directory package for it. Each directory takes a unique name from
//! the process id, a label, and a counter, so tests running in parallel never
//! share one. `Drop` removes the directory and everything under it.
//!
//! The integration test pulls this file in with `#[path]`, so the unit tests
//! and the end-to-end tests build one directory the same way.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Make an empty directory under the system temp directory.
    pub fn new(label: &str) -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("cgit-manage-test-{}-{}-{}", std::process::id(), label, n);
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the test directory");
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Build a path inside the directory, for example `d.join("demo.git/HEAD")`.
    pub fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.join(rel).exists()
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.join(rel)).expect("read a file in the test directory")
    }

    pub fn write(&self, rel: &str, contents: &str) {
        std::fs::write(self.join(rel), contents).expect("write a file in the test directory");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
