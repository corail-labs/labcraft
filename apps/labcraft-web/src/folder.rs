//! A folder dropped on the page, or chosen, read once (`docs/architecture.md`, 11.5): walked with the bounds of a walk of
//! `engine` (32 levels, 100,000 files, hidden folders left out), and only its program files read, each at most one byte
//! past the largest file that `check` reads, so that a larger one is still refused by its size. The browser walks; this
//! decides.

use std::path::Path;

use labcraft_engine::walk::{BOUNDS, has_program_extension};
use labcraft_types::Limits;

/// What the page reads of program files in all *(proposed)*: the page keeps them for a restart, and holds them about
/// three times while it sends them to a worker, which holds them about twice while it takes them.
pub const MAX_READ: u64 = 256 * 1024 * 1024;

/// Whether the walk goes into a folder at this path, relative to the one given, with `/`: neither hidden nor too deep.
pub fn enters(path: &str) -> bool {
    path.split('/').count() <= BOUNDS.levels && !path.split('/').any(hidden)
}

/// What a file is to the walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken {
    /// A program file: [`Walk::read`] says how much of it to read.
    Program,
    /// Another file, which the walk counts and does not read.
    Other,
    /// Under a hidden folder: the walk never sees it.
    Left,
}

/// What the page reads of the files of a folder, in the order the browser lists them, and what it must say.
#[derive(Debug, Default)]
pub struct Walk {
    files: usize,
    read: u64,
    stopped: Option<String>,
}

impl Walk {
    pub fn new() -> Walk {
        Walk::default()
    }

    /// A file at `path`, relative to the folder given, with `/`; `None` once the walk stopped at a limit, after which
    /// nothing is taken.
    pub fn file(&mut self, path: &str) -> Option<Taken> {
        if self.stopped.is_some() {
            return None;
        }
        let mut folders = path.split('/').rev().skip(1);
        if folders.any(hidden) {
            return Some(Taken::Left);
        }
        if path.split('/').count() > BOUNDS.levels + 1 {
            self.stopped = Some(format!("the folder is more than {} levels deep: the viewer reads no further", BOUNDS.levels));
            return None;
        }
        self.files += 1;
        if self.files > BOUNDS.files {
            self.stopped = Some(format!("the folder holds more than {} files: the viewer reads no further", BOUNDS.files));
            return None;
        }
        Some(if has_program_extension(Path::new(path)) { Taken::Program } else { Taken::Other })
    }

    /// How much of a program file of `size` bytes to read; `None` when the files read would pass [`MAX_READ`], which
    /// stops the walk.
    pub fn read(&mut self, size: u64) -> Option<u64> {
        let taken = size.min(Limits::default().max_file_size.saturating_add(1));
        if self.stopped.is_some() || self.read.saturating_add(taken) > MAX_READ {
            self.stopped = Some(format!("the program files of the folder pass {} MiB: the viewer reads no further", MAX_READ >> 20));
            return None;
        }
        self.read += taken;
        Some(taken)
    }

    /// Why the walk stopped, if it did.
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }
}

/// A hidden name, as a walk of `engine` leaves out a folder that has one.
fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    #[test]
    fn program_files_are_read_and_hidden_folders_are_left_as_engine_leaves_them() {
        let mut walk = Walk::new();
        assert_eq!(walk.file("a.lcf"), Some(Taken::Program));
        assert_eq!(walk.file("sub/T.lct"), Some(Taken::Program));
        assert_eq!(walk.file("notes.txt"), Some(Taken::Other));
        assert_eq!(walk.file(".git/x.lcf"), Some(Taken::Left));
        assert_eq!(walk.file("sub/.cache/x.lcf"), Some(Taken::Left));
        // A hidden file is read, as a walk of `engine` reads it.
        assert_eq!(walk.file("sub/.hidden.lcf"), Some(Taken::Program));
        assert!(!enters(".git") && !enters("sub/.cache") && enters("sub"));
        assert_eq!(walk.read(10), Some(10));
        assert_eq!(walk.stopped(), None);
    }

    #[test]
    fn a_file_too_large_is_read_one_byte_past_the_limit() {
        let max = Limits::default().max_file_size;
        assert_eq!(Walk::new().read(max * 10), Some(max + 1));
    }

    #[test]
    fn the_walk_stops_at_its_limits() {
        let mut walk = Walk::new();
        for index in 0..BOUNDS.files {
            assert!(walk.file(&format!("f{index}.txt")).is_some());
        }
        assert_eq!(walk.file("one.lcf"), None);
        assert!(walk.stopped().is_some_and(|why| why.contains("100000")));
        assert_eq!(walk.file("two.lcf"), None);

        let deep = vec!["d"; BOUNDS.levels].join("/");
        assert!(Walk::new().file(&format!("{deep}/f.lcf")).is_some());
        let deeper = vec!["d"; BOUNDS.levels + 1].join("/");
        assert_eq!(Walk::new().file(&format!("{deeper}/f.lcf")), None);
        assert!(!enters(&deeper));

        let mut walk = Walk::new();
        let max = Limits::default().max_file_size;
        let mut count = 0;
        while walk.read(max).is_some() {
            count += 1;
        }
        assert_eq!(count, MAX_READ / max);
        assert!(walk.stopped().is_some_and(|why| why.contains("256 MiB")));
        assert_eq!(walk.file("after.lcf"), None);
    }
}
