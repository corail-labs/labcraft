"""Layout of the text (spec 5.2, "Layout of the text" and section 2): `lcf.py fmt --check`.

A lexical check of the source text of a `.lcf` or `.lct` file. It is aware of string literals and
comments (a string is masked before the rules apply, a comment is cut off). Widths are counted in
code points, on the code of the line (an end-of-line comment is not counted). It checks:

- file: LF line endings (no CR LF, no lone CR), no tab, no space at the end of a line, exactly one
  line feed at the end of the file;
- sections: header, `use` lines, comment block + declaration, `panel`, `layout`, `icon` are
  separated by exactly one blank line (none between two `use` lines, none between a comment block
  and what follows it, none between an `extern` line and its `fn` line); no blank line inside a
  body or a section; `use` lines one per line, sorted by code point, each library once, no comment
  between them;
- indentation: 4 spaces per level (the level is the nesting of braces, parentheses and brackets;
  one more level for the clauses of a split `loop` or `for`, whose `{` is alone at the level of
  the statement);
- spaces: one after a comma, a `;` and a `:`, none before; one on each side of `=`, `=>`, `->`;
  one inside braces written on one line (none in `{}`), none just inside parentheses, brackets and
  angle brackets, no trailing comma in a one-line form; none around `..`, `..=` and `.`, after
  `#`, before `...`, inside a unit (`m/s^2`); one between a number and its unit (`2 s`; literals
  keep their spelling, `1e3` is fine);
- braces of a body: `{` ends its line, `}` stands alone, an empty body is `{}`, an arm that is only
  a `yield` is one line, a `match` or a `handle` is never on one line; a brace left open or closed
  in the middle of a line (a partial split) is reported;
- none between a name and a following `(`, `[` or `<` (`layout` and `icon` words excepted), one after a keyword or a number (`yield (a, b)`,
  `code -1 (err)`), and one before `(` after a word of the `layout` and `icon` sections;
- no alignment: two consecutive spaces nowhere but in the indentation, strings and comments;
- a statement inside a body is not wrapped over several lines (a parenthesis or bracket left open
  at the end of a line, a struct construction `r = S {` that ends its line);
- comments: `//` and `///` followed by at least one space, or by nothing when empty; one space
  before an end-of-line comment;
- line width, 100 code points: a signature (parameters one per line and `) -> (outputs) {` alone
  on the last line; if that line is too long, or the outputs have a comment, `) -> (` ends a line,
  one output per line, then `) {`; a function without parameters keeps `fn name() -> (` on its
  first line), the fields of a struct and the variants of an enum are written one item per line,
  each followed by a comma, if and only if their one-line form is longer than 100 code points or
  has a comment between the brackets or braces; the clauses of a `loop` or a `for` are written one
  per line (the keyword ends the line, commas between the clauses, none after the last, then `{`
  alone) if and only if the one-line form is longer than 100 code points or has a comment before
  the `{`. Every other statement is one line, whatever its length.

- names and units in their one spelling (R-20): NFC, no Greek mu, outside strings and comments;
- items of `panel` and `layout`: their order, plain decimal coordinates, no empty section.

Not checked (not lexical, or not worth it): the order of the sections, the order of the lines (see
`canon.py`), spaces around `<` `>` when used as operators (there are none in the language), the
width of `extern` lines, the spelling of literals, layout entries that name no node.

`check_layout` only reports; it never rewrites. `check_spec_layout` applies it to the code blocks
of the spec (see its docstring).
"""
import re
import unicodedata

WIDTH = 100
STRUCT_OPEN = re.compile(r"\s=\s(?:loop(?: continue_on_error)?|for(?: parallel)?)$")
STRUCT_ONE = re.compile(r"\s=\s(?:loop|for)\b.*\{$")
CONSTRUCT = re.compile(r"^(?:#\w+: )?(?:next )?\w+(?:, (?:next )?\w+)* = ((?:\w+\.)*\w+) \{$")
LOOP_STMT = re.compile(r"\s=\s(?:loop|for)(?!\w)")
CLAUSE = re.compile(r"(?:carry(?!\w)|\w+ in(?!\w))")
MATCH_STMT = re.compile(r"(?:^|=\s)(?:match|handle)\s")
NO_SPACE_AFTER = re.compile(r"(?<![\w.])(?:yield|in|until|collect|match|for|handle"
                            r"|-?\d[\d_]*(?:\.\d[\d_]*)?(?:[eE]-?\d[\d_]*)?)(?=[(\[<])")
OPERATOR = re.compile(r"->|=>|\.\.=|[<>!=]=|=")
NUMBER_UNIT = re.compile(r"(?<![\w.])(?>\d[\d_]*(?:\.\d[\d_]*)?(?:[eE]-?\d[\d_]*)?)(?=[^\W\d_]|°)")
NAME_BRACKET = re.compile(r"(?<![\w.])([\w.]+)[ ]+([(\[<])")
KEYWORDS = {"yield", "in", "until", "collect", "at", "via", "return", "match", "for", "handle"}
SECTIONS = ("panel", "layout", "icon")
KINDS = {"labcraft": "header", "use": "use", "fn": "decl", "type": "decl", "extern": "extern",
         "panel": "section", "layout": "section", "icon": "section"}


def _lex(raw):
    """(code, comment): `code` keeps the length of the line up to the comment, with the inside of
    each string literal masked by `x`; `comment` starts at `//`, or is None."""
    out, i, quote = [], 0, False
    while i < len(raw):
        c = raw[i]
        if quote:
            if c == "\\" and i + 1 < len(raw):
                out.append("xx")
                i += 2
                continue
            if c == '"':
                quote = False
                out.append(c)
            else:
                out.append("x")
        elif c == '"':
            quote = True
            out.append(c)
        elif raw.startswith("//", i):
            return "".join(out), raw[i:]
        else:
            out.append(c)
        i += 1
    return "".join(out), None


class Ln:
    def __init__(self, no, raw):
        self.no, self.raw = no, raw
        self.blank = not raw.strip(" \t")
        self.code, self.comment = _lex(raw)
        self.indent = re.match(r"[ \t]*", raw).group()
        self.body = self.code[len(self.indent):].strip(" \t")  # masked code without indentation
        self.text = raw[len(self.indent):len(self.code)].strip(" \t")  # same, strings unmasked
        self.depth = self.level = 0
        self.sect = False
        self.sname = ""  # name of the section (`panel`, `layout`, `icon`) the line belongs to
        self.arm_yield = False  # `pattern => {` whose body is only a `yield`
        self.next_code = self.next_stop = None  # index of the next line with code / with code or blank


def _count(s, chars):
    return sum(s.count(c) for c in chars)


def _levels(lines):
    """Nesting depth at the start of each line, and the indentation level it should have."""
    depth, split, sect, sname = 0, False, False, ""
    for ln in lines:
        ln.depth = depth
        b = ln.body
        if not b:
            ln.sect, ln.sname = sect, sname
            continue
        if depth == 0:
            sect = b.split(" ")[0] in SECTIONS
            sname = b.split(" ")[0] if sect else ""
        ln.sect, ln.sname = sect, sname
        ln.level = max(depth - (len(b) - len(b.lstrip(")]}"))), 0)
        if split:
            if b == "{":
                split = False
            elif b.endswith("{"):
                split = False
            else:
                ln.level = depth + 1
        elif STRUCT_OPEN.search(b):
            split = True
        depth += _count(b, "([{") - _count(b, ")]}")


def _top_commas(s):
    depth = n = 0
    for c in s:
        depth += (c in "([{") - (c in ")]}")
        n += c == "," and depth == 0
    return n


def _join(parts):
    """One-line form of the lines of a split construct."""
    out = ""
    for p in parts:
        if out and p[0] in ")]}" and out.endswith(","):
            out = out[:-1]
        if out and not out.endswith("(") and not p.startswith(")") and not (out.endswith("{") and p.startswith("}")):
            out += " "
        out += p
    return out


def _width(ln):
    return len(ln.indent) + len(ln.text)


def _file_rules(src, out):
    norm = src.replace("\r\n", "\n")
    if "\r\n" in src:
        out(src[:src.index("\r\n")].count("\n") + 1, "line ending is CR LF: LF only (spec 2)")
    m = re.search(r"\r(?!\n)", src)
    if m:
        out(src[:m.start()].count("\n") + 1, "a CR that is not followed by a line feed: LF only (spec 2)")
    if norm and not norm.endswith("\n"):
        out(norm.count("\n") + 1, "the file does not end with a line feed (spec 2)")
    elif norm.endswith("\n\n"):
        out(norm.rstrip("\n").count("\n") + 2, "the file ends with more than one line feed (spec 2)")


def _blank_rules(L, out):
    prev, blanks = None, 0
    uses = []
    for idx, ln in enumerate(L):
        if ln.blank:
            if ln.depth > 0:
                out(ln.no, "blank line inside a body or a section")
            elif prev is None:
                out(ln.no, "blank line before the header")
            blanks += 1
            continue
        if ln.depth == 0:
            kind = "comment" if not ln.body else KINDS.get(ln.body.split(" ")[0], "other")
            if prev is not None and kind != "other":
                want = 0 if prev in ("comment", "extern") or (prev == "use" and kind == "use") else 1
                if blanks > 1:
                    out(ln.no, "more than one blank line: sections are separated by exactly one")
                elif blanks != want:
                    if prev == "extern":
                        what = "an `extern` line is followed directly by its `fn` line, with no blank line"
                    elif want == 0:
                        what = "no blank line here"
                    elif kind == "comment" and prev == "use" and _use_follows(L, idx):
                        what = "no comment between `use` lines: they are consecutive"
                    else:
                        what = "sections are separated by one blank line"
                    out(ln.no, what + " (5.2)")
                elif kind == "comment" and prev == "use" and _use_follows(L, idx):
                    out(ln.no, "no comment between `use` lines: they are consecutive (5.2)")
            if kind == "use":
                uses.append(ln)
            elif kind != "comment":
                _use_rules(uses, out)
                uses = []
            if kind != "other" or prev is None:
                prev = kind
        blanks = 0
    _use_rules(uses, out)


def _use_follows(L, idx):
    """True if the next line with code at depth 0 after L[idx] is a `use` line."""
    j = L[idx].next_stop
    return j is not None and bool(L[j].body) and L[j].depth == 0 and L[j].body.split(" ")[0] == "use"


def _use_rules(uses, out):
    names, seen = [], set()
    for ln in uses:
        if not re.fullmatch(r"use [^\s,]+", ln.body):
            out(ln.no, "one `use` per line, with one path: `use name`")
        elif ln.body in seen:
            out(ln.no, f"`{ln.body}` appears twice: each library once")
        seen.add(ln.body)
        names.append(ln.body)
    want = sorted(names)
    for ln, n, w in zip(uses, names, want):
        if n != w:
            out(ln.no, f"`use` lines are not sorted (code point order): expected `{w}` here, found `{n}`")
            break


def _indent_rules(L, out):
    for k, ln in enumerate(L):
        if ln.blank:
            continue
        if "\t" in ln.indent:
            out(ln.no, "tab in the indentation: 4 spaces per level")
            continue
        levels = {ln.level}
        if not ln.body:  # comment line: it takes the level of the statement that follows
            if ln.next_code is None:
                continue
            nxt = L[ln.next_code]
            if nxt.body[0] == "}":  # end of a body: the comment is at the level of the body
                levels = {nxt.level + 1}
            else:
                levels = {nxt.level} | ({nxt.level + 1} if nxt.body[0] in ")]" else set())
        if len(ln.indent) not in {4 * n for n in levels}:
            want = 4 * min(levels)
            out(ln.no, f"indentation is {len(ln.indent)} spaces, expected {want} (4 per level)")


def _brace_rules(ln, out):
    """Braces of a body (spec 5.2): `{` ends its line, `}` stands alone, an arm that is only a
    `yield` is one line, a `match` or a `handle` is never on one line."""
    b, n = ln.body, ln.no
    if not b:
        return
    opens, closes = b.count("{"), b.count("}")
    if opens > closes and not b.endswith("{"):
        out(n, "a `{` that opens a body, a struct or a list ends its line: no item after it (5.2)")
    if closes > opens and not b.startswith("}"):
        out(n, "a `}` stands alone on its line: no item before it (5.2)")
    if MATCH_STMT.search(b) and not b.endswith("{"):
        out(n, "a `match` or a `handle` is never written on one line: `{` ends its line (5.2)")
    if re.search(r"=>\s*\{(?!$)", b):
        out(n, "the body of an arm is in braces on their own lines, or the arm is `pattern => yield …` (5.2)")
    if b.startswith("fn "):
        depth = 0
        for p, c in enumerate(b):
            depth += (c in "([") - (c in ")]")
            if c == "{" and depth == 0:
                rest = b[p + 1:].strip()
                if rest and rest != "}":
                    out(n, "a function body is in braces on their own lines; only an empty body is `{}` (5.2)")
                break
    if LOOP_STMT.search(b) and not b.endswith("{") and not b.endswith("{}") and not STRUCT_OPEN.search(b):
        out(n, "a `loop` or a `for` line ends with `{` (clauses on the line), or with the keyword "
               "(one clause per line after it, then `{` alone) (5.2)")
    if ln.arm_yield:
        out(n, "an arm whose body is only a `yield` is written on one line: `pattern => yield …` (5.2)")


def _space_rules(ln, out):
    b = ln.body
    if not b:
        return
    n = ln.no
    if ln.raw != ln.raw.rstrip(" \t"):
        out(n, "space at the end of the line")
    if "\t" in b:
        out(n, "tab: use spaces")
    if ln.depth >= 1 and _count(b, "([") > _count(b, ")]"):
        out(n, "a statement stays on one line, whatever its length (5.1)")
    m = CONSTRUCT.match(b)
    if ln.depth >= 1 and not ln.sect and m and m.group(1) != "loop":
        out(n, "a struct construction stays on one line, whatever its length (5.2)")
    _brace_rules(ln, out)
    if "  " in b:
        out(n, "two consecutive spaces: no alignment (5.2)")
    for m in re.finditer(r"\s,", b):
        out(n, "space before a comma")
    for m in re.finditer(r",(?=\S)", b):
        out(n, "one space is needed after a comma")
    for m in re.finditer(r",\s*[)\]}]", b):
        out(n, f"no comma before `{m.group()[-1]}` in a form written on one line")
    for m in re.finditer(r"\s;", b):
        out(n, "space before `;`")
    for m in re.finditer(r";(?=\S)", b):
        out(n, "one space is needed after `;`")
    for m in re.finditer(r"\s:", b):
        out(n, "space before `:`")
    for m in re.finditer(r":(?=\S)", b):
        out(n, "one space is needed after `:`")
    for m in OPERATOR.finditer(b):
        t, a, z = m.group(), m.start(), m.end()
        if t in ("->", "=>", "=") and (a == 0 or b[a - 1] != " " or z >= len(b) or b[z] != " "):
            out(n, f"one space is needed on each side of `{t}`")
    for m in re.finditer(r"\{(?=[^\s}])", b):
        out(n, "one space is needed after `{` in braces written on one line")
    for m in re.finditer(r"(?<=[^\s{])\}", b):
        out(n, "one space is needed before `}` in braces written on one line")
    for m in re.finditer(r"\{\s+\}", b):
        out(n, "empty braces are written `{}`")
    for m in re.finditer(r"(?<=\S)\{", b):
        out(n, "one space is needed before `{`")
    for m in re.finditer(r"[(\[<] ", b):
        out(n, f"space after `{m.group()[0]}`")
    for m in re.finditer(r"\s[)\]>]", b):
        out(n, f"space before `{m.group()[1]}`")
    if ln.sname in ("layout", "icon"):
        for m in re.finditer(r"[\w.](?=\()|\)(?=\()", b):
            out(n, "one space is needed before `(` after a word of the `layout` and `icon` sections")
    else:
        for m in NAME_BRACKET.finditer(b):
            if m.group(1) not in KEYWORDS and not m.group(1)[0].isdigit():
                out(n, f"no space between `{m.group(1)}` and `{m.group(2)}`")
        for m in NO_SPACE_AFTER.finditer(b):
            out(n, f"one space is needed between `{m.group()}` and `{b[m.end()]}`")
    for m in re.finditer(r"-\s", b):
        out(n, "no space after a unary minus (`-1`)")
    for m in re.finditer(r"\s\.\.\.", b):
        out(n, "no space before `...`")
    for m in re.finditer(r"(?<!\.)\.\.=?(?!\.)", b):
        if (m.start() > 0 and b[m.start() - 1] == " ") or (m.end() < len(b) and b[m.end()] == " "):
            out(n, "no space around `..` and `..=`")
    for m in re.finditer(r"(?<=\s)\.(?!\.)|(?<!\.)\.(?!\.)(?=\s)", b):
        out(n, "no space around `.`")
    for m in re.finditer(r"#\s", b):
        out(n, "no space after `#`")
    for m in re.finditer(r"\s[/*^·]|[/*^·]\s", b.replace(".*", "..")):  # `visa.* =>`: a category
        out(n, "no space inside a unit (`m/s^2`)")
    for m in NUMBER_UNIT.finditer(b):
        out(n, f"one space is needed between the number `{m.group()}` and its unit")


def _comment_rules(ln, out):
    if ln.raw != ln.raw.rstrip(" \t") and not ln.body:
        out(ln.no, "space at the end of the line")
    c = ln.comment
    if c is None:
        return
    rest = c[3:] if c.startswith("///") else c[2:]
    if rest and not rest.startswith(" "):
        out(ln.no, "a comment starts with `//` or `///` followed by at least one space (or by nothing)")
    if ln.body and ln.code[len(ln.code.rstrip(" \t")):] != " ":
        out(ln.no, "an end-of-line comment is preceded by exactly one space")


def _check_items(items, out, what):
    for ln in items:
        if not ln.body:
            continue
        if not ln.body.endswith(","):
            out(ln.no, f"each {what} of a split form ends with a comma")
        elif _top_commas(ln.body[:-1]):
            out(ln.no, f"one {what} per line")


def _sig_end(L, i):
    """Index of the line where the parentheses opened from line i are closed again."""
    depth = 0
    for j in range(i, len(L)):
        depth += _count(L[j].body, "([") - _count(L[j].body, ")]")
        if depth <= 0:
            return j
    return None


def _check_fn(L, i, out):
    j = _sig_end(L, i)
    if j is None:
        return
    hdr = L[i:j + 1]
    last = hdr[-1]
    commented = any(l.comment is not None for l in hdr[:-1])
    one = _join([l.text for l in hdr if l.body])
    zero = re.match(r"fn .*\(\) -> \($", hdr[0].body) is not None
    if j == i:
        if len(one) > WIDTH:
            if re.match(r"fn [^(]*\(\)", hdr[0].body):
                out(hdr[0].no, f"signature is {len(one)} characters, over {WIDTH}: `fn name() -> (` "
                               "ends the first line, one output per line, then `) {`")
            else:
                out(hdr[0].no, f"signature is {len(one)} characters, over {WIDTH}: one parameter "
                               "per line, `) -> (outputs) {` on its own line")
        return
    if not commented and len(one) <= WIDTH:
        out(hdr[0].no, f"signature fits on one line ({len(one)} characters): expected `{one}`")
        return
    if zero:
        if last.body not in (") {", ")"):
            out(last.no, "a split output list ends with `) {`")
        _check_items(hdr[1:-1], out, "output")
        return
    if re.search(r"\) -> \($", hdr[0].body):
        out(hdr[0].no, "outputs are split only when the parameters are: `fn name(` ends the first line")
        return
    if not hdr[0].body.endswith("("):
        out(hdr[0].no, "a split signature ends its first line with `(`, one parameter per line")
        return
    if hdr[1].body.startswith(")"):
        out(hdr[0].no, "a function without parameters keeps `fn name() -> (` on its first line")
        return
    k = next((n for n, l in enumerate(hdr) if n and l.body.startswith(")")), None)
    if k is None:
        return
    _check_items(hdr[1:k], out, "parameter")
    tail = hdr[k:]
    if len(tail) == 1:
        if _width(tail[0]) > WIDTH:
            out(tail[0].no, f"`) -> (outputs) {{` is {_width(tail[0])} characters, over {WIDTH}: "
                            "`) -> (` ends the line, one output per line, then `) {`")
        return
    if tail[0].body != ") -> (":
        out(tail[0].no, "a split output list is `) -> (` alone on its line, one output per line")
        return
    if tail[-1].body not in (") {", ")"):
        out(tail[-1].no, "a split output list ends with `) {`")
    one = _join([l.text for l in tail if l.body])
    if not any(l.comment is not None for l in tail[:-1]) and len(one) <= WIDTH:
        out(tail[0].no, f"the outputs fit on one line ({len(one)} characters): expected `{one}`")
    _check_items(tail[1:-1], out, "output")


def _check_type(L, i, out):
    first = L[i]
    if not first.body.endswith("{"):
        if _width(first) > WIDTH and "{" in first.body:
            out(first.no, f"declaration is {_width(first)} characters, over {WIDTH}: "
                          "one field or variant per line")
        return
    k = next((n for n in range(i + 1, len(L))
              if L[n].depth == first.depth + 1 and L[n].body.startswith("}")), None)
    if k is None:
        return
    block = L[i:k + 1]
    one = _join([l.text for l in block if l.body])
    if not any(l.comment is not None for l in block[:-1]) and len(one) <= WIDTH:
        out(first.no, f"declaration fits on one line ({len(one)} characters): expected `{one}`")
    _check_items(block[1:-1], out, "field or variant")


def _check_loop(L, i, out):
    ln = L[i]
    if not STRUCT_OPEN.search(ln.body):
        if STRUCT_ONE.search(ln.body) and _width(ln) > WIDTH:
            out(ln.no, f"line is {_width(ln)} characters, over {WIDTH}: the keyword `loop` or `for` ends the "
                       "line, then one clause per line, then `{` alone")
        return
    j, clauses = i + 1, []
    while j < len(L) and L[j].body != "{":
        if L[j].body:
            if not CLAUSE.match(L[j].body):  # not a clause: stop here (keeps the scan linear)
                break
            clauses.append(L[j])
        j += 1
    if j >= len(L) or L[j].body != "{":
        out((clauses[-1] if clauses else ln).no,
            "`{` alone on the line after the last clause, at the level of the statement")
        return
    for c in clauses[:-1]:
        if not c.body.endswith(","):
            out(c.no, "the clauses of a split loop are separated by commas")
    if clauses and clauses[-1].body.endswith(","):
        out(clauses[-1].no, "no comma after the last clause")
    for c in clauses:
        if _top_commas(c.body[:-1] if c.body.endswith(",") else c.body):
            out(c.no, "one clause per line")
    one = " " * (4 * ln.level) + ln.text + " " + ", ".join(c.text.rstrip(",") for c in clauses) + " {"
    one = one.replace("  {", " {") if not clauses else one
    if len(one) <= WIDTH and not any(l.comment is not None for l in L[i:j]):
        out(ln.no, f"the clauses fit on one line ({len(one)} characters): expected `{one.strip()}`")


def _groups(s):
    """Contents of the top-level parenthesized groups of `s`."""
    out, depth, start = [], 0, 0
    for i, c in enumerate(s):
        if c == "(":
            depth += 1
            if depth == 1:
                start = i + 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                out.append(s[start:i])
    return out


def _names(group):
    """Names of the parameters or outputs of a signature group."""
    out, depth, cur = [], 0, ""
    for c in group + ",":
        depth += (c in "([{") - (c in ")]}")
        if c == "," and depth == 0:
            if cur.strip():
                out.append(cur.split(":")[0].strip())
            cur = ""
        else:
            cur += c
    return out


def _signature(L):
    """(inputs, outputs) of the function of the file, from its text."""
    for i, ln in enumerate(L):
        if ln.depth == 0 and ln.body.startswith("fn "):
            j = _sig_end(L, i)
            if j is None:
                break
            groups = _groups(" ".join(l.body for l in L[i:j + 1]))
            if len(groups) >= 2:
                return _names(groups[0]), _names(groups[1])
            break
    return [], []


def _first_unordered(items, out, what):
    """Report the first of `items` [(key, line, text)] that is not where a stable sort puts it."""
    want = sorted(items, key=lambda it: it[0])
    for got, exp in zip(items, want):
        if got is not exp:
            out(got[1], f"items of `{what}` are not in canonical order (5.2): expected `{exp[2]}` "
                        f"here, found `{got[2]}`")
            return


def _item_rules(L, out):
    """Items of `panel` and `layout` (spec 5.2): order, plain coordinates, no empty section."""
    inputs, outputs = _signature(L)
    panel, layout = [], []
    for k, ln in enumerate(L):
        if ln.depth == 0 and ln.sname in ("panel", "layout") and ln.body:
            before = k > 0 and L[k - 1].comment is not None and not L[k - 1].body
            closed = ln.body == ln.sname + " {}"
            wrapped = (ln.body == ln.sname + " {" and k + 1 < len(L) and L[k + 1].body == "}"
                       and L[k + 1].comment is None)
            if (closed or wrapped) and ln.comment is None and not before:
                out(ln.no, f"an empty `{ln.sname}` section is removed (5.2)")
        if ln.depth != 1 or not ln.body:
            continue
        if ln.sname == "panel":
            m = re.match(r"(control|indicator) ([^\s:]+)\s*:", ln.body)
            if m:
                kind, name = m.groups()
                terminals = inputs if kind == "control" else outputs
                rank = (0, terminals.index(name), "") if name in terminals else (1, 0, name)
                panel.append(((kind != "control", *rank), ln.no, ln.text))
        elif ln.sname == "layout":
            m = re.match(r"node (\S+)", ln.body)
            w = re.match(r"wire (\S+) -> (\S+)", ln.body)
            if m:
                layout.append(((0, m.group(1), ""), ln.no, ln.text))
            elif w:
                layout.append(((1, w.group(1), w.group(2)), ln.no, ln.text))
            for point in re.findall(r"\(([^()]*)\)", ln.body):
                for x in point.split(","):
                    x = x.strip()
                    if re.fullmatch(r"-?[0-9][0-9_]*", x):
                        value = int(x.replace("_", ""))
                        if abs(value) <= 1_000_000 and x != str(value):
                            out(ln.no, f"a coordinate is written in plain decimal: `{value}`, not `{x}` (5.2)")
    _first_unordered(panel, out, "panel")
    _first_unordered(layout, out, "layout")


def _unicode_rules(ln, out):
    """Names and units in their one spelling (R-20): `code` has strings masked, comments cut."""
    if "\u03bc" in ln.code or unicodedata.normalize("NFC", ln.code) != ln.code:
        out(ln.no, "a name or a unit is not in its one spelling (R-20): NFC, and U+00B5 for the micro sign")


def _links(L):
    """One reverse pass: next line with code, next line with code or blank, arms made of a `yield`."""
    nc = ns = None
    for k in range(len(L) - 1, -1, -1):
        ln = L[k]
        ln.next_code, ln.next_stop = nc, ns
        if ln.body:
            nc = ns = k
        elif ln.blank:
            ns = k
    for ln in L:
        if ln.body.endswith("=> {") and ln.next_code is not None:
            a = L[ln.next_code]
            if a.body.startswith("yield") and a.next_code is not None and L[a.next_code].body == "}":
                ln.arm_yield = True


def check_layout(src):
    """[(line, message)] for each rule of the layout of the text that `src` breaks."""
    problems = set()

    def out(line, msg):
        problems.add((line if isinstance(line, int) else line.no, "layout (5.2): " + msg))

    _file_rules(src, out)
    work = src.replace("\r\n", "\n").replace("\r", "")
    L = [Ln(n, raw) for n, raw in enumerate(work.split("\n")[:-1] if work.endswith("\n")
                                             else work.split("\n"), 1)]
    _levels(L)
    _links(L)
    _blank_rules(L, out)
    _indent_rules(L, out)
    for ln in L:
        _space_rules(ln, out)
        _comment_rules(ln, out)
        _unicode_rules(ln, out)
    _item_rules(L, out)
    for i, ln in enumerate(L):
        if ln.depth == 0 and ln.body.startswith("fn "):
            _check_fn(L, i, out)
        elif ln.depth == 0 and ln.body.startswith("type "):
            _check_type(L, i, out)
        _check_loop(L, i, out)
    return sorted(problems)


# ---- code blocks of the spec -------------------------------------------------------------------

def spec_blocks(path):
    """([(line of the first line of the block, [lines])], number skipped): the ```text blocks of the spec.

    Same selection as `spec_units` of tools/lcf-view/lcf_view.py (blocks with `…` are skipped), and
    the reference listings are skipped too: the block that starts with `[f64]` (3.2) and every
    block after the heading `## Appendix A`. A fence that is indented (in a list) dedents its block.
    """
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    blocks, cur, appendix, skipped = [], None, False, 0
    for i, l in enumerate(lines, 1):
        s = l.strip()
        if cur is None and s.startswith("## Appendix A"):
            appendix = True
        if cur is None and s.startswith("```"):
            cur, lang, start, ind = [], s[3:].strip(), i + 1, len(l) - len(l.lstrip(" "))
        elif cur is not None and s == "```":
            first = next((x.strip() for x in cur if x.strip()), "")
            text = "\n".join(cur)
            if lang in ("", "text") and not appendix:
                if "…" in text or first.startswith("[f64]"):
                    skipped += 1
                else:
                    blocks.append((start, cur))
            cur = None
        elif cur is not None:
            cur.append(l[ind:] if l[:ind].strip() == "" else l)
    return blocks, skipped


def _wrap(body):
    """(source text, number of lines added before the block) that lets the layout rules apply to
    a block of the spec that is not a whole file, or None if the block is not recognized."""
    first = next((l.strip() for l in body if l.strip() and not l.strip().startswith("//")), "")
    text = "\n".join(body) + "\n"
    word = first.split(" ")[0]
    if re.match(r"^labcraft \d+$", first):
        return text, 0
    if word in ("fn", "type", "extern"):
        return "labcraft 1\n\n" + text, 2
    if word in SECTIONS:
        return "labcraft 1\n\nfn spec_check() -> () {}\n\n" + text, 4
    if " = " in first or first.startswith("#") or re.match(r"^[\w.]+\(", first):
        indented = "".join(("    " + l if l.strip() else l) + "\n" for l in body)
        return "labcraft 1\n\nfn spec_check() -> () {\n" + indented + "}\n", 3
    return None


def check_spec_layout(path):
    """([(line in the markdown file, message)], blocks checked, blocks skipped).

    Whole files (blocks that start with `labcraft 1`) are checked like a file. Other blocks are
    completed so that the same rules apply without complaints about a missing header or section:
    `fn` and `type` blocks get a header, statements are placed in a body, `panel`, `layout` and
    `icon` blocks follow an empty function. A block that cannot be recognized is skipped.
    """
    problems, done = set(), 0
    blocks, skipped = spec_blocks(path)
    for start, body in blocks:
        w = _wrap(body)
        if w is None:
            skipped += 1
            continue
        src, added = w
        done += 1
        last = start + len(body) - 1
        for line, msg in check_layout(src):
            problems.add((min(max(line - added, 1) + start - 1, max(last, start)), msg))
    return sorted(problems), done, skipped
