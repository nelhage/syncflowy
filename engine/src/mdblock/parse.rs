//! The line-based body parser (§4.1, §4.3, §4.4, §5.3, §6.1, §9.1 guard 4).
//!
//! Structure comes from indentation widths, not from a fixed unit: a list item is a child
//! of the nearest open item whose marker is indented less than its own (a stack, as in
//! CommonMark), so tabs and any consistent number of spaces both work, and inconsistent
//! input is coerced (with a warning) instead of rejected. Tabs count to the next multiple
//! of 4 columns (CommonMark's tab stop).

use std::collections::BTreeSet;

use super::frontmatter;
use super::{
    CONFLICT_PREFIX, Checkbox, ItemKind, LocalDoc, LocalItem, PLACEHOLDER_TEXT, ParseError,
    ParseWarning, READ_ONLY_PREFIX, STUB_PREFIX, WarningKind,
};
use crate::mdinline::{ConflictField, IdTag, LineTag, TagFlag, find_tags, trim_line};

/// Indentation of `s`: (bytes of leading spaces/tabs, width in columns).
pub(super) fn indent_of(s: &str) -> (usize, usize) {
    let mut w = 0;
    let mut b = 0;
    for c in s.chars() {
        match c {
            ' ' => w += 1,
            '\t' => w = (w / 4 + 1) * 4,
            _ => break,
        }
        b += 1;
    }
    (b, w)
}

/// Remove up to `target` columns of leading indentation.
fn dedent(s: &str, target: usize) -> &str {
    let mut w = 0;
    for (i, c) in s.char_indices() {
        if w >= target {
            return &s[i..];
        }
        match c {
            ' ' => w += 1,
            '\t' => w = (w / 4 + 1) * 4,
            _ => return &s[i..],
        }
    }
    ""
}

fn ws_or_end(s: &str) -> bool {
    s.is_empty() || s.starts_with([' ', '\t'])
}

fn trim_ws(s: &str) -> &str {
    s.trim_matches([' ', '\t'])
}

/// A list marker at the start of `rest` (after indentation): `(ordered, text after the
/// marker and the whitespace following it)`.
pub(super) fn list_marker(rest: &str) -> Option<(bool, &str)> {
    let b = rest.as_bytes();
    let first = *b.first()?;
    if matches!(first, b'-' | b'*' | b'+') && ws_or_end(&rest[1..]) {
        return Some((false, trim_ws(&rest[1..])));
    }
    if first.is_ascii_digit() {
        let n = b.iter().take_while(|c| c.is_ascii_digit()).count();
        if n <= 9 && matches!(b.get(n), Some(b'.' | b')')) && ws_or_end(&rest[n + 1..]) {
            return Some((true, trim_ws(&rest[n + 1..])));
        }
    }
    None
}

/// An ATX heading at column 0: `(level, text)`.
pub(super) fn heading_marker(line: &str) -> Option<(u8, &str)> {
    let n = line.bytes().take_while(|&c| c == b'#').count();
    if (1..=6).contains(&n) && ws_or_end(&line[n..]) {
        Some((n as u8, trim_ws(&line[n..])))
    } else {
        None
    }
}

/// An opening code fence at the start of `s`: `(fence char, length, info string)`.
/// A backtick fence whose info string contains a backtick is not a fence (CommonMark).
pub(super) fn opening_fence(s: &str) -> Option<(char, usize, &str)> {
    let c = s.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = s.chars().take_while(|&x| x == c).count();
    if n < 3 {
        return None;
    }
    let info = trim_ws(&s[n..]);
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info))
}

/// True if `s` (indentation removed) closes a fence of `c` × `len`.
pub(super) fn is_closing_fence(s: &str, c: char, len: usize) -> bool {
    let n = s.chars().take_while(|&x| x == c).count();
    n >= len && trim_ws(&s[n * c.len_utf8()..]).is_empty()
}

/// Strip `prefix` (`🔒`, `⚠`) when followed by whitespace or the end.
fn strip_symbol<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let sym = prefix.trim_end();
    let rest = s.strip_prefix(sym)?;
    ws_or_end(rest).then(|| trim_ws(rest))
}

struct Ln<'a> {
    no: usize,
    raw: &'a str,
    /// Text after the indentation.
    rest: &'a str,
    width: usize,
    blank: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Root,
    Slot(usize),
}

struct Slot {
    item: LocalItem,
    children: Vec<usize>,
    /// Marker indentation width (list items) or 0 (headings).
    width: usize,
    /// Indentation of the first list child, for the consistency warning.
    child_width: Option<usize>,
}

/// The result of reading an item's first line (after the marker or `#`s).
struct Head {
    tag: Option<IdTag>,
    extra_tags: Vec<IdTag>,
    kind: ItemKind,
    checkbox: Checkbox,
    name_md: String,
    /// `Some((char, len, info))` if the text opens a code fence whose content follows.
    fence: Option<(char, usize, String)>,
}

struct Parser<'a> {
    lines: Vec<Ln<'a>>,
    i: usize,
    slots: Vec<Slot>,
    top: Vec<usize>,
    root_note: Vec<String>,
    root_child_width: Option<usize>,
    /// Open headings: (raw level, slot).
    headings: Vec<(u8, usize)>,
    /// Open list items of the current section.
    list: Vec<usize>,
    /// Pending blank lines: (line number, indentation width if whitespace-only).
    pending: Vec<(usize, usize)>,
    warnings: Vec<ParseWarning>,
    /// `headings:` of the file's config (for [`WarningKind::HeadingBeyondConfig`]).
    headings_cfg: u8,
}

/// Parse a whole file (see [`super::parse`]).
pub(super) fn parse(text: &str) -> Result<LocalDoc, ParseError> {
    let raw_lines = frontmatter::split_lines(text);
    let fm = frontmatter::parse(&raw_lines)?;
    let lines = raw_lines
        .iter()
        .enumerate()
        .skip(fm.body_start)
        .map(|(i, raw)| {
            let (b, width) = indent_of(raw);
            let rest = &raw[b..];
            Ln {
                no: i + 1,
                raw,
                rest,
                width,
                // Blank as `md_to_wf` sees it (Unicode White_Space only), so a line of NBSPs
                // is a blank line and not a stray line.
                blank: trim_line(rest).is_empty(),
            }
        })
        .collect();
    let mut p = Parser {
        lines,
        i: 0,
        slots: Vec::new(),
        top: Vec::new(),
        root_note: Vec::new(),
        root_child_width: None,
        headings: Vec::new(),
        list: Vec::new(),
        pending: Vec::new(),
        warnings: fm.warnings,
        headings_cfg: fm.fm.config.headings,
    };
    p.run();
    let mut slots: Vec<Option<Slot>> = p.slots.into_iter().map(Some).collect();
    let items = p.top.iter().map(|&i| assemble(&mut slots, i)).collect();
    let mut warnings = p.warnings;
    warnings.sort_by_key(|w| w.line);
    Ok(LocalDoc {
        frontmatter: fm.fm,
        root_note: p.root_note,
        items,
        warnings,
    })
}

fn assemble(slots: &mut [Option<Slot>], i: usize) -> LocalItem {
    let slot = slots[i].take().expect("each slot is assembled once");
    let mut item = slot.item;
    item.children = slot.children.iter().map(|&c| assemble(slots, c)).collect();
    item
}

impl<'a> Parser<'a> {
    fn warn(&mut self, line: usize, kind: WarningKind, message: impl Into<String>) {
        self.warnings.push(ParseWarning {
            line,
            kind,
            message: message.into(),
        });
    }

    fn owner(&self) -> Owner {
        match self.headings.last() {
            Some(&(_, s)) => Owner::Slot(s),
            None => Owner::Root,
        }
    }

    fn run(&mut self) {
        while self.i < self.lines.len() {
            let ln = &self.lines[self.i];
            if ln.blank {
                let ws = (!ln.raw.is_empty()).then_some(ln.width);
                self.pending.push((ln.no, ws.unwrap_or(0)));
                self.i += 1;
                continue;
            }
            if ln.width == 0
                && let Some((level, text)) = heading_marker(ln.rest)
            {
                self.flush_trailing();
                self.heading(level, text);
                continue;
            }
            if let Some((ordered, text)) = list_marker(ln.rest) {
                self.flush_trailing();
                self.list_item(ordered, text);
                continue;
            }
            self.text_line();
        }
        self.flush_trailing();
    }

    // ----- notes -------------------------------------------------------------------------

    fn note_of(&mut self, owner: Owner) -> &mut Vec<String> {
        match owner {
            Owner::Root => &mut self.root_note,
            Owner::Slot(s) => &mut self.slots[s].item.note_md,
        }
    }

    fn extend_end(&mut self, owner: Owner, line: usize) {
        if let Owner::Slot(s) = owner {
            let it = &mut self.slots[s].item;
            it.end_line = it.end_line.max(line);
        }
    }

    /// Blank lines before the next structural line or the end: they belong to the open
    /// list item's note only if whitespace-only and indented deeper than its marker
    /// (that is how trailing empty note lines are rendered); everything up to the last
    /// such line counts.
    fn flush_trailing(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        if let Some(&s) = self.list.last() {
            let w = self.slots[s].width;
            if let Some(last) = pending.iter().rposition(|&(_, ws)| ws > w) {
                for &(no, _) in &pending[..=last] {
                    self.slots[s].item.note_md.push(String::new());
                    self.extend_end(Owner::Slot(s), no);
                }
            }
        }
    }

    /// A non-structural, non-blank line.
    fn text_line(&mut self) {
        let ln = &self.lines[self.i];
        let (no, width, rest) = (ln.no, ln.width, ln.rest);
        let pending = std::mem::take(&mut self.pending);
        let owner;
        if let Some(&s) = self.list.last() {
            owner = Owner::Slot(s);
            if width <= self.slots[s].width {
                // §9.1 guard 4: a line that fits nowhere (a lazy continuation, column-0
                // text after a list) joins the nearest preceding item's note.
                self.warn(
                    no,
                    WarningKind::StrayLine,
                    "line is not indented under an item; attached to the preceding item's note",
                );
            } else {
                for &(pno, _) in &pending {
                    self.slots[s].item.note_md.push(String::new());
                    self.extend_end(owner, pno);
                }
            }
        } else {
            owner = self.owner();
            // Root and heading notes: blank lines inside count, leading ones do not.
            if !self.note_of(owner).is_empty() {
                for _ in &pending {
                    self.note_of(owner).push(String::new());
                }
            }
        }
        let line = trim_ws(rest).to_owned();
        let fence = opening_fence(&line).map(|(c, len, _)| (c, len));
        self.note_of(owner).push(line);
        self.extend_end(owner, no);
        self.i += 1;
        if let Some((c, len)) = fence
            && let Some(close) = self.note_fence_end(owner, c, len)
        {
            // A fenced block typed inside a note stays note text, and the lines inside it
            // are not structure (`- x` in it is not a child). Only a fence that is closed
            // before the note's item ends counts, so an unclosed fence never swallows
            // the items after it.
            while self.i <= close {
                let ln = &self.lines[self.i];
                let (lno, l) = (ln.no, trim_ws(ln.rest).to_owned());
                self.note_of(owner).push(l);
                self.extend_end(owner, lno);
                self.i += 1;
            }
        }
    }

    /// The index of the line closing a fence opened in `owner`'s note on the previous
    /// line, if it closes before the note can end: before a line indented at or left of
    /// the owning list item's marker, or (for the root and heading notes) before a
    /// column-0 heading or list item.
    fn note_fence_end(&self, owner: Owner, c: char, len: usize) -> Option<usize> {
        let list_width = match owner {
            Owner::Slot(s) if self.slots[s].item.heading.is_none() => Some(self.slots[s].width),
            _ => None,
        };
        for j in self.i..self.lines.len() {
            let ln = &self.lines[j];
            if ln.blank {
                continue;
            }
            match list_width {
                Some(w) if ln.width <= w => return None,
                None if ln.width == 0
                    && (heading_marker(ln.rest).is_some() || list_marker(ln.rest).is_some()) =>
                {
                    return None;
                }
                _ => {}
            }
            if is_closing_fence(ln.rest, c, len) {
                return Some(j);
            }
        }
        None
    }

    // ----- items -------------------------------------------------------------------------

    fn new_slot(&mut self, item: LocalItem, width: usize) -> usize {
        self.slots.push(Slot {
            item,
            children: Vec::new(),
            width,
            child_width: None,
        });
        self.slots.len() - 1
    }

    fn attach(&mut self, parent: Owner, child: usize) {
        match parent {
            Owner::Root => self.top.push(child),
            Owner::Slot(p) => self.slots[p].children.push(child),
        }
    }

    fn heading(&mut self, raw_level: u8, text: &str) {
        let no = self.lines[self.i].no;
        self.list.clear();
        while self.headings.last().is_some_and(|&(l, _)| l >= raw_level) {
            self.headings.pop();
        }
        let parent_level = self
            .headings
            .last()
            .map_or(0, |&(_, s)| self.slots[s].item.heading.unwrap_or(0));
        let level = parent_level + 1;
        if raw_level != level {
            // §4.1: a heading that skips a level is coerced to parent level + 1.
            self.warn(
                no,
                WarningKind::HeadingLevelSkipped,
                format!("heading level {raw_level} coerced to {level}"),
            );
        }
        if level > self.headings_cfg {
            self.warn(
                no,
                WarningKind::HeadingBeyondConfig,
                format!(
                    "heading at depth {level} but headings: {}; kept as a heading",
                    self.headings_cfg
                ),
            );
        }
        let head = self.head(text, no);
        let parent = self.owner();
        let item = LocalItem {
            tag: head.tag,
            extra_tags: head.extra_tags,
            kind: head.kind,
            heading: Some(level),
            checkbox: head.checkbox,
            ordered: false,
            name_md: head.name_md,
            note_md: Vec::new(),
            children: Vec::new(),
            line: no,
            end_line: no,
        };
        if item.is_marker() {
            self.warn(
                no,
                WarningKind::MisplacedMarker,
                "a conflict marker written as a heading",
            );
        }
        let s = self.new_slot(item, 0);
        self.attach(parent, s);
        if let Owner::Slot(p) = parent {
            self.mark_marker_parent(p, no);
        }
        self.headings.push((raw_level, s));
        self.i += 1;
        if let Some((c, len, info)) = head.fence {
            self.heading_code(s, c, len, &info);
        }
    }

    fn list_item(&mut self, ordered: bool, text: &str) {
        let (no, width) = (self.lines[self.i].no, self.lines[self.i].width);
        while self
            .list
            .last()
            .is_some_and(|&s| self.slots[s].width >= width)
        {
            self.list.pop();
        }
        let parent = match self.list.last() {
            Some(&s) => Owner::Slot(s),
            None => self.owner(),
        };
        let cw = match parent {
            Owner::Root => &mut self.root_child_width,
            Owner::Slot(p) => &mut self.slots[p].child_width,
        };
        let inconsistent = cw.is_some_and(|c| c != width);
        cw.get_or_insert(width);
        if inconsistent {
            self.warn(
                no,
                WarningKind::Indentation,
                "indentation differs from the item's siblings; coerced",
            );
        }
        let head = self.head(text, no);
        let item = LocalItem {
            tag: head.tag,
            extra_tags: head.extra_tags,
            kind: head.kind,
            heading: None,
            checkbox: head.checkbox,
            ordered,
            name_md: head.name_md,
            note_md: Vec::new(),
            children: Vec::new(),
            line: no,
            end_line: no,
        };
        let s = self.new_slot(item, width);
        self.attach(parent, s);
        if let Owner::Slot(p) = parent {
            self.mark_marker_parent(p, no);
        }
        self.list.push(s);
        self.i += 1;
        if let Some((c, len, info)) = head.fence {
            self.list_code(s, c, len, &info);
        }
    }

    /// A conflict marker with children is misplaced (it is never a node).
    fn mark_marker_parent(&mut self, p: usize, line: usize) {
        if self.slots[p].item.is_marker() && self.slots[p].children.len() == 1 {
            self.warn(
                line,
                WarningKind::MisplacedMarker,
                "items nested under a conflict marker",
            );
        }
    }

    /// Code content of a list item (§4.3): lines until the closing fence, which may be
    /// indented anywhere at or below the item's marker; a non-blank line indented at or
    /// left of the marker ends an unclosed fence.
    fn list_code(&mut self, s: usize, c: char, len: usize, info: &str) {
        let item_width = self.slots[s].width;
        let start = self.i;
        let mut close: Option<(usize, usize)> = None;
        let mut j = start;
        while j < self.lines.len() {
            let ln = &self.lines[j];
            if !ln.blank && ln.width >= item_width && is_closing_fence(ln.rest, c, len) {
                close = Some((j, ln.width));
                break;
            }
            if !ln.blank && ln.width <= item_width {
                break;
            }
            j += 1;
        }
        let mut end = j;
        if close.is_none() {
            while end > start && self.lines[end - 1].blank {
                end -= 1;
            }
            let no = self.slots[s].item.line;
            self.warn(no, WarningKind::UnclosedFence, "code fence not closed");
        }
        let target = match close {
            Some((_, w)) if w > item_width => w,
            _ => self.lines[start..end]
                .iter()
                .filter(|l| !l.blank)
                .map(|l| l.width)
                .min()
                .unwrap_or(0),
        };
        let content: Vec<String> = self.lines[start..end]
            .iter()
            .map(|l| dedent(l.raw, target).to_owned())
            .collect();
        let last = match close {
            Some((ci, _)) => self.lines[ci].no,
            None if end > start => self.lines[end - 1].no,
            None => self.slots[s].item.line,
        };
        self.finish_code(s, info, content, last);
        self.i = close.map_or(end, |(ci, _)| ci + 1);
    }

    /// Code content of a heading: column-0 lines up to a closing fence indented less
    /// than 4 columns; if there is none, up to the next heading or column-0 list item.
    fn heading_code(&mut self, s: usize, c: char, len: usize, info: &str) {
        let start = self.i;
        let close = (start..self.lines.len()).find(|&j| {
            let ln = &self.lines[j];
            !ln.blank && ln.width < 4 && is_closing_fence(ln.rest, c, len)
        });
        let end = close.unwrap_or_else(|| {
            let mut e = (start..self.lines.len())
                .find(|&j| {
                    let ln = &self.lines[j];
                    ln.width == 0
                        && (heading_marker(ln.rest).is_some() || list_marker(ln.rest).is_some())
                })
                .unwrap_or(self.lines.len());
            while e > start && self.lines[e - 1].blank {
                e -= 1;
            }
            e
        });
        if close.is_none() {
            let no = self.slots[s].item.line;
            self.warn(no, WarningKind::UnclosedFence, "code fence not closed");
        }
        let target = close.map_or(0, |ci| self.lines[ci].width);
        let content: Vec<String> = self.lines[start..end]
            .iter()
            .map(|l| dedent(l.raw, target).to_owned())
            .collect();
        let last = match close {
            Some(ci) => self.lines[ci].no,
            None if end > start => self.lines[end - 1].no,
            None => self.slots[s].item.line,
        };
        self.finish_code(s, info, content, last);
        self.i = close.map_or(end, |ci| ci + 1);
    }

    fn finish_code(&mut self, s: usize, info: &str, content: Vec<String>, last: usize) {
        // §4.3: a non-empty info string is folded into the code as its first line, as the
        // API's create pipeline does (DM§6.2 step 7).
        let mut lines = Vec::with_capacity(content.len() + 1);
        if !info.is_empty() {
            lines.push(info.to_owned());
        }
        lines.extend(content);
        let it = &mut self.slots[s].item;
        it.name_md = lines.join("\n");
        it.end_line = it.end_line.max(last);
    }

    /// Read an item's first line after its marker: tags, checkbox, prefixes, kind (§4.3,
    /// §4.4, §5.3, §6.1, §10).
    fn head(&mut self, text: &str, no: usize) -> Head {
        let found = find_tags(text);
        let mut ids: Vec<IdTag> = Vec::new();
        let mut conflict: Option<ConflictField> = None;
        let mut out = String::with_capacity(text.len());
        let mut pos = 0;
        let mut skip_space = false;
        for m in &found {
            let mut seg = &text[pos..m.range.start];
            if skip_space && seg.starts_with([' ', '\t']) {
                seg = &seg[1..];
            }
            out.push_str(seg);
            // §4.4: a tag is removed together with one adjacent space.
            if out.ends_with([' ', '\t']) {
                out.pop();
                skip_space = false;
            } else {
                skip_space = true;
            }
            pos = m.range.end;
            match &m.tag {
                LineTag::Id(t) => {
                    if has_unknown_flag(&text[m.range.clone()]) {
                        self.warn(
                            no,
                            WarningKind::UnknownTagFlag,
                            format!("unknown flag in id tag {:?}", &text[m.range.clone()]),
                        );
                    }
                    ids.push(t.clone());
                }
                LineTag::Conflict(f) => conflict = Some(*f),
            }
        }
        let mut seg = &text[pos..];
        if skip_space && seg.starts_with([' ', '\t']) {
            seg = &seg[1..];
        }
        out.push_str(seg);
        let mut t = trim_ws(&out);

        let mut checkbox = Checkbox::None;
        let cb = t.as_bytes();
        if cb.len() >= 3
            && cb[0] == b'['
            && cb[2] == b']'
            && matches!(cb[1], b' ' | b'x' | b'X')
            && ws_or_end(&t[3..])
        {
            checkbox = if cb[1] == b' ' {
                Checkbox::Open
            } else {
                Checkbox::Ticked
            };
            t = trim_ws(&t[3..]);
        }

        let mut extra_tags = Vec::new();
        let mut tag = None;
        if let Some(field) = conflict {
            // §6.1: a conflict marker has no id tag; any found are kept as extras.
            if !ids.is_empty() {
                self.warn(
                    no,
                    WarningKind::MultipleTags,
                    "a conflict marker carries an id tag (ignored)",
                );
            }
            extra_tags = ids;
            let t = strip_symbol(t, CONFLICT_PREFIX).unwrap_or(t);
            let (name_md, fence) = match opening_fence(t) {
                Some((c, n, info)) => (String::new(), Some((c, n, info.to_owned()))),
                None => (t.to_owned(), None),
            };
            let name_md = if field == ConflictField::Note {
                String::new()
            } else {
                name_md
            };
            let fence = if field == ConflictField::Note {
                None
            } else {
                fence
            };
            return Head {
                tag: None,
                extra_tags,
                kind: ItemKind::ConflictMarker(field),
                checkbox,
                name_md,
                fence,
            };
        }
        if let Some(last) = ids.pop() {
            if !ids.is_empty() {
                // §4.4: two tags on a line: the last one wins.
                self.warn(
                    no,
                    WarningKind::MultipleTags,
                    format!("{} id tags on one line; the last one wins", ids.len() + 1),
                );
            }
            extra_tags = ids;
            tag = Some(last);
        }
        let ro = tag
            .as_ref()
            .is_some_and(|t| t.flags.contains(&TagFlag::ReadOnly));
        if ro && t == PLACEHOLDER_TEXT {
            return Head {
                tag,
                extra_tags,
                kind: ItemKind::Placeholder,
                checkbox,
                name_md: String::new(),
                fence: None,
            };
        }
        if ro && t.starts_with(STUB_PREFIX) {
            // §15.1: a mirror stub; its text is derived and kept verbatim.
            return Head {
                tag,
                extra_tags,
                kind: ItemKind::MirrorStub,
                checkbox,
                name_md: t.to_owned(),
                fence: None,
            };
        }
        if ro && let Some(r) = strip_symbol(t, READ_ONLY_PREFIX) {
            t = r;
        }
        let (kind, name_md, fence) = if let Some((c, n, info)) = opening_fence(t) {
            (ItemKind::Code, String::new(), Some((c, n, info.to_owned())))
        } else if let Some(q) = t.strip_prefix('>') {
            (ItemKind::Quote, trim_ws(q).to_owned(), None)
        } else if t.len() >= 3 && t.bytes().all(|b| b == b'-') {
            (ItemKind::Divider, String::new(), None)
        } else {
            (ItemKind::Bullet, t.to_owned(), None)
        };
        Head {
            tag,
            extra_tags,
            kind,
            checkbox,
            name_md,
            fence,
        }
    }
}

/// True if the tag text has a `-flag` other than `ro` after its 12 hex digits.
fn has_unknown_flag(tag: &str) -> bool {
    let b = tag.as_bytes();
    let Some(start) = (0..b.len().saturating_sub(11)).find(|&i| {
        b[i..i + 12].iter().all(u8::is_ascii_hexdigit) && (i == 0 || !b[i - 1].is_ascii_hexdigit())
    }) else {
        return false;
    };
    let flags: String = tag[start + 12..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let known: BTreeSet<&str> = [TagFlag::ReadOnly.as_str()].into();
    flags
        .split('-')
        .filter(|f| !f.is_empty())
        .any(|f| !known.contains(f.to_ascii_lowercase().as_str()))
}
