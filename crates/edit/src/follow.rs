//! A rename made in the text (spec 11, commands.md 9.1): when a write changes a function, a name that a line lost and a
//! name that another line gained are one node when the two lines are the same once the renames of that write are
//! applied, and the entries of `layout` that named the old name take the new one, in their place in the text, nothing
//! else of the file changing (decisions of the owner, 2026-11-25).
//!
//! Two lines are the same when their tokens are, comments left out, the names they bind and read mapped by the renames,
//! in the same body: that of the function, or the same body of a structure that is the same itself. The tokens of a
//! structure are those of its head, its arms and its end, without the lines of its bodies, which are lines of their own.
//! Who reads a value does not count. The pairs are found step by step, each line taking the one line that can be it,
//! then checked once all are known: a pair whose line needs a rename that was not found goes, until none goes.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use labcraft_format::{Lexer, Token, TokenKind, decode, read};
use labcraft_graph::{Decl, Function, Layout, LayoutKind, NodeRef, Rhs, SourceMap, StatementKind};
use labcraft_types::{Interner, Limits, Name, Position, Report, Span};

use crate::scope::{self, Loc};

/// At most this many pairs of lines of one shape are compared *(proposed)*: lines that many of one shape cannot be told
/// apart, and a rename among them is not guessed.
const MAX_COMPARED: usize = 4096;
/// At most this many passes *(proposed)*: each finds the lines that the renames found before tell apart.
const MAX_PASSES: usize = 64;

/// The text after a write, with the entries of `layout` that its renames moved, and those renames, old name first (a
/// label with its `#`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Followed {
    pub bytes: Vec<u8>,
    pub renamed: Vec<(String, String)>,
}

/// The two kinds of names that a rename changes: a value, or a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Space {
    Value,
    Label,
}

/// A name of one kind.
type Named = (Space, Name);

/// Renames, both ways, so that a name has at most one new name and a new name at most one old one.
#[derive(Clone, Debug, Default)]
struct Renames {
    to: HashMap<Named, Name>,
    from: HashMap<Named, Name>,
}

impl Renames {
    fn get(&self, space: Space, old: Name) -> Option<Name> {
        self.to.get(&(space, old)).copied()
    }

    /// Whether `old` may become `new` beside these renames.
    fn allows(&self, space: Space, old: Name, new: Name) -> bool {
        self.to.get(&(space, old)).is_none_or(|known| *known == new) && self.from.get(&(space, new)).is_none_or(|known| *known == old)
    }

    fn insert(&mut self, space: Space, old: Name, new: Name) {
        self.to.insert((space, old), new);
        self.from.insert((space, new), old);
    }
}

/// A line of the function, as the pairing reads it.
struct Line {
    /// Its tokens, by their place in [`Side::tokens`].
    key: Vec<usize>,
    /// What it binds: its targets, the patterns, `in` clauses and carries of its bodies, and its label.
    binds: HashSet<Named>,
    /// The structure that holds it, and which of its bodies.
    parent: Option<(Loc, usize)>,
    /// Its place among the lines of its body that have its tokens, so that of two structures alike the first is the first.
    ordinal: usize,
}

/// One text of the function.
struct Side<'t> {
    tokens: Vec<Token<'t>>,
    lines: HashMap<Loc, Line>,
}

/// What a line binds: its targets, the patterns, `in` clauses and carries of its bodies, and its label.
fn binds(statement: &labcraft_graph::Statement) -> HashSet<Named> {
    let mut binds: HashSet<Named> = scope::targets(statement).into_iter().map(|name| (Space::Value, name)).collect();
    for sub in 0..scope::subs(statement).len() {
        binds.extend(scope::head_names(statement, sub).into_iter().map(|name| (Space::Value, name)));
    }
    binds.extend(statement.label.map(|label| (Space::Label, label.name)));
    binds
}

/// The names that the lines of the function bind, of each kind, and its inputs and outputs, which no rename here changes
/// (`signature.rename` does).
fn names_of(function: &Function) -> (HashSet<Named>, HashSet<Name>) {
    let signature =
        function.signature.params.iter().map(|param| param.name).chain(function.signature.outputs.iter().map(|output| output.name)).collect();
    let bound = scope::all(function).iter().filter_map(|loc| scope::statement(function, loc)).flat_map(binds).collect();
    (bound, signature)
}

/// The names that the old text lost and those that the new one gained, of each kind.
fn changed(old: &Function, new: &Function) -> (HashSet<Named>, HashSet<Named>) {
    let ((old_bound, old_signature), (new_bound, new_signature)) = (names_of(old), names_of(new));
    let outside = |named: &Named, bound: &HashSet<Named>, signature: &HashSet<Name>| {
        !bound.contains(named) && !(named.0 == Space::Value && signature.contains(&named.1))
    };
    let lost = old_bound.iter().filter(|named| outside(named, &new_bound, &new_signature) && !old_signature.contains(&named.1)).copied().collect();
    let gained = new_bound.iter().filter(|named| outside(named, &old_bound, &old_signature) && !new_signature.contains(&named.1)).copied().collect();
    (lost, gained)
}

/// The entries of `layout` that the renames of a write move, or `None` when the write renamed nothing that `layout`
/// names: a text that does not parse, or holds no function body, renames nothing.
pub fn follow_renames(before: &[u8], after: &[u8], limits: &Limits) -> Option<Followed> {
    if before == after {
        return None;
    }
    let mut names = Interner::new();
    let old = read(before, &mut names, limits);
    let new = read(after, &mut names, limits);
    let (old_file, new_file) = (old.file?, new.file?);
    let (Decl::Function(old_function), Decl::Function(new_function)) = (&old_file.decl, &new_file.decl) else { return None };
    let layout = new_file.layout.as_ref()?;
    let (lost, gained) = changed(old_function, new_function);
    if lost.is_empty() || gained.is_empty() {
        return None;
    }
    let old_text = decode(before, limits).ok()?;
    let new_text = decode(after, limits).ok()?;
    let old_side = Side::of(old_function, &old.source_map, lex(old_text, &mut names, limits)?);
    let new_side = Side::of(new_function, &new.source_map, lex(new_text, &mut names, limits)?);
    let renames = Pairing { old: &old_side, new: &new_side, lost, gained }.pair()?;
    // `decode` leaves out a byte order mark, which the bytes keep.
    let mark = after.get(..after.len() - new_text.len()).unwrap_or_default();
    rewrite(layout, &new.source_map, new_function, (mark, new_text), &renames, &names)
}

/// The tokens of a text, comments and line endings left out.
fn lex<'t>(text: &'t str, names: &mut Interner, limits: &Limits) -> Option<Vec<Token<'t>>> {
    let mut report = Report::for_limits(limits);
    let mut tokens = Vec::new();
    for token in Lexer::new(text, names, &mut report) {
        let token = token.ok()?;
        if !matches!(token.kind, TokenKind::Comment(_) | TokenKind::Doc(_) | TokenKind::Newline) {
            tokens.push(token);
        }
    }
    Some(tokens)
}

impl<'t> Side<'t> {
    fn of(function: &Function, map: &SourceMap, tokens: Vec<Token<'t>>) -> Side<'t> {
        let mut side = Side { tokens, lines: HashMap::new() };
        let mut seen: HashMap<(scope::BodyPath, String), usize> = HashMap::new();
        for loc in scope::all(function) {
            let Some(statement) = scope::statement(function, &loc) else { continue };
            let Some(span) = map.span(statement.id) else { continue };
            // The tokens of its span, but for those of the lines of its bodies, which come in order and apart.
            let mut inside: Vec<(usize, usize)> = scope::subs(statement)
                .iter()
                .flat_map(|(body, _)| body.statements.iter().filter_map(|inner| map.span(inner.id)))
                .map(|span| side.range(span))
                .collect();
            inside.sort_unstable();
            let (start, end) = side.range(span);
            let mut key = Vec::new();
            let mut at = start;
            for (from, to) in inside {
                key.extend(at..from.clamp(at, end));
                at = at.max(to);
            }
            key.extend(at..end);
            let parent = loc.body.split_last().map(|(step, outer)| (Loc { body: outer.to_vec(), index: step.stmt }, step.sub));
            let tokens: Vec<TokenKind> = key.iter().filter_map(|index| side.tokens.get(*index).map(|token| token.kind)).collect();
            let alike = seen.entry((loc.body.clone(), format!("{tokens:?}"))).or_insert(0);
            let ordinal = *alike;
            *alike += 1;
            side.lines.insert(loc, Line { key, binds: binds(statement), parent, ordinal });
        }
        side
    }

    /// The tokens of a span, by their places.
    fn range(&self, span: Span) -> (usize, usize) {
        let start = self.tokens.partition_point(|token| token.span.start < span.start);
        let end = self.tokens.partition_point(|token| token.span.end <= span.end);
        (start, end.max(start))
    }

    /// The kind of name that the token at `index` is: a label after `#`, a field or a segment of a path after `.`, which
    /// stays as it is, a value otherwise.
    fn space(&self, index: usize) -> Option<Space> {
        match index.checked_sub(1).and_then(|previous| self.tokens.get(previous)).map(|token| token.kind) {
            Some(TokenKind::Hash) => Some(Space::Label),
            Some(TokenKind::Dot) => None,
            _ => Some(Space::Value),
        }
    }
}

/// The two texts, and the names that one lost and the other gained.
struct Pairing<'s, 't> {
    old: &'s Side<'t>,
    new: &'s Side<'t>,
    lost: HashSet<Named>,
    gained: HashSet<Named>,
}

impl<'s, 't> Pairing<'s, 't> {
    /// The renames that the pairs of lines give, once checked; `None` when there is none.
    fn pair(&self) -> Option<Renames> {
        let candidates = |side: &'s Side<'t>, changing: &HashSet<Named>| -> Vec<(&'s Loc, &'s Line)> {
            let mut found: Vec<(&Loc, &Line)> =
                side.lines.iter().filter(|(_, line)| line.binds.iter().any(|named| changing.contains(named))).collect();
            found.sort_by(|a, b| a.0.cmp(b.0));
            found
        };
        let (olds, news) = (candidates(self.old, &self.lost), candidates(self.new, &self.gained));
        let mut accepted: Vec<(Loc, Loc)> = Vec::new();
        let (mut used_old, mut used_new): (HashSet<&Loc>, HashSet<&Loc>) = (HashSet::new(), HashSet::new());
        let mut renames = Renames::default();
        // Each pass puts the lines left in buckets by their shape, the renames found so far written out, so that a line
        // that only the line before it tells apart finds its bucket alone at the next pass.
        for _ in 0..MAX_PASSES {
            let mut buckets: HashMap<u64, (Vec<&Loc>, Vec<&Loc>)> = HashMap::new();
            for (loc, line) in olds.iter().filter(|(loc, _)| !used_old.contains(*loc)) {
                buckets.entry(self.shape(self.old, line, &renames, true)).or_default().0.push(loc);
            }
            for (loc, line) in news.iter().filter(|(loc, _)| !used_new.contains(*loc)) {
                buckets.entry(self.shape(self.new, line, &renames, false)).or_default().1.push(loc);
            }
            let mut grew = false;
            let mut order: Vec<&(Vec<&Loc>, Vec<&Loc>)> = buckets.values().collect();
            order.sort_by(|a, b| (a.0.first(), a.1.first()).cmp(&(b.0.first(), b.1.first())));
            for (olds, news) in order {
                if olds.is_empty() || news.is_empty() || olds.len().saturating_mul(news.len()) > MAX_COMPARED {
                    continue;
                }
                let mut fits: Vec<(&Loc, &Loc)> = Vec::new();
                for old in olds {
                    for new in news {
                        if self.same(old, new, &renames, false, &mut Renames::default()) {
                            fits.push((old, new));
                        }
                    }
                }
                let (mut of_old, mut of_new): (HashMap<&Loc, usize>, HashMap<&Loc, usize>) = (HashMap::new(), HashMap::new());
                for (old, new) in &fits {
                    *of_old.entry(*old).or_default() += 1;
                    *of_new.entry(*new).or_default() += 1;
                }
                for (old, new) in fits {
                    if of_old.get(old) == Some(&1) && of_new.get(new) == Some(&1) {
                        accepted.push((old.clone(), new.clone()));
                        used_old.insert(old);
                        used_new.insert(new);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
            renames = self.renames_of(&accepted);
        }
        // Checked with every rename known: a pair whose line needs a rename that was not found goes, until none goes.
        loop {
            renames = self.renames_of(&accepted);
            let before = accepted.len();
            accepted.retain(|(old, new)| self.same(old, new, &renames, true, &mut Renames::default()));
            if accepted.len() == before {
                break;
            }
        }
        (!renames.to.is_empty()).then_some(renames)
    }

    /// What tells two lines apart before pairing them: a hash of their tokens, each name that a rename may change left out,
    /// unless it is one of `known`, written out as its new name. Two lines of one hash are still compared token by token.
    fn shape(&self, side: &Side, line: &Line, known: &Renames, old: bool) -> u64 {
        let mut hasher = DefaultHasher::new();
        for index in &line.key {
            let Some(token) = side.tokens.get(*index) else { continue };
            std::mem::discriminant(&token.kind).hash(&mut hasher);
            match (token.kind, side.space(*index)) {
                (TokenKind::Name(name), Some(space)) if old && self.lost.contains(&(space, name)) => {
                    known.get(space, name).hash(&mut hasher);
                }
                (TokenKind::Name(name), Some(space)) if !old && self.gained.contains(&(space, name)) => {
                    known.from.contains_key(&(space, name)).then_some(name).hash(&mut hasher);
                }
                (TokenKind::Name(name) | TokenKind::DegreeName(name), _) => name.hash(&mut hasher),
                (TokenKind::Keyword(keyword), _) => keyword.as_str().hash(&mut hasher),
                (
                    TokenKind::Int(text)
                    | TokenKind::Float(text)
                    | TokenKind::Based(text)
                    | TokenKind::String(text)
                    | TokenKind::Comment(text)
                    | TokenKind::Doc(text),
                    _,
                ) => text.hash(&mut hasher),
                _ => {}
            }
        }
        hasher.finish()
    }

    /// The renames that the accepted pairs give for the names their old lines bind. A name that two pairs would rename
    /// apart is renamed by none.
    fn renames_of(&self, accepted: &[(Loc, Loc)]) -> Renames {
        let mut found: HashMap<Named, HashSet<Name>> = HashMap::new();
        let mut back: HashMap<Named, HashSet<Name>> = HashMap::new();
        for (old, new) in accepted {
            let mut pairs = Renames::default();
            if !self.same(old, new, &Renames::default(), false, &mut pairs) {
                continue;
            }
            let Some(line) = self.old.lines.get(old) else { continue };
            for ((space, from), to) in &pairs.to {
                if line.binds.contains(&(*space, *from)) {
                    found.entry((*space, *from)).or_default().insert(*to);
                    back.entry((*space, *to)).or_default().insert(*from);
                }
            }
        }
        let mut renames = Renames::default();
        for ((space, from), tos) in found {
            if let [to] = tos.iter().copied().collect::<Vec<_>>()[..]
                && back.get(&(space, to)).is_some_and(|froms| froms.len() == 1)
            {
                renames.insert(space, from, to);
            }
        }
        renames
    }

    /// Whether the old line and the new one are the same, once renamed by `known`: when `strict`, every name that the
    /// write lost or gained must be one of `known`; otherwise the renames it needs beside them go in `more`.
    fn same(&self, old: &Loc, new: &Loc, known: &Renames, strict: bool, more: &mut Renames) -> bool {
        let (Some(a), Some(b)) = (self.old.lines.get(old), self.new.lines.get(new)) else { return false };
        if a.key.len() != b.key.len() {
            return false;
        }
        for (i, j) in a.key.iter().zip(&b.key) {
            let (Some(x), Some(y)) = (self.old.tokens.get(*i), self.new.tokens.get(*j)) else { return false };
            let space = self.old.space(*i);
            if space != self.new.space(*j) {
                return false;
            }
            let fits = match (x.kind, y.kind, space) {
                (TokenKind::Name(from), TokenKind::Name(to), Some(space)) => self.renamed(space, from, to, known, strict, more),
                (x, y, _) => x == y,
            };
            if !fits {
                return false;
            }
        }
        match (&a.parent, &b.parent) {
            (None, None) => true,
            (Some((old_parent, old_sub)), Some((new_parent, new_sub))) => {
                let place = |side: &Side, loc: &Loc| side.lines.get(loc).map(|line| line.ordinal);
                old_sub == new_sub
                    && place(self.old, old_parent) == place(self.new, new_parent)
                    && self.same(old_parent, new_parent, known, strict, more)
            }
            _ => false,
        }
    }

    /// Whether the name `from` of the old text may be `to` of the new one.
    fn renamed(&self, space: Space, from: Name, to: Name, known: &Renames, strict: bool, more: &mut Renames) -> bool {
        let (lost, gained) = (self.lost.contains(&(space, from)), self.gained.contains(&(space, to)));
        if from == to {
            return !lost && !gained;
        }
        if !lost || !gained {
            return false;
        }
        if known.get(space, from) == Some(to) {
            return true;
        }
        if strict || !known.allows(space, from, to) || !more.allows(space, from, to) {
            return false;
        }
        more.insert(space, from, to);
        true
    }
}

/// The new text, with the names of the `layout` entries that the renames move, and the renames that moved one. A rename
/// to a name that the section already names moves nothing: those entries win. The bytes start with `mark`, which the
/// text does not hold.
fn rewrite(
    layout: &Layout,
    map: &SourceMap,
    function: &Function,
    (mark, text): (&[u8], &str),
    renames: &Renames,
    names: &Interner,
) -> Option<Followed> {
    let mut named: HashSet<Named> = HashSet::new();
    let mut refs: Vec<(NodeRef, Option<(labcraft_graph::Id, Name)>)> = Vec::new();
    for item in &layout.items {
        match &item.kind {
            LayoutKind::Frame { node: Some(node), .. } | LayoutKind::Node { node, .. } => refs.push((*node, None)),
            LayoutKind::Wire { from, to, .. } => {
                refs.push((from.node, None));
                refs.push((to.node, to.terminal.map(|terminal| (to.id, terminal))));
            }
            LayoutKind::Frame { node: None, .. } | LayoutKind::Terminal { .. } => {}
        }
    }
    // The structures of the new text, by the names that designate them: an input of one is named by a carry, an `in`
    // clause or an outer value, which a rename changes, where an input of a call is named by its parameter.
    let structures: HashSet<Named> = scope::all(function)
        .into_iter()
        .filter_map(|loc| scope::statement(function, &loc))
        .filter(|statement| {
            matches!(&statement.kind, StatementKind::Binding { value: Rhs::For(_) | Rhs::Loop(_) | Rhs::Match(_) | Rhs::Handle(_), .. })
        })
        .flat_map(|statement| {
            let label = statement.label.map(|label| (Space::Label, label.name));
            label.into_iter().chain(scope::line_name(statement).map(|name| (Space::Value, name)))
        })
        .collect();
    // What the section names already: its nodes, and the inputs of structures that it names by a value.
    let designated = |node: &NodeRef| renames.get(space_of(node), node.name).unwrap_or(node.name);
    for (node, terminal) in &refs {
        named.insert((space_of(node), node.name));
        if let Some((_, terminal)) = terminal
            && structures.contains(&(space_of(node), designated(node)))
        {
            named.insert((Space::Value, *terminal));
        }
    }
    let moves = |space: Space, name: Name| renames.get(space, name).filter(|to| !named.contains(&(space, *to)));
    let mut cursor = Cursor::new(text);
    let mut edits: Vec<(usize, usize, Name)> = Vec::new();
    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut moved = |space: Space, from: Name, to: Name| {
        let mark = if space == Space::Label { "#" } else { "" };
        renamed.push((format!("{mark}{}", names.resolve(from)), format!("{mark}{}", names.resolve(to))));
    };
    for (node, terminal) in &refs {
        let space = space_of(node);
        if let Some(to) = moves(space, node.name)
            && let Some(edit) = name_at_end(&mut cursor, map.span(node.id), names.resolve(node.name), to)
        {
            edits.push(edit);
            moved(space, node.name, to);
        }
        if let Some((id, name)) = terminal
            && structures.contains(&(space, designated(node)))
            && let Some(to) = moves(Space::Value, *name)
            && let Some(edit) = name_at_end(&mut cursor, map.span(*id), names.resolve(*name), to)
        {
            edits.push(edit);
            moved(Space::Value, *name, to);
        }
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(start, _, _)| *start);
    edits.dedup_by_key(|(start, _, _)| *start);
    let mut out = Vec::with_capacity(mark.len() + text.len());
    out.extend_from_slice(mark);
    let mut at = 0;
    for (start, end, to) in edits {
        out.extend_from_slice(text.get(at..start)?.as_bytes());
        out.extend_from_slice(names.resolve(to).as_bytes());
        at = end;
    }
    out.extend_from_slice(text.get(at..)?.as_bytes());
    renamed.sort();
    renamed.dedup();
    Some(Followed { bytes: out, renamed })
}

fn space_of(node: &NodeRef) -> Space {
    if node.is_label { Space::Label } else { Space::Value }
}

/// The bytes of the name `old` that end the span, to be written `to`: `None` when the text there is not that name.
fn name_at_end(cursor: &mut Cursor, span: Option<Span>, old: &str, to: Name) -> Option<(usize, usize, Name)> {
    let span = span?;
    let end = cursor.offset(span.end)?;
    let start = end.checked_sub(old.len())?;
    (cursor.text.get(start..end)? == old).then_some((start, end, to))
}

/// From a position of the text, by its line and its column in characters, to its byte.
struct Cursor<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Cursor<'a> {
        let mut starts = vec![0usize];
        starts.extend(text.char_indices().filter(|(_, c)| *c == '\n').map(|(at, _)| at + 1));
        Cursor { text, starts }
    }

    fn offset(&mut self, position: Position) -> Option<usize> {
        let start = *self.starts.get(usize::try_from(position.line).ok()?.checked_sub(1)?)?;
        let column = usize::try_from(position.column).ok()?.checked_sub(1)?;
        let rest = self.text.get(start..)?;
        match rest.char_indices().nth(column) {
            Some((at, _)) => Some(start + at),
            None => (rest.chars().count() == column).then_some(self.text.len()),
        }
    }
}

#[cfg(test)]
mod tests;
