//! Writes the seeds that the fuzzer would not reach by itself to the folder given as argument:
//!
//! - the programs of the Nesting family, at the default limit and one level above it: after 100,000 runs from the other
//!   seeds, the deepest input is 4 levels deep. Generated, so that the grammar cannot leave them behind;
//! - the libraries of the repository for the targets `check` and `diagram`: the files of each folder of `examples/`,
//!   `libs/`, `docs/rfc/`, `tests/acceptance/` and `tests/harness/routing-cases/` (the drawings built to break the rules of
//!   routing), as many as fit in 64 KiB and at most 32, separated by two NUL bytes, so that calls and types cross files;
//! - for the target `simulation`, simulation files of arrays and of inline tables nested at the limit of the reader of `toml`
//!   and one level above it (`runtime::simulation::TOML_NESTING`, cli.md 6.2).
//!
//! Run from `fuzz/`.

use std::path::{Path, PathBuf};
use std::{env, fs};

use labcraft_runtime::simulation::TOML_NESTING;
use labcraft_types::Limits;

#[path = "../../../crates/format/tests/deep/mod.rs"]
mod deep;

/// As the target `check` splits its input.
const SEPARATOR: &[u8] = b"\0\0";
const FILES: usize = 32;
/// The longest input that `run.sh` lets libFuzzer try.
const MAX_LEN: usize = 65_536;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = env::args().nth(1).ok_or("usage: deep_seeds FOLDER")?;
    let folder = Path::new(&folder);
    fs::create_dir_all(folder)?;
    let limit = usize::try_from(Limits::default().max_nesting())?;
    for (kind, program) in deep::DEEP {
        let kind = kind.replace(' ', "-");
        fs::write(folder.join(format!("{kind}-at-the-limit.lcf")), program(limit))?;
        fs::write(folder.join(format!("{kind}-above-the-limit.lcf")), program(limit + 1))?;
    }
    // A key of one segment adds no level, and each array or inline table one (cli.md 6.2).
    let toml = usize::try_from(TOML_NESTING)?;
    for depth in [toml, toml + 1] {
        let arrays = format!("a = {}{}\n", "[".repeat(depth), "]".repeat(depth));
        let tables = format!("a = {}1{}\n", "{ b = ".repeat(depth), " }".repeat(depth));
        fs::write(folder.join(format!("simulation-arrays-{depth}.toml")), arrays)?;
        fs::write(folder.join(format!("simulation-tables-{depth}.toml")), tables)?;
    }
    let mut libraries = Vec::new();
    for root in ["../examples", "../libs", "../docs/rfc", "../tests/acceptance", "../tests/harness/routing-cases"] {
        libraries_under(Path::new(root), &mut libraries)?;
    }
    for (index, files) in libraries.iter().enumerate() {
        // As many files of the folder as fit, in order; a file too large for any seed is left out.
        let (mut seed, mut count, mut part) = (Vec::new(), 0, 0);
        for file in files {
            let bytes = fs::read(file)?;
            if count == FILES || (count > 0 && seed.len() + SEPARATOR.len() + bytes.len() > MAX_LEN) {
                fs::write(folder.join(format!("library-{index}-{part}.bin")), std::mem::take(&mut seed))?;
                (count, part) = (0, part + 1);
            }
            if bytes.len() > MAX_LEN {
                continue;
            }
            if count > 0 {
                seed.extend_from_slice(SEPARATOR);
            }
            seed.extend(bytes);
            count += 1;
        }
        if count > 1 {
            fs::write(folder.join(format!("library-{index}-{part}.bin")), seed)?;
        }
    }
    Ok(())
}

/// The programs of each folder under `folder`, in the order of their paths, one list per folder that has some.
fn libraries_under(folder: &Path, found: &mut Vec<Vec<PathBuf>>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder)?.map(|entry| entry.map(|entry| entry.path())).collect::<Result<_, _>>()?;
    entries.sort();
    let programs: Vec<PathBuf> =
        entries.iter().filter(|path| path.extension().is_some_and(|extension| extension == "lcf" || extension == "lct")).cloned().collect();
    if programs.len() > 1 {
        found.push(programs);
    }
    for path in entries.iter().filter(|path| path.is_dir()) {
        libraries_under(path, found)?;
    }
    Ok(())
}
