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
    ("max", include_str!("../../../libs/core/max.lcf")),
    ("min", include_str!("../../../libs/core/min.lcf")),
    ("mul", include_str!("../../../libs/core/mul.lcf")),
    ("ne", include_str!("../../../libs/core/ne.lcf")),
    ("neg", include_str!("../../../libs/core/neg.lcf")),
    ("not", include_str!("../../../libs/core/not.lcf")),
    ("or", include_str!("../../../libs/core/or.lcf")),
    ("pow", include_str!("../../../libs/core/pow.lcf")),
    ("quotient", include_str!("../../../libs/core/quotient.lcf")),
    ("remainder", include_str!("../../../libs/core/remainder.lcf")),
    ("round_to_i32", include_str!("../../../libs/core/round_to_i32.lcf")),
    ("select", include_str!("../../../libs/core/select.lcf")),
    ("shl", include_str!("../../../libs/core/shl.lcf")),
    ("shr", include_str!("../../../libs/core/shr.lcf")),
    ("sqrt", include_str!("../../../libs/core/sqrt.lcf")),
    ("sub", include_str!("../../../libs/core/sub.lcf")),
    ("to_f32", include_str!("../../../libs/core/to_f32.lcf")),
    ("to_i32", include_str!("../../../libs/core/to_i32.lcf")),
    ("unitless", include_str!("../../../libs/core/unitless.lcf")),
    ("with_unit", include_str!("../../../libs/core/with_unit.lcf")),
    ("xor", include_str!("../../../libs/core/xor.lcf")),
];

/// Whether a file is one of these, by its name and its canonical form, which is the embedded file byte for byte: `check`
/// reads it as the block of a primitive, not as a program (spec A.1), so that `libs/core` says nothing of them, whatever
/// folder is given. A change that `fmt` undoes (a byte order mark, CR LF, the layout) changes nothing; a file changed in
/// any other way, or under another name, is a program again, and is judged as one.
pub fn is_block_file(shown: &str, bytes: &[u8], limits: &Limits) -> bool {
    let name = shown.rsplit(['/', '\\']).next().unwrap_or(shown);
    let Some(stem) = name.strip_suffix(".lcf") else { return false };
    let Some(&(_, text)) = FILES.iter().find(|&&(listed, _)| listed == stem) else { return false };
    text.as_bytes() == bytes || labcraft_format::format(bytes, &mut Interner::new(), limits).text.as_deref() == Some(text)
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

    /// Its sections read with no problem, and its outline is `template primitive`, which names no terminal.
    #[test]
    fn each_file_has_sections_that_read_and_the_d() {
        for &(name, text) in FILES {
            let Some((file, names)) = parse(text) else { panic!("{name}") };
            let Some((_, signature)) = declared(&file) else { panic!("{name}") };
            let (inputs, outputs) = terminals(signature);
            let section = file.terminals.as_ref().unwrap_or_else(|| panic!("{name}: no `terminals` section"));
            let (_, problems) = labcraft_format::block::read(section, &inputs, &outputs, &names);
            assert!(problems.is_empty(), "{name}: {problems:?}");
            assert!(block(name).is_some_and(|block| matches!(block.outline, Some(Read::D { trimmed: true, .. }))), "{name}");
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

    /// A file of `libs/core` is known by its name and its text, not by one of them.
    #[test]
    fn a_block_file_is_known_whole() {
        let Some(&(_, add)) = FILES.iter().find(|&&(name, _)| name == "add") else { panic!() };
        assert!(is_block_file("libs/core/add.lcf", add.as_bytes(), &Limits::default()));
        assert!(is_block_file("add.lcf", add.as_bytes(), &Limits::default()));
        assert!(is_block_file("core\\add.lcf", add.as_bytes(), &Limits::default()));
        assert!(!is_block_file("libs/core/sub.lcf", add.as_bytes(), &Limits::default()));
        assert!(!is_block_file("libs/core/add.lct", add.as_bytes(), &Limits::default()));
    }

    /// What `fmt` undoes leaves a file of `libs/core` a block file, so that `check` says the same before and after `fmt`:
    /// a byte order mark, CR LF, another layout. Any other change makes it a program.
    #[test]
    fn a_block_file_is_known_by_its_canonical_form() {
        let Some(&(_, pow)) = FILES.iter().find(|&&(name, _)| name == "pow") else { panic!() };
        let known = |text: &str| is_block_file("libs/core/pow.lcf", text.as_bytes(), &Limits::default());
        assert!(known(&format!("\u{FEFF}{pow}")), "a byte order mark");
        assert!(known(&pow.replace('\n', "\r\n")), "CR LF");
        assert!(known(&pow.replacen("x: N, exponent", "x: N,  exponent", 1)), "a space more");
        assert!(known(&format!("{pow}\n")), "a line feed more at the end");
        assert!(!known(&pow.replacen("exponent: i32", "exponent: f64", 1)), "another type");
        assert!(!known(&pow.replacen("a constant.", "a constant!", 1)), "another comment");
        assert!(!known(&pow.replacen("labcraft 1", "labcraft 2", 1)), "a file that does not read");
        assert!(!is_block_file("libs/core/sub.lcf", format!("\u{FEFF}{pow}").as_bytes(), &Limits::default()), "under another name");
    }

    /// The canonical form is that of the limits of the command, which `fmt` takes too: under a limit of the size of a
    /// file that the file goes over, it has none, and the file is a program, as `fmt` leaves it.
    #[test]
    fn the_canonical_form_takes_the_limits_of_the_command() {
        let Some(&(_, pow)) = FILES.iter().find(|&&(name, _)| name == "pow") else { panic!() };
        let spaced = format!("{pow}\n\n\n");
        let large = Limits::default();
        let mut small = Limits::default();
        small.max_file_size = pow.len() as u64;
        assert!(is_block_file("pow.lcf", spaced.as_bytes(), &large));
        assert!(!is_block_file("pow.lcf", spaced.as_bytes(), &small));
    }

    /// Only the primitives drawn as a D have a file (A.1): every one of `core` but `call`, which has the ordinary block.
    #[test]
    fn only_call_has_no_file() {
        assert!(block("min").is_some());
        assert!(block("call").is_none());
    }
}
