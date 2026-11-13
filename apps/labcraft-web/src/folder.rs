//! A folder dropped on the page, or chosen, read once (`docs/architecture.md`, 11.5). The page does not judge it: it
//! gives the session the tree that a walk of `engine` would see, its folders and its program files, and the walk of
//! `engine` judges it, with its bounds and its diagnostics (`LC0010`), as on the desktop. The page only bounds what it
//! lists and what it reads: no more than that walk can reach, and [`MAX_READ`] in all.

use std::path::Path;

use labcraft_engine::walk::{BOUNDS, has_program_extension};
use labcraft_types::Limits;

/// What the page reads of program files in all *(proposed)*: the page keeps them for a restart and holds them about
/// three times while it sends them to a worker, which holds them about twice while it takes them; and reads them again
/// at a restart.
pub const MAX_READ: u64 = 64 * 1024 * 1024;

/// The files and folders that the page lists of a folder dropped at most: a walk of `engine` takes no more than
/// [`BOUNDS`] of each, and the rest is never looked at.
pub const MAX_LISTED: usize = 4 * BOUNDS.files;

/// Whether the page lists what a folder at this path holds, relative to the one given, with `/`: a hidden folder is
/// left out as a walk of `engine` leaves it; one deeper than that walk goes is given empty, so that it says so.
pub fn enters(path: &str) -> bool {
    !path.split('/').any(hidden) && path.split('/').count() < BOUNDS.levels
}

/// What the page gives the session of a folder: the files to read, by their index in the list given, and the folders
/// that no file read makes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub read: Vec<usize>,
    pub folders: Vec<String>,
}

/// Of the files and folders listed, paths relative to the folder given with `/`: the program files that a walk of
/// `engine` reaches, the first in the order of their paths and one more, so that the walk says it left some out; and the
/// folders, a folder deeper than the walk goes given empty at the first depth it does not enter, and a file deeper than
/// that only by its folder at that depth, so that the walk says it left it out.
pub fn plan(files: &[String], folders: &[String]) -> Plan {
    // The folders above a file, or a folder and those above it: a walk of `engine` goes into none that is hidden, and
    // reads the files of the folders less than `BOUNDS.levels` deep.
    let reached = |folders: &[&str]| !folders.iter().any(|name| hidden(name));
    let mut read: Vec<usize> = (0..files.len())
        .filter(|&index| {
            files.get(index).is_some_and(|path| {
                let folders = above(path);
                reached(&folders) && folders.len() < BOUNDS.levels && has_program_extension(Path::new(path))
            })
        })
        .collect();
    read.sort_by(|a, b| files.get(*a).cmp(&files.get(*b)));
    read.truncate(BOUNDS.files + 1);
    let deepest = |path: &str| path.split('/').take(BOUNDS.levels).collect::<Vec<_>>().join("/");
    let mut given: Vec<String> =
        folders.iter().filter(|folder| reached(&folder.split('/').collect::<Vec<_>>())).map(|folder| deepest(folder)).collect();
    given.extend(
        files
            .iter()
            .map(|path| above(path))
            .filter(|folders| reached(folders) && folders.len() >= BOUNDS.levels)
            .map(|folders| deepest(&folders.join("/"))),
    );
    given.sort();
    given.dedup();
    given.truncate(BOUNDS.files + 1);
    Plan { read, folders: given }
}

/// What the page reads of the program files of a folder: each at most one byte past the largest file that `check`
/// reads, so that a larger one is still refused by its size, and [`MAX_READ`] in all.
#[derive(Debug, Default)]
pub struct Budget {
    read: u64,
    spent: bool,
}

impl Budget {
    /// How much of a file of `size` bytes to read; `None` once the budget is spent, after which nothing is read.
    pub fn take(&mut self, size: u64) -> Option<u64> {
        let taken = size.min(Limits::default().max_file_size.saturating_add(1));
        if self.spent || self.read.saturating_add(taken) > MAX_READ {
            self.spent = true;
            return None;
        }
        self.read += taken;
        Some(taken)
    }

    /// What the page must say, if it stopped reading.
    pub fn notice(&self) -> Option<String> {
        self.spent.then(|| format!("the program files of the folder pass {} MiB: the viewer read the first ones only", MAX_READ >> 20))
    }
}

/// The folders above a file, from the folder given.
fn above(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts
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

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn program_files_are_read_and_hidden_folders_are_left_as_engine_leaves_them() {
        let files = paths(&["b.lcf", "notes.txt", ".git/x.lcf", "sub/.cache/x.lcf", "sub/.hidden.lcf", "sub/T.lct", "a.lcf"]);
        let plan = plan(&files, &paths(&["sub", ".git", "empty"]));
        // In the order of their paths; a hidden file is read, as a walk of `engine` reads it.
        assert_eq!(plan.read, [6, 0, 4, 5]);
        assert_eq!(plan.folders, ["empty", "sub"]);
        assert!(!enters(".git") && !enters("sub/.cache") && enters("sub"));
    }

    #[test]
    fn what_is_deeper_than_the_walk_is_given_by_its_folder_at_that_depth() {
        let at = |levels: usize| vec!["d"; levels].join("/");
        let files =
            vec![format!("{}/f.lcf", at(BOUNDS.levels - 1)), format!("{}/g.lcf", at(BOUNDS.levels)), format!("{}/h.lcf", at(BOUNDS.levels + 3))];
        let plan = plan(&files, &[at(BOUNDS.levels + 2)]);
        assert_eq!(plan.read, [0]);
        assert_eq!(plan.folders, [at(BOUNDS.levels)]);
        assert!(enters(&at(BOUNDS.levels - 1)) && !enters(&at(BOUNDS.levels)));
    }

    #[test]
    fn one_file_more_than_the_walk_takes_is_read_so_that_it_says_so() {
        let files: Vec<String> = (0..BOUNDS.files + 10).map(|index| format!("f{index:06}.lcf")).collect();
        let plan = plan(&files, &[]);
        assert_eq!(plan.read.len(), BOUNDS.files + 1);
        assert_eq!(plan.read.first(), Some(&0));
    }

    #[test]
    fn a_file_too_large_is_read_one_byte_past_the_limit_and_the_budget_stops_the_reading() {
        let max = Limits::default().max_file_size;
        assert_eq!(Budget::default().take(max * 10), Some(max + 1));
        let mut budget = Budget::default();
        let mut count = 0;
        while budget.take(max).is_some() {
            count += 1;
        }
        assert_eq!(count, MAX_READ / max);
        assert!(budget.notice().is_some_and(|why| why.contains("64 MiB")));
        assert_eq!(budget.take(1), None, "nothing more once spent");
    }
}
