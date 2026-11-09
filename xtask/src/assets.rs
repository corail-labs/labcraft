//! `cargo xtask assets`: every asset has its line in `ATTRIBUTION.md` (`docs/craft-ecosystem.md`), and every file that
//! `ATTRIBUTION.md` names exists. The assets are the files of `assets/` and the data that `cargo xtask unicode` writes
//! from the Unicode Character Database.

use std::path::Path;

/// Files outside `assets/` that come from elsewhere.
const DATA: [&str; 3] = ["crates/types/src/text/tables.rs", "crates/types/tests/data/GraphemeBreakTest.txt", "crates/types/LICENSE-Unicode.txt"];

pub fn files(root: &Path) -> Result<Vec<String>, String> {
    let mut files: Vec<String> = DATA.iter().map(|file| (*file).to_owned()).collect();
    walk(root, &root.join("assets"), &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root).map_err(|error| format!("{}: {error}", path.display()))?;
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// What is wrong: an asset that no line of `ATTRIBUTION.md` names, a line that names a file that does not exist, a line
/// whose first cell is not one path.
pub fn check(root: &Path, files: &[String], attribution: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let mut listed = Vec::new();
    for cell in first_cells(attribution) {
        match cell.strip_prefix('`').and_then(|cell| cell.strip_suffix('`')).filter(|path| !path.contains('`')) {
            Some(path) if root.join(path).is_file() => listed.push(path),
            Some(path) => violations.push(format!("ATTRIBUTION.md has a line for `{path}`, which is not a file")),
            None => violations.push(format!("ATTRIBUTION.md: the first cell `{cell}` is not one path in backquotes")),
        }
    }
    for file in files {
        if !listed.contains(&file.as_str()) {
            violations.push(format!("{file}: no line in ATTRIBUTION.md"));
        }
    }
    violations
}

/// The first cell of each line of the tables, but their header and the line under it: the file of the line.
fn first_cells(attribution: &str) -> Vec<&str> {
    let mut cells = Vec::new();
    let mut in_table = false;
    for line in attribution.lines() {
        let Some(row) = line.trim().strip_prefix('|') else {
            in_table = false;
            continue;
        };
        let cell = row.split('|').next().unwrap_or_default().trim();
        if !in_table {
            in_table = true; // the header
        } else if !cell.starts_with("---") {
            cells.push(cell);
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_cell_of_a_line_names_its_file() {
        let text = "| File | Licence |\n|---|---|\n| `assets/a.ttf` | see `assets/LICENSE-a.md` |\n\nText.\n\n| File |\n|---|\n| `assets/b.ttf` |\n";
        assert_eq!(first_cells(text), ["`assets/a.ttf`", "`assets/b.ttf`"]);
    }

    #[test]
    fn an_asset_without_its_line_and_a_line_without_its_file_are_violations() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new("."));
        let files = vec!["assets/fonts/Hack-Regular.ttf".to_owned(), "assets/fonts/LICENSE-Hack.md".to_owned()];
        // Named only in the licence column, the licence has no line of its own.
        let text = "| File | Licence |\n|---|---|\n| `assets/fonts/Hack-Regular.ttf` | `assets/fonts/LICENSE-Hack.md` |\n| `docs/images/none.png` | |\n| Hack | |\n";
        assert_eq!(
            check(root, &files, text),
            [
                "ATTRIBUTION.md has a line for `docs/images/none.png`, which is not a file",
                "ATTRIBUTION.md: the first cell `Hack` is not one path in backquotes",
                "assets/fonts/LICENSE-Hack.md: no line in ATTRIBUTION.md",
            ]
        );
    }
}
