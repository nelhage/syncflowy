//! The writer shared by [`super::render`] (from a [`super::View`]) and
//! [`super::normalize`] (from a [`super::LocalDoc`]): both first build a list of
//! [`RItem`]s, so the two produce the same bytes for the same content (invariant 1).
//!
//! Layout of the body (§4.1):
//!
//! * blocks separated by one blank line: the root note; the list items of the root (root
//!   conflict markers first); each heading with its note; the list items of each
//!   heading (its conflict markers first); sub-headings;
//! * list items indented with one tab per level, markers `- ` or `N.`; the first line is
//!   `[checkbox] [🔒] <kind prefix><text> <tag>` with empty parts left out;
//! * a code item's fence opens on its first line, its lines and the closing fence are
//!   indented one level deeper than the marker (blank code lines are written empty);
//! * note lines are indented one level deeper than the marker; an empty note line is a
//!   blank line, except after the last non-empty note line, where it is written as the
//!   indentation alone (otherwise the parser could not tell it from the blank lines
//!   between items);
//! * a heading's code lines and note lines are at column 0; heading and root notes lose
//!   leading and trailing empty lines (blank lines around a paragraph carry nothing).

use std::collections::BTreeSet;

use super::frontmatter;
use super::parse::opening_fence;
use super::{
    CONFLICT_PREFIX, Checkbox, Frontmatter, ItemKind, PLACEHOLDER_TEXT, READ_ONLY_PREFIX, TagStyle,
};
use crate::mdinline::{ConflictField, TagFlag, TagForm, format_conflict_tag, format_tag};
use crate::tree::ShortId;

/// The label of a note conflict marker (the local note follows as its note lines).
pub(super) const NOTE_MARKER_LABEL: &str = "local note";

/// One item to write.
#[derive(Debug, Clone)]
pub(super) struct RItem {
    pub tag: Option<(ShortId, BTreeSet<TagFlag>)>,
    pub kind: ItemKind,
    pub heading: Option<u8>,
    pub checkbox: Checkbox,
    pub ordered: bool,
    /// Prefix the text with `🔒 ` (read-only rendering, §5.3).
    pub read_only: bool,
    /// The name Markdown; for code (and multi-line name markers) the code lines joined
    /// with `\n`. Ignored for placeholders and dividers.
    pub name: String,
    /// Note lines (empty: no note).
    pub note: Vec<String>,
    pub children: Vec<RItem>,
}

/// The tag form for an item (§4.4: block ids are not allowed on headings).
fn form(style: TagStyle, heading: bool) -> TagForm {
    match style {
        TagStyle::Auto | TagStyle::BlockId if heading => TagForm::Comment,
        TagStyle::Auto | TagStyle::BlockId => TagForm::BlockId,
        TagStyle::Comment => TagForm::Comment,
        TagStyle::Dataview => TagForm::Dataview,
        TagStyle::DataviewHidden => TagForm::DataviewHidden,
        TagStyle::Html => TagForm::Html,
    }
}

/// A backtick fence longer than any backtick run that starts a line of `code` (after
/// optional indentation), so no code line can close it; at least three.
fn fence_for(code: &str) -> String {
    let longest = code
        .split('\n')
        .map(|l| {
            l.trim_start_matches([' ', '\t'])
                .chars()
                .take_while(|&c| c == '`')
                .count()
        })
        .max()
        .unwrap_or(0);
    "`".repeat((longest + 1).max(3))
}

/// True if a name conflict marker's text must be written as a fenced block: it has
/// several lines (a code block's name) or would itself read as a fence.
pub(super) fn marker_needs_fence(name: &str) -> bool {
    name.contains('\n') || opening_fence(name).is_some()
}

/// The first line's text after the marker (or `#`s), and the code block, if any, as
/// `(fence, code)`.
fn first_line(it: &RItem, style: TagStyle) -> (String, Option<(String, String)>) {
    let mut parts: Vec<String> = Vec::new();
    match it.checkbox {
        Checkbox::None => {}
        Checkbox::Open => parts.push("[ ]".to_owned()),
        Checkbox::Ticked => parts.push("[x]".to_owned()),
    }
    let mut code = None;
    match it.kind {
        ItemKind::ConflictMarker(field) => {
            parts.push(CONFLICT_PREFIX.trim_end().to_owned());
            match field {
                ConflictField::Name if marker_needs_fence(&it.name) => {
                    let f = fence_for(&it.name);
                    parts.push(f.clone());
                    code = Some((f, it.name.clone()));
                }
                ConflictField::Name => parts.push(it.name.clone()),
                ConflictField::Note => parts.push(NOTE_MARKER_LABEL.to_owned()),
            }
            parts.push(format_conflict_tag(field));
        }
        kind => {
            if it.read_only && kind != ItemKind::Placeholder {
                parts.push(READ_ONLY_PREFIX.trim_end().to_owned());
            }
            match kind {
                ItemKind::Bullet => parts.push(it.name.clone()),
                ItemKind::Quote if it.name.is_empty() => parts.push(">".to_owned()),
                ItemKind::Quote => parts.push(format!("> {}", it.name)),
                ItemKind::Divider => parts.push("---".to_owned()),
                ItemKind::Placeholder => parts.push(PLACEHOLDER_TEXT.to_owned()),
                ItemKind::Code => {
                    let f = fence_for(&it.name);
                    parts.push(f.clone());
                    code = Some((f, it.name.clone()));
                }
                ItemKind::ConflictMarker(_) => unreachable!("handled above"),
            }
            if let Some((short, flags)) = &it.tag {
                parts.push(format_tag(short, flags, form(style, it.heading.is_some())));
            }
        }
    }
    let text = parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (text, code)
}

fn trim_blank_edges(lines: &[String]) -> &[String] {
    let start = lines
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(start, |e| e + 1);
    &lines[start..end]
}

fn push_line(out: &mut Vec<String>, line: String) {
    let t = line.trim_end_matches([' ', '\t']).len();
    let mut line = line;
    line.truncate(t);
    out.push(line);
}

fn list_item(it: &RItem, depth: usize, ord: &mut usize, style: TagStyle, out: &mut Vec<String>) {
    let ind = "\t".repeat(depth);
    let marker = if it.ordered {
        *ord += 1;
        format!("{ord}.")
    } else {
        "-".to_owned()
    };
    let (text, code) = first_line(it, style);
    push_line(out, format!("{ind}{marker} {text}"));
    let cind = format!("{ind}\t");
    if let Some((fence, code)) = code {
        for l in code.split('\n') {
            out.push(if l.is_empty() {
                String::new()
            } else {
                format!("{cind}{l}")
            });
        }
        out.push(format!("{cind}{fence}"));
    }
    let last = it.note.iter().rposition(|l| !l.is_empty());
    for (k, l) in it.note.iter().enumerate() {
        if !l.is_empty() {
            out.push(format!("{cind}{l}"));
        } else if last.is_some_and(|last| k < last) {
            out.push(String::new());
        } else {
            out.push(cind.clone());
        }
    }
    let mut child_ord = 0;
    for c in &it.children {
        list_item(c, depth + 1, &mut child_ord, style, out);
    }
}

fn heading(it: &RItem, style: TagStyle, blocks: &mut Vec<Vec<String>>) {
    let level = usize::from(it.heading.unwrap_or(1).max(1));
    let mut block = Vec::new();
    let (text, code) = first_line(it, style);
    push_line(&mut block, format!("{} {text}", "#".repeat(level)));
    if let Some((fence, code)) = code {
        // UNVERIFIED (our choice): a code block at heading depth is a heading opening a
        // fence, with the code at column 0; it round-trips through our parser, but
        // Obsidian shows a heading followed by a code block.
        block.extend(code.split('\n').map(str::to_owned));
        block.push(fence);
    }
    block.extend(trim_blank_edges(&it.note).iter().cloned());
    blocks.push(block);
    section(&it.children, style, blocks);
}

fn section(children: &[RItem], style: TagStyle, blocks: &mut Vec<Vec<String>>) {
    let mut list = Vec::new();
    let mut ord = 0;
    for c in children {
        if c.heading.is_some() {
            if !list.is_empty() {
                blocks.push(std::mem::take(&mut list));
            }
            ord = 0;
            heading(c, style, blocks);
        } else {
            list_item(c, 0, &mut ord, style, &mut list);
        }
    }
    if !list.is_empty() {
        blocks.push(list);
    }
}

/// Write a whole file.
pub(super) fn emit(fm: &Frontmatter, root_note: &[String], items: &[RItem]) -> String {
    let style = fm.config.tags;
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let note = trim_blank_edges(root_note);
    if !note.is_empty() {
        blocks.push(note.to_vec());
    }
    section(items, style, &mut blocks);
    let mut out = frontmatter::render(fm);
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for l in b {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}
