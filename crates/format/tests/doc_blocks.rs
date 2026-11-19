//! The code blocks of the spec and of the skills (AGENTS.md): each block that is not an illustration (it holds `…`)
//! reads with no error and is in canonical form (spec 5.2). A block that is not a whole file is completed into one: a
//! header before a `fn`, `extern` or `type` block (one file per `type`), an empty function before a `panel`, `layout`,
//! `terminals`, `outline` or `icon` block, a body around statements. A block that is none of these is prose (a tree, a command) and is left out.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use labcraft_format::{STACK_SIZE, format};
use labcraft_types::{Interner, Limits};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Markdown files whose code blocks are programs: the spec, and every `.md` of `skills/` at any depth.
fn markdown_files() -> Vec<PathBuf> {
    fn walk(folder: &Path, files: &mut Vec<PathBuf>) {
        let mut entries: Vec<PathBuf> = fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                files.push(path);
            }
        }
    }
    let mut files = vec![repository().join("docs/language-spec-v0.md")];
    walk(&repository().join("skills"), &mut files);
    files
}

/// Blocks whose grammar the spec gives before the parser reads it: each is a text the block holds. The test checks
/// that each still fails, so that the entry goes when the parser catches up (`AGENTS.md`).
const AHEAD_OF_THE_PARSER: &[&str] = &[
    // Spec 11.4, M1 step 6 (#165): the outline of the compact block.
    "outline {",
];

/// The blocks fenced as `text` or with no word, each with the line of the spec where it starts. A block indented in a
/// list loses the indent of its fence.
fn blocks(text: &str) -> Vec<(usize, Vec<&str>)> {
    let mut found = Vec::new();
    let mut open: Option<(bool, usize, usize, Vec<&str>)> = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        match &mut open {
            None => {
                if let Some(info) = trimmed.strip_prefix("```") {
                    let indent = line.len() - line.trim_start().len();
                    open = Some((matches!(info.trim(), "" | "text"), indent, index + 2, Vec::new()));
                }
            }
            Some((program, indent, start, lines)) => {
                if trimmed == "```" {
                    if *program {
                        found.push((*start, std::mem::take(lines)));
                    }
                    open = None;
                } else {
                    let cut = line.len() - line.trim_start().len();
                    lines.push(line.get((*indent).min(cut)..).unwrap_or(line));
                }
            }
        }
    }
    assert!(open.is_none(), "a fence is not closed");
    found
}

fn is_statement(first: &str) -> bool {
    first.contains(" = ")
        || first.starts_with('#')
        || first.split_once('(').is_some_and(|(name, _)| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.'))
}

/// The files a block stands for, in canonical form when the block is; none for prose or an illustration.
fn files_of(lines: &[&str]) -> Vec<String> {
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    if text.contains('…') {
        return Vec::new();
    }
    let first = lines.iter().map(|line| line.trim()).find(|line| !line.is_empty() && !line.starts_with("//")).unwrap_or("");
    let word = first.split(' ').next().unwrap_or("");
    // A mistyped header (`labcraft1`) is still a whole file, so `read` reports it instead of the test skipping it.
    if word.starts_with("labcraft") {
        vec![text]
    } else if word == "type" {
        let mut files = Vec::new();
        let mut part = String::new();
        for line in lines {
            if line.starts_with("type ") && !part.trim().is_empty() {
                files.push(format!("labcraft 1\n\n{}\n", part.trim_end()));
                part.clear();
            }
            part.push_str(line);
            part.push('\n');
        }
        files.push(format!("labcraft 1\n\n{}\n", part.trim_end()));
        files
    } else if matches!(word, "fn" | "extern") && text.contains('{') {
        vec![format!("labcraft 1\n\n{text}")]
    } else if matches!(word, "panel" | "layout" | "terminals" | "outline" | "icon") {
        vec![format!("labcraft 1\n\nfn spec_check() -> () {{}}\n\n{text}")]
    } else if !matches!(word, "fn" | "extern") && is_statement(first) {
        let body: String = lines.iter().map(|line| if line.trim().is_empty() { format!("{line}\n") } else { format!("    {line}\n") }).collect();
        vec![format!("labcraft 1\n\nfn spec_check() -> () {{\n{body}}}\n")]
    } else {
        Vec::new()
    }
}

#[test]
fn every_program_block_reads_and_is_canonical() {
    let work = || {
        let mut problems = Vec::new();
        let mut checked = 0;
        for path in markdown_files() {
            let text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
            let name = path.strip_prefix(repository()).unwrap_or(&path).display().to_string();
            for (start, lines) in blocks(&text) {
                for file in files_of(&lines) {
                    checked += 1;
                    let mut names = Interner::new();
                    let formatted = format(file.as_bytes(), &mut names, &Limits::default());
                    let errors: Vec<String> = formatted.report.diagnostics().iter().map(|d| format!("{d:?}")).collect();
                    if let Some(ahead) = AHEAD_OF_THE_PARSER.iter().find(|ahead| file.contains(*ahead)) {
                        if errors.is_empty() {
                            problems.push(format!("{name}, block at line {start}: reads now, remove `{ahead}` from AHEAD_OF_THE_PARSER"));
                        }
                    } else if !errors.is_empty() {
                        problems.push(format!("{name}, block at line {start}: {}", errors.join("; ")));
                    } else if formatted.text.as_deref() != Some(file.as_str()) {
                        problems.push(format!("{name}, block at line {start}: not canonical, `fmt` writes\n{}", formatted.text.unwrap_or_default()));
                    }
                }
            }
        }
        // 37 on 2026-11-15; far fewer means the blocks are no longer found.
        assert!(checked >= 30, "{checked} blocks checked");
        assert!(problems.is_empty(), "{} blocks:\n{}", problems.len(), problems.join("\n"));
    };
    std::thread::Builder::new().stack_size(STACK_SIZE).spawn(work).unwrap().join().unwrap();
}
