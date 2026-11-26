//! A folder dropped on the page, or chosen, read once (`docs/architecture.md`, 11.5). The page does not judge it: it
//! gives the session the tree that a walk of `engine` would see, its folders and its program files, and the walk of
//! `engine` judges it, with its bounds and its diagnostics (`LC0010`), as on the desktop; and the settings of the
//! project, its `.labcraft/config.toml`, [`MAX_CONFIG`] at most. The page only bounds what it lists and what it reads:
//! no more than that walk can reach, and [`MAX_READ`] in all; and it refuses a folder that a zip could not hold
//! ([`too_many`]).

use std::path::Path;

use labcraft_engine::walk::{BOUNDS, has_program_extension};
use labcraft_types::Limits;

/// What the page reads of program files in all *(proposed)*: the page keeps them for a restart and holds them about
/// three times while it sends them to a worker, which holds them about twice while it takes them; and reads them again
/// at a restart.
pub const MAX_READ: u64 = 64 * 1024 * 1024;

/// The settings of the project, which the page reads with its program files (ide.md 12).
pub const CONFIG: &str = ".labcraft/config.toml";
/// The most bytes of [`CONFIG`] that the page takes: a larger one is left out, and said.
pub const MAX_CONFIG: u64 = 1024 * 1024;

/// The files and folders that the page lists of a folder dropped at most: a walk of `engine` takes no more than
/// [`BOUNDS`] of each, and the rest is never looked at.
pub const MAX_LISTED: usize = 4 * BOUNDS.files;

/// Whether the page lists what a folder at this path holds, relative to the one given, with `/`: a hidden folder is
/// left out as a walk of `engine` leaves it; one deeper than that walk goes is given empty, so that it says so.
pub fn enters(path: &str) -> bool {
    !path.split('/').any(hidden) && path.split('/').count() < BOUNDS.levels
}

/// Whether the page lists what a folder of a drop holds: one that [`enters`] says, and `.labcraft`, for [`CONFIG`].
pub fn lists(path: &str) -> bool {
    enters(path) || Some(path) == CONFIG.split('/').next()
}

/// What the page gives the session of a folder: the files to read, by their index in the list given, and the folders
/// that no file read makes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub read: Vec<usize>,
    pub folders: Vec<String>,
    /// [`CONFIG`], when the folder has it.
    pub config: Option<usize>,
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
    let config = files.iter().position(|path| path == CONFIG);
    Plan { read, folders: given, config }
}

/// Why the page refuses what was dropped or chosen, when it holds no program file to read and a zip: the page opens a
/// folder, not a zip, which the user unzips first.
pub fn only_a_zip(files: &[String], plan: &Plan) -> Option<String> {
    if !plan.read.is_empty() {
        return None;
    }
    let zip = files.iter().find(|path| Path::new(path).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip")))?;
    Some(format!("`{zip}` is a zip, and the web page opens a folder only: unzip it, then drop the folder or choose it with Open folder"))
}

/// Why the page leaves out [`CONFIG`] of `size` bytes, if it does: over [`MAX_CONFIG`].
pub fn config_too_large(size: u64) -> Option<String> {
    (size > MAX_CONFIG).then(|| format!("`{CONFIG}` holds {size} bytes, over {} MiB: its settings are left out", MAX_CONFIG >> 20))
}

/// The entries of the zip of what the page holds, as [`archive`](crate::archive) counts them: the files, and the folders
/// that hold none of the files and folders given.
pub fn entries(files: &[String], folders: &[String]) -> usize {
    // Every folder above a path given holds something.
    let mut holding = std::collections::HashSet::new();
    for path in files.iter().chain(folders) {
        let mut at = path.as_str();
        while let Some((above, _)) = at.rsplit_once('/') {
            if !holding.insert(above) {
                break;
            }
            at = above;
        }
    }
    files.len() + folders.iter().filter(|folder| !holding.contains(folder.as_str())).count()
}

/// Why the page refuses a folder that a zip without zip64 could not hold, which `Download` could then not save: what
/// the desktop app opens in place.
pub fn too_many(files: &[String], folders: &[String]) -> Option<String> {
    let count = entries(files, folders);
    (count > crate::archive::MAX_ENTRIES).then(|| {
        format!(
            "This folder has {count} entries; the web page holds at most {}, which a zip can hold. Open it in the desktop app.",
            crate::archive::MAX_ENTRIES
        )
    })
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

    /// The settings of the project are read, those of a folder inside it are not; `.labcraft` is listed for them.
    #[test]
    fn the_settings_of_the_project_are_read_beside_its_program_files() {
        let files = paths(&["a.lcf", "sub/.labcraft/config.toml", ".labcraft/config.toml", ".labcraft/journal/1.jsonl"]);
        let plan = plan(&files, &paths(&[".labcraft"]));
        assert_eq!((plan.read.as_slice(), plan.config), ([0].as_slice(), Some(2)));
        assert!(plan.folders.is_empty(), "a hidden folder is not given");
        assert_eq!(super::plan(&paths(&["a.lcf"]), &[]).config, None);
        assert!(lists(".labcraft") && !lists(".labcraft/journal") && !lists(".git"));
        assert_eq!(config_too_large(MAX_CONFIG), None);
        assert!(config_too_large(MAX_CONFIG + 1).is_some_and(|why| why.contains("1 MiB") && why.contains(CONFIG)));
    }

    /// A zip alone is refused with what to do; a zip beside program files is not the matter of the page.
    #[test]
    fn a_zip_dropped_alone_is_refused_with_what_to_do() {
        let zip = paths(&["examples.ZIP"]);
        let refused = only_a_zip(&zip, &plan(&zip, &[])).unwrap();
        assert!(refused.contains("`examples.ZIP`") && refused.contains("unzip it"), "{refused}");
        let beside = paths(&["a.lcf", "old.zip"]);
        assert_eq!(only_a_zip(&beside, &plan(&beside, &[])), None);
        let nothing = paths(&["notes.txt"]);
        assert_eq!(only_a_zip(&nothing, &plan(&nothing, &[])), None, "the walk of `engine` says there is nothing to check");
    }

    /// The entries of a zip: each file, and each folder that holds nothing given; past the limit of a zip, refused.
    #[test]
    fn a_folder_past_the_entries_of_a_zip_is_refused() {
        assert_eq!(entries(&paths(&["a.lcf", "sub/b.lcf"]), &paths(&["sub", "empty", "deep", "deep/er"])), 4);
        let at_most = crate::archive::MAX_ENTRIES;
        let files: Vec<String> = (0..at_most - 1).map(|index| format!("f{index}.lcf")).collect();
        assert_eq!(too_many(&files, &paths(&["empty"])), None, "at the limit");
        let refused = too_many(&files, &paths(&["empty", "other"])).unwrap();
        assert!(refused.contains(&format!("{} entries", at_most + 1)) && refused.contains("desktop app"), "{refused}");
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
