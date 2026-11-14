//! What the tests of the viewer share: a folder of their own, and two programs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A folder of its own under the temporary folder, removed at the end.
pub struct Folder(pub PathBuf);

impl Folder {
    pub fn new(files: &[(&str, &str)]) -> Folder {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("labcraft-viewer-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        for (name, text) in files {
            std::fs::write(path.join(name), text).unwrap();
        }
        Folder(path.canonicalize().unwrap())
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub const ONE: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.add(x, 1.0)\n}\n";
pub const TWO: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    y = core.add(a, 1.0)\n}\n";
