//! Writes the programs of the Nesting family, at the default limit and one level above it, to the folder given
//! as argument. The fuzzer would not reach them by itself: after 100,000 runs from the other seeds, the deepest
//! input is 4 levels deep. Generated, so that the grammar cannot leave them behind.

use std::path::Path;
use std::{env, fs};

use labcraft_types::Limits;

#[path = "../../../crates/format/tests/deep/mod.rs"]
mod deep;

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
    Ok(())
}
