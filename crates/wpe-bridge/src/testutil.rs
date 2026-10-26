//! Scratch directories for tests; no tempfile crate, so the lockfile stays small.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new() -> Scratch {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("wpe-bridge-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Writes `text` to `rel` under the scratch dir, creating parents.
    pub fn write(&self, rel: &str, text: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        p
    }

    /// Like `write`, and marks the file executable.
    pub fn exec(&self, rel: &str, text: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = self.write(rel, text);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
