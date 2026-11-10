//! The walk of `engine` (`docs/cli.md`, section 1), on the disk: its tests are here, where the disk is.

pub use labcraft_engine::walk::*;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use labcraft_types::Limits;

    use super::*;
    use crate::disk::Disk;

    /// A folder of its own under the temporary folder of the system, removed at the end of the test.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("labcraft-walk-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Folder(fs::canonicalize(path).unwrap())
        }

        fn file(&self, relative: &str, bytes: &[u8]) -> &Self {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            self
        }

        fn walk(&self) -> Walked {
            walk(&Disk, std::slice::from_ref(&self.0), Some(&self.0), &Limits::default())
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn shown(path: &Path, current: Option<&Path>) -> String {
        shown_of(&Disk, path, fs::canonicalize(path).ok().as_deref(), current)
    }

    fn shown_files(walked: &Walked) -> Vec<&str> {
        walked.files.iter().map(|file| file.shown.as_str()).collect()
    }

    fn refused_files(walked: &Walked) -> Vec<&str> {
        walked.refused.iter().map(|refused| refused.shown.as_str()).collect()
    }

    fn codes(walked: &mut Walked) -> Vec<&'static str> {
        walked.report.finish();
        walked.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect()
    }

    #[test]
    fn a_path_is_shown_relative_to_the_current_folder_when_it_is_under_it() {
        let folder = Folder::new("shown");
        folder.file("a/b.lcf", b"");
        let current = Some(folder.0.as_path());
        assert_eq!(shown(&folder.0.join("a").join("b.lcf"), current), "a/b.lcf");
        assert_eq!(shown(&folder.0.join("a").join("..").join("a").join("b.lcf"), current), "a/b.lcf");
        assert_eq!(shown(&folder.0, current), ".");
        assert_eq!(shown(&folder.0.join("a").join(".."), current), ".");
        assert_eq!(shown(&folder.0.join("missing"), current), "missing");
        assert_eq!(shown(&folder.0.join("missing").join("x.lcf"), current), "missing/x.lcf");
        assert_eq!(shown(Path::new(""), current), "\"\"");
        let elsewhere = shown(&folder.0, Some(&folder.0.join("a")));
        assert!(elsewhere.contains("labcraft-walk-shown") && !elsewhere.contains('\\'), "{elsewhere}");
    }

    #[test]
    fn what_a_parent_folder_reaches_under_the_current_folder_is_relative_to_it() {
        let folder = Folder::new("parent");
        folder.file("a.lcf", b"").file("sub/b.lcf", b"");
        let sub = folder.0.join("sub");
        let walked = walk(&Disk, std::slice::from_ref(&folder.0), Some(&sub), &Limits::default());
        let names = shown_files(&walked);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"b.lcf"), "{names:?}");
        assert!(names.iter().any(|name| name.ends_with("/a.lcf")), "{names:?}");
    }

    /// For a library that a `use` asks for: its sub-folders are other libraries, and an empty one is simply not it.
    #[test]
    fn walking_a_folder_alone_stays_in_it_and_says_nothing_when_it_holds_nothing() {
        let folder = Folder::new("alone");
        folder.file("a.lcf", b"").file("sub/b.lcf", b"").file("only-sub/deep/c.lcf", b"");
        let mut walked = walk_alone(&Disk, &folder.0, Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["a.lcf"]);
        assert!(codes(&mut walked).is_empty());
        let mut walked = walk_alone(&Disk, &folder.0.join("only-sub"), Some(&folder.0), &Limits::default());
        assert!(walked.files.is_empty());
        assert!(codes(&mut walked).is_empty(), "neither LC0002 nor LC0010");
    }

    #[test]
    fn files_come_in_the_order_of_their_paths_not_of_their_folders() {
        let folder = Folder::new("order");
        folder.file("a.lcf", b"").file("a/x.lct", b"").file("a-b.lcf", b"").file("a0.lcf", b"").file("Z.lcf", b"").file("note.txt", b"");
        let mut walked = folder.walk();
        assert_eq!(shown_files(&walked), ["Z.lcf", "a-b.lcf", "a.lcf", "a/x.lct", "a0.lcf"]);
        assert!(codes(&mut walked).is_empty());
    }

    #[test]
    fn a_folder_that_starts_with_a_dot_is_skipped_unless_it_is_named() {
        let folder = Folder::new("hidden");
        folder.file(".hidden/a.lcf", b"").file("b.lcf", b"").file(".c.lcf", b"");
        assert_eq!(shown_files(&folder.walk()), [".c.lcf", "b.lcf"]);
        let walked = walk(&Disk, &[folder.0.join(".hidden")], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), [".hidden/a.lcf"]);
    }

    #[test]
    fn a_file_named_twice_or_through_its_folder_counts_once() {
        let folder = Folder::new("twice");
        folder.file("sub/a.lcf", b"");
        let file = folder.0.join("sub").join("a.lcf");
        let sub = folder.0.join("sub");
        let dots = folder.0.join("sub").join("..").join("sub").join("a.lcf");
        let mut walked = walk(&Disk, &[file.clone(), file, dots, sub.clone(), folder.0.clone(), sub], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["sub/a.lcf"]);
        assert!(codes(&mut walked).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_spelled_in_another_case_is_the_same_folder() {
        let folder = Folder::new("case");
        folder.file("sub/a.lcf", b"");
        let walked = walk(&Disk, &[folder.0.join("SUB").join("a.lcf"), folder.0.join("sub").join("a.lcf")], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["sub/a.lcf"]);
        let walked = walk(&Disk, &[folder.0.join("SUB"), folder.0.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["sub/a.lcf"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_stream_of_a_file_is_not_a_program() {
        let folder = Folder::new("stream");
        folder.file("notes.txt", b"");
        let stream = folder.0.join("notes.txt:a.lcf");
        fs::write(&stream, b"").unwrap();
        let mut walked = walk(&Disk, &[stream], Some(&folder.0), &Limits::default());
        assert!(walked.files.is_empty());
        assert_eq!(codes(&mut walked), ["LC0008"]);
    }

    #[test]
    fn naming_a_folder_inside_another_loses_nothing_and_it_has_its_own_bounds() {
        let folder = Folder::new("inside");
        // Too deep for the folder given first, not for the one given inside it.
        folder.file(&format!("{}/deep.lcf", nested(BOUNDS.levels)), b"").file("a.lcf", b"");
        let inner = folder.0.join("d");
        let mut walked = walk(&Disk, &[folder.0.clone(), inner.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(walked.files.len(), 2);
        assert_eq!(codes(&mut walked), ["LC0010"]);
        let mut reversed = walk(&Disk, &[inner, folder.0.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(reversed.files, walked.files);
        assert_eq!(codes(&mut reversed), ["LC0010"]);
    }

    #[test]
    fn a_folder_inside_another_with_nothing_to_check_says_so_whatever_the_order() {
        let folder = Folder::new("inside-empty");
        folder.file("a.lcf", b"").file("empty/note.txt", b"");
        let empty = folder.0.join("empty");
        let mut one = walk(&Disk, &[folder.0.clone(), empty.clone()], Some(&folder.0), &Limits::default());
        let mut other = walk(&Disk, &[empty, folder.0.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(codes(&mut one), ["LC0002"]);
        assert_eq!(codes(&mut other), ["LC0002"]);
    }

    #[test]
    fn the_order_of_the_arguments_changes_nothing() {
        let folder = Folder::new("arguments");
        folder.file("a.lcf", b"").file("empty/note.txt", b"").file("full/b.lcf", b"");
        let paths = [folder.0.clone(), folder.0.join("empty"), folder.0.join("full"), folder.0.join("missing")];
        let mut one = walk(&Disk, &paths, Some(&folder.0), &Limits::default());
        let reversed: Vec<PathBuf> = paths.iter().rev().cloned().collect();
        let mut other = walk(&Disk, &reversed, Some(&folder.0), &Limits::default());
        assert_eq!(one.files, other.files);
        assert_eq!(codes(&mut one), ["LC0002", "LC0001"]);
        assert_eq!(one.report.diagnostics(), {
            other.report.finish();
            other.report.diagnostics()
        });
    }

    #[test]
    fn a_name_that_only_ends_like_an_extension_has_none() {
        let folder = Folder::new("extension");
        folder.file(".lcf", b"").file("a.LCF", b"").file("b.lcf.txt", b"").file("c.lct", b"");
        assert_eq!(shown_files(&folder.walk()), ["c.lct"]);
        let mut named = walk(&Disk, &[folder.0.join("a.LCF")], Some(&folder.0), &Limits::default());
        assert_eq!(refused_files(&named), ["a.LCF"]);
        assert_eq!(codes(&mut named), ["LC0008"]);
    }

    #[test]
    fn nothing_found_is_an_error_for_each_path() {
        let folder = Folder::new("nothing");
        folder.file("empty/note.txt", b"").file("full/a.lcf", b"");
        let paths = [folder.0.join("empty"), folder.0.join("full"), folder.0.join("missing")];
        let mut walked = walk(&Disk, &paths, Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["full/a.lcf"]);
        assert!(walked.refused.is_empty(), "a folder or a missing path is not a file");
        assert_eq!(codes(&mut walked), ["LC0002", "LC0001"]);
    }

    fn nested(levels: usize) -> String {
        vec!["d"; levels].join("/")
    }

    #[test]
    fn folders_stop_at_32_levels() {
        let folder = Folder::new("levels");
        // The folder given is the first level.
        folder.file(&format!("{}/kept.lcf", nested(BOUNDS.levels - 1)), b"");
        let mut walked = folder.walk();
        assert_eq!(walked.files.len(), 1);
        assert!(codes(&mut walked).is_empty());
        folder.file(&format!("{}/lost.lcf", nested(BOUNDS.levels)), b"");
        let mut walked = folder.walk();
        assert_eq!(walked.files.len(), 1);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn a_folder_with_only_what_is_left_out_is_not_said_to_be_empty() {
        let folder = Folder::new("only-deep");
        folder.file(&format!("{}/lost.lcf", nested(BOUNDS.levels)), b"");
        assert_eq!(codes(&mut folder.walk()), ["LC0010"]);
    }

    fn small(folder: &Folder, files: usize) -> Walked {
        walk_within(&Disk, std::slice::from_ref(&folder.0), Some(&folder.0), &Limits::default(), Bounds { levels: 32, files, alone: false })
    }

    #[test]
    fn the_files_that_stay_are_the_first_in_the_order_of_the_paths() {
        let folder = Folder::new("files");
        for name in ["d", "b", "e", "a", "c", "g", "f"] {
            folder.file(&format!("{name}.lcf"), b"");
        }
        let mut walked = small(&folder, 3);
        assert_eq!(shown_files(&walked), ["a.lcf", "b.lcf", "c.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn a_refused_file_counts_as_a_file() {
        let folder = Folder::new("exactly");
        folder.file("a.lcf", b"").file("b.lcf", b"");
        let mut walked = small(&folder, 2);
        assert_eq!(shown_files(&walked).len(), 2);
        assert!(codes(&mut walked).is_empty());
    }

    #[test]
    fn folders_are_bounded_too() {
        let folder = Folder::new("folders");
        for name in ["a", "b", "c", "d"] {
            folder.file(&format!("{name}/x.lcf"), b"");
        }
        // The folder given and two of those in it.
        let mut walked = small(&folder, 3);
        assert_eq!(shown_files(&walked), ["a/x.lcf", "b/x.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn the_bytes_of_a_folder_are_bounded_in_the_order_of_the_paths() {
        let folder = Folder::new("bytes");
        folder.file("a.lcf", &[0; 6]).file("b.lcf", &[0; 4]).file("c.lcf", &[0; 1]).file("d.lcf", &[0; 1]);
        let mut limits = Limits::default();
        limits.max_library_size = 10;
        let mut walked = walk(&Disk, std::slice::from_ref(&folder.0), Some(&folder.0), &limits);
        assert_eq!(shown_files(&walked), ["a.lcf", "b.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn a_file_over_its_own_limit_counts_only_for_what_would_be_read() {
        let folder = Folder::new("large");
        folder.file("a.lcf", &[0; 100]).file("b.lcf", &[0; 1]);
        let mut limits = Limits::default();
        limits.max_library_size = 10;
        limits.max_file_size = 5;
        let mut walked = walk(&Disk, std::slice::from_ref(&folder.0), Some(&folder.0), &limits);
        assert_eq!(shown_files(&walked), ["a.lcf", "b.lcf"]);
        assert!(codes(&mut walked).is_empty());
    }

    #[test]
    fn an_error_of_the_system_says_its_cause_alone() {
        let wrapped = io::Error::new(io::ErrorKind::PermissionDenied, "cannot save C:\\a\\long\\path: Accès refusé");
        assert_eq!(io_text(&wrapped), "permission denied");
        assert_eq!(io_text(&io::Error::from(io::ErrorKind::NotFound)), "entity not found");
    }

    #[cfg(unix)]
    #[test]
    fn what_two_paths_reach_is_said_once() {
        let folder = Folder::new("said-once");
        let fifo = folder.0.join("pipe.lcf");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        std::os::unix::fs::symlink(&folder.0, folder.0.join("loop")).unwrap();
        fs::create_dir(folder.0.join("sub")).unwrap();
        std::os::unix::fs::symlink(&folder.0, folder.0.join("sub").join("loop")).unwrap();
        let paths = [folder.0.clone(), fifo.clone(), folder.0.join("sub"), fifo];
        let walked = walk(&Disk, &paths, Some(&folder.0), &Limits::default());
        assert_eq!(refused_files(&walked), ["pipe.lcf"]);
        assert_eq!(walked.report.errors(), 2, "the file, and `sub`, which holds nothing");
        assert_eq!(walked.report.warnings(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_file_is_known_by_its_real_path() {
        let folder = Folder::new("refused-once");
        fs::create_dir(folder.0.join("sub")).unwrap();
        let sub = folder.0.join("sub");
        let fifo = sub.join("pipe.lcf");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let spelled = sub.join(".").join("pipe.lcf");
        let mut walked = walk(&Disk, &[folder.0.clone(), fifo, spelled], Some(&sub), &Limits::default());
        assert_eq!(refused_files(&walked), ["pipe.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0001"]);
    }

    #[cfg(unix)]
    #[test]
    fn two_names_that_show_alike_are_two_files() {
        use std::os::unix::ffi::OsStrExt as _;
        let folder = Folder::new("not-unicode");
        let names = [folder.0.join(std::ffi::OsStr::from_bytes(b"a\xFF.txt")), folder.0.join(std::ffi::OsStr::from_bytes(b"a\xFE.txt"))];
        for name in &names {
            fs::write(name, b"").unwrap();
        }
        let mut walked = walk(&Disk, &names, Some(&folder.0), &Limits::default());
        assert_eq!(walked.refused.len(), 2);
        assert_eq!(codes(&mut walked), ["LC0008", "LC0008"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_special_file_with_the_extension_is_refused_never_read_and_counted() {
        let folder = Folder::new("fifo");
        let fifo = folder.0.join("pipe.lcf");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let mut walked = folder.walk();
        assert_eq!(refused_files(&walked), ["pipe.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0001"]);
        let mut walked = walk(&Disk, std::slice::from_ref(&fifo), Some(&folder.0), &Limits::default());
        assert_eq!(refused_files(&walked), ["pipe.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0001"]);
        assert!(std::process::Command::new("mkfifo").arg(folder.0.join("other.lcf")).status().unwrap().success());
        let mut walked = small(&folder, 1);
        assert_eq!(refused_files(&walked), ["other.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010", "LC0001"]);
    }
}
