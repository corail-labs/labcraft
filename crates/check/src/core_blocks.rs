//! The blocks of the primitives of `core` (spec, appendix A.1): the files of `libs/core`, which give their terminals and
//! their outline in the grammar that every library uses, embedded here so that every drawing has them, in WASM too. They
//! are no library: `use core` names the primitives, whose signatures stay in [`crate::primitives`] (spec A.1). `cargo xtask libs` allows this one embedding of `libs/` by a crate of `crates/`.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use labcraft_graph::{Decl, File, Icon, Signature};
use labcraft_types::{Interner, Limits, Name};

use crate::places::Block;

/// Each file of `libs/core`, by the name of its primitive.
pub const FILES: &[(&str, &str)] = &[
    ("abs", include_str!("../../../libs/core/abs.lcf")),
    ("add", include_str!("../../../libs/core/add.lcf")),
    ("and", include_str!("../../../libs/core/and.lcf")),
    ("div", include_str!("../../../libs/core/div.lcf")),
    ("eq", include_str!("../../../libs/core/eq.lcf")),
    ("ge", include_str!("../../../libs/core/ge.lcf")),
    ("gt", include_str!("../../../libs/core/gt.lcf")),
    ("le", include_str!("../../../libs/core/le.lcf")),
    ("lt", include_str!("../../../libs/core/lt.lcf")),
    ("mul", include_str!("../../../libs/core/mul.lcf")),
    ("ne", include_str!("../../../libs/core/ne.lcf")),
    ("neg", include_str!("../../../libs/core/neg.lcf")),
    ("not", include_str!("../../../libs/core/not.lcf")),
    ("or", include_str!("../../../libs/core/or.lcf")),
    ("select", include_str!("../../../libs/core/select.lcf")),
    ("sqrt", include_str!("../../../libs/core/sqrt.lcf")),
    ("sub", include_str!("../../../libs/core/sub.lcf")),
    ("unitless", include_str!("../../../libs/core/unitless.lcf")),
    ("with_unit", include_str!("../../../libs/core/with_unit.lcf")),
];

/// Whether a file is one of these, as embedded, by its name and its bytes: `check` reads it as the block of a primitive,
/// not as a program (spec A.1), so that `libs/core` says nothing of them, whatever folder is given. A file changed in any
/// way, or under another name, is a program again, and is judged as one.
pub fn is_block_file(shown: &str, bytes: &[u8]) -> bool {
    let name = shown.rsplit(['/', '\\']).next().unwrap_or(shown);
    let Some(stem) = name.strip_suffix(".lcf") else { return false };
    FILES.iter().any(|&(listed, text)| listed == stem && text.as_bytes() == bytes)
}

/// What a file of `libs/core` gives: its block, and the icon of its glyph with the names of its file.
struct Read {
    block: Block,
    icon: Option<Icon>,
    names: Interner,
}

fn read_all() -> &'static HashMap<&'static str, Read> {
    static BLOCKS: OnceLock<HashMap<&'static str, Read>> = OnceLock::new();
    BLOCKS.get_or_init(|| FILES.iter().filter_map(|&(name, text)| Some((name, read(text)?))).collect())
}

/// The block of the primitive `core.name`, when `libs/core` gives it one: read once, for every drawing.
pub fn block(name: &str) -> Option<&'static Block> {
    read_all().get(name).map(|read| &read.block)
}

/// The icon of the glyph of the primitive `core.name` (spec A.1), with the names of its file, which its numbers are
/// written in; none for `with_unit` and `unitless`, whose unit is a text, or for an icon that `check` would refuse.
pub fn icon(name: &str) -> Option<(&'static Icon, &'static Interner)> {
    read_all().get(name).and_then(|read| read.icon.as_ref().map(|icon| (icon, &read.names)))
}

/// The block of a file of `libs/core`, read with names of its own: its outline names no terminal, a template (the tests
/// of this module hold it), so nothing of it is read against the names of a check.
fn read(text: &str) -> Option<Read> {
    let (mut file, names) = parse(text)?;
    let (_, signature) = declared(&file)?;
    let (inputs, outputs) = terminals(signature);
    let block = Block::read(file.terminals.as_ref(), file.outline.as_ref(), None, (&inputs, &outputs), &names, HashSet::new());
    let icon = file.icon.take().filter(|icon| crate::icon_refused(icon, &names).is_none());
    Some(Read { block, icon, names })
}

fn parse(text: &str) -> Option<(File, Interner)> {
    let mut names = Interner::new();
    let file = labcraft_format::read(text.as_bytes(), &mut names, &Limits::default()).file?;
    Some((file, names))
}

/// The name and the signature of its `extern` declaration.
fn declared(file: &File) -> Option<(Name, &Signature)> {
    match &file.decl {
        Decl::Extern(declared) => Some((declared.name, &declared.signature)),
        Decl::Function(_) | Decl::Type(_) => None,
    }
}

/// The names of its terminals: inputs, outputs.
fn terminals(signature: &Signature) -> (Vec<Name>, Vec<Name>) {
    (signature.params.iter().map(|param| param.name).collect(), signature.outputs.iter().map(|output| output.name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::Read;
    use crate::primitives::{ParamKind, PrimitiveLibrary, primitive};

    /// Each file reads, names its primitive, and gives the terminals of the table in their order: the names of its
    /// inputs (its unit, `kind unit`, is no terminal) and of its outputs (spec A.1).
    #[test]
    fn each_file_gives_the_terminals_of_its_primitive() {
        for &(name, text) in FILES {
            let (file, names) = parse(text).unwrap_or_else(|| panic!("{name}: the file does not read"));
            let (declared_name, signature) = declared(&file).unwrap_or_else(|| panic!("{name}: no `extern` declaration"));
            assert_eq!(names.resolve(declared_name), name);
            let table = primitive(PrimitiveLibrary::Core, name).unwrap_or_else(|| panic!("{name}: no primitive of this name"));
            let (inputs, outputs) = terminals(signature);
            let text = |list: &[Name]| list.iter().map(|name| names.resolve(*name)).collect::<Vec<_>>();
            let wanted: Vec<&str> = table.params.iter().filter(|param| param.kind == ParamKind::Value).map(|param| param.name).collect();
            assert_eq!(text(&inputs), wanted, "{name}");
            assert_eq!(text(&outputs), table.outputs.unwrap_or_default().to_vec(), "{name}");
            assert!(block(name).is_some(), "{name}");
        }
    }

    /// Its sections read with no problem, and its outline is `template d`, which names no terminal.
    #[test]
    fn each_file_has_sections_that_read_and_the_d() {
        for &(name, text) in FILES {
            let Some((file, names)) = parse(text) else { panic!("{name}") };
            let Some((_, signature)) = declared(&file) else { panic!("{name}") };
            let (inputs, outputs) = terminals(signature);
            let section = file.terminals.as_ref().unwrap_or_else(|| panic!("{name}: no `terminals` section"));
            let (_, problems) = labcraft_format::block::read(section, &inputs, &outputs, &names);
            assert!(problems.is_empty(), "{name}: {problems:?}");
            assert!(block(name).is_some_and(|block| matches!(block.outline, Some(Read::D(_)))), "{name}");
        }
    }

    /// Every file of `libs/core` is in `FILES`, and only those.
    #[test]
    fn files_lists_the_folder() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../libs/core");
        let mut found: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .filter_map(|entry| entry.unwrap().path().file_stem().map(|stem| stem.to_string_lossy().into_owned()))
            .collect();
        found.sort();
        let listed: Vec<&str> = FILES.iter().map(|&(name, _)| name).collect();
        assert_eq!(found, listed);
    }

    /// A file of `libs/core` is known by its name and its bytes, not by one of them.
    #[test]
    fn a_block_file_is_known_whole() {
        let Some(&(_, add)) = FILES.iter().find(|&&(name, _)| name == "add") else { panic!() };
        assert!(is_block_file("libs/core/add.lcf", add.as_bytes()));
        assert!(is_block_file("add.lcf", add.as_bytes()));
        assert!(is_block_file("core\\add.lcf", add.as_bytes()));
        assert!(!is_block_file("libs/core/sub.lcf", add.as_bytes()));
        assert!(!is_block_file("libs/core/add.lct", add.as_bytes()));
        assert!(!is_block_file("libs/core/add.lcf", format!("{add}\n").as_bytes()));
    }

    /// Only the primitives drawn as a D have a file (A.1): `min` and `max` have the ordinary block.
    #[test]
    fn a_primitive_without_a_glyph_has_no_file() {
        assert!(block("min").is_none());
        assert!(block("call").is_none());
    }
}
