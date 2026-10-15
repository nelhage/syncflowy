//! The frontmatter (§4.1, §9.1 guard 1 and the frontmatter controls): a tiny parser for
//! the YAML subset we use, and its renderer. No YAML dependency: only top-level
//! `key: value` lines are interpreted, and only for the two keys the engine owns
//! (`workflowy:` and `syncflowy:`); every other line is kept verbatim.

use super::{
    CheckboxPolicy, DocConfig, Frontmatter, MAX_HEADINGS, ParseError, ParseWarning, RootRef,
    TagStyle, WarningKind,
};
use crate::tree::{IdMap, NodeId, ShortId};

/// The lines of a text (a leading byte-order mark dropped). Line endings are `\n`,
/// `\r\n` or a lone `\r`, as in CommonMark (and as `mdinline` reads them); a final line
/// ending does not start another line.
pub(super) fn split_lines(text: &str) -> Vec<&str> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = Vec::new();
    let b = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                start = i + 1;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                if b.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The parsed frontmatter, its warnings, and the 0-based index of the first body line.
pub(super) struct Parsed {
    pub fm: Frontmatter,
    pub warnings: Vec<ParseWarning>,
    pub body_start: usize,
}

fn is_fence(line: &str, closing: bool) -> bool {
    let t = line.trim_end_matches([' ', '\t']);
    t == "---" || (closing && t == "...")
}

/// Index of the byte where a YAML comment starts (`#` at the start or after whitespace,
/// outside quotes), if any.
fn comment_start(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut prev_ws = true;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '#' && prev_ws {
                    return Some(i);
                }
                if (c == '"' || c == '\'') && prev_ws {
                    quote = Some(c);
                }
            }
        }
        prev_ws = c == ' ' || c == '\t';
    }
    None
}

fn strip_comment(s: &str) -> &str {
    match comment_start(s) {
        Some(i) => &s[..i],
        None => s,
    }
    .trim()
}

/// Remove one level of YAML quoting (`"…"` with `\"`/`\\` escapes, or `'…'` with `''`).
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(o) => out.push(o),
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        s[1..s.len() - 1].replace("''", "'")
    } else {
        s.to_owned()
    }
}

/// `key: rest` for a top-level mapping line (the key at column 0).
fn top_key(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let rest = &line[colon + 1..];
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    Some((key, rest))
}

impl RootRef {
    /// See the declaration in the module root.
    pub(super) fn parse_impl(s: &str) -> Result<RootRef, ParseError> {
        let raw = s.trim().to_owned();
        let bad = || ParseError::BadRoot(raw.clone());
        let value = unquote(strip_comment(&raw));
        let value = value.trim();
        if let Ok(short) = ShortId::parse(value) {
            return Ok(RootRef {
                raw,
                short,
                full: None,
            });
        }
        if let Ok(full) = NodeId::parse(value) {
            let short = full.short().ok_or_else(bad)?;
            return Ok(RootRef {
                raw,
                short,
                full: Some(full),
            });
        }
        // DM§3.2 URL form: `https?://(beta.)?workflowy.com/#/<12 hex>` followed by the
        // end, `/` or `?` (scheme, host and hex are case-insensitive).
        let lower = value.to_ascii_lowercase();
        let rest = lower
            .strip_prefix("https://")
            .or_else(|| lower.strip_prefix("http://"))
            .ok_or_else(bad)?;
        let rest = rest.strip_prefix("beta.").unwrap_or(rest);
        let rest = rest.strip_prefix("workflowy.com/#/").ok_or_else(bad)?;
        let hex = rest.get(..12).ok_or_else(bad)?;
        let after = &rest[12..];
        if !(after.is_empty() || after.starts_with(['/', '?'])) {
            return Err(bad());
        }
        let short = ShortId::parse(hex).map_err(|_| bad())?;
        Ok(RootRef {
            raw,
            short,
            full: None,
        })
    }

    pub(super) fn resolve_impl(&self, ids: &IdMap) -> Option<NodeId> {
        self.full.clone().or_else(|| ids.get(&self.short).cloned())
    }
}

fn bad_config(key: &str, value: &str) -> ParseError {
    ParseError::BadConfig {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn parse_bool(key: &str, v: &str) -> Result<bool, ParseError> {
    match v.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(bad_config(key, v)),
    }
}

fn tag_style(v: &str) -> Option<TagStyle> {
    Some(match v {
        "auto" => TagStyle::Auto,
        "block" => TagStyle::BlockId,
        "comment" => TagStyle::Comment,
        "dataview" => TagStyle::Dataview,
        "dataview-hidden" => TagStyle::DataviewHidden,
        "html" => TagStyle::Html,
        _ => return None,
    })
}

fn tag_style_name(s: TagStyle) -> &'static str {
    match s {
        TagStyle::Auto => "auto",
        TagStyle::BlockId => "block",
        TagStyle::Comment => "comment",
        TagStyle::Dataview => "dataview",
        TagStyle::DataviewHidden => "dataview-hidden",
        TagStyle::Html => "html",
    }
}

fn checkbox_policy(v: &str) -> Option<CheckboxPolicy> {
    Some(match v {
        "todos-and-leaves" => CheckboxPolicy::TodosAndLeaves,
        "todos" => CheckboxPolicy::TodosOnly,
        "all" => CheckboxPolicy::All,
        "none" => CheckboxPolicy::None,
        _ => return None,
    })
}

fn checkbox_policy_name(p: CheckboxPolicy) -> &'static str {
    match p {
        CheckboxPolicy::TodosAndLeaves => "todos-and-leaves",
        CheckboxPolicy::TodosOnly => "todos",
        CheckboxPolicy::All => "all",
        CheckboxPolicy::None => "none",
    }
}

/// Split a flow mapping body (without the braces) at top-level commas, honouring quotes.
fn split_flow(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, c) in body.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == ',' => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            None => {}
        }
    }
    out.push(&body[start..]);
    out
}

/// The index of the closing `}` of a flow mapping opened at byte 0 of `s`, outside quotes.
fn flow_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '}' => return Some(i),
            None => {}
        }
    }
    None
}

/// The keys of `syncflowy:`, which are warned about when written at the top level.
const OPTION_KEYS: [&str; 8] = [
    "headings",
    "ids",
    "checkboxes",
    "dates",
    "depth",
    "paused",
    "force",
    "gen",
];

/// The key of a top-level frontmatter line that is a `syncflowy:` option written outside
/// it (`paused: true`), if `line` is one.
pub(super) fn misplaced_option(line: &str) -> Option<&str> {
    top_key(line)
        .map(|(k, _)| k)
        .filter(|k| OPTION_KEYS.contains(k))
}

struct ConfigState {
    config: DocConfig,
    generation: Option<u64>,
    generation_text: Option<String>,
    paused: bool,
    force: bool,
}

/// Apply one `syncflowy:` entry. Unknown keys warn (§4.1: dropped on the next render).
fn apply_entry(
    st: &mut ConfigState,
    key: &str,
    raw_value: &str,
    line: usize,
    warnings: &mut Vec<ParseWarning>,
) -> Result<(), ParseError> {
    let v = unquote(raw_value);
    let v = v.trim();
    match key {
        "headings" => match v.parse::<u8>() {
            Ok(n) if n <= MAX_HEADINGS => st.config.headings = n,
            _ => return Err(bad_config(key, v)),
        },
        "ids" => st.config.tags = tag_style(v).ok_or_else(|| bad_config(key, v))?,
        "checkboxes" => {
            st.config.checkboxes = checkbox_policy(v).ok_or_else(|| bad_config(key, v))?;
        }
        "dates" => st.config.inline.dates = parse_bool(key, v)?,
        "depth" => {
            st.config.max_depth = match v {
                // UNVERIFIED (our choice): an explicit null means "unlimited".
                "null" | "~" | "" => None,
                _ => match v.parse::<u32>() {
                    Ok(n) if n > 0 => Some(n),
                    _ => return Err(bad_config(key, v)),
                },
            }
        }
        "paused" => st.paused = parse_bool(key, v)?,
        "force" => st.force = parse_bool(key, v)?,
        // §9.1 guard 1: an unparseable `gen` is not an error; `sync` falls back to safe mode
        // and names the value.
        "gen" => {
            st.generation = v.parse::<u64>().ok();
            st.generation_text = st.generation.is_none().then(|| v.to_owned());
        }
        _ => warnings.push(ParseWarning {
            line,
            kind: WarningKind::UnknownConfigKey,
            message: format!("unknown syncflowy key {key:?} (dropped on the next render)"),
        }),
    }
    Ok(())
}

fn apply_flow(
    st: &mut ConfigState,
    body: &str,
    line: usize,
    warnings: &mut Vec<ParseWarning>,
) -> Result<(), ParseError> {
    for entry in split_flow(body) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((k, v)) = entry.split_once(':') else {
            return Err(bad_config(entry, ""));
        };
        apply_entry(st, &unquote(k), v, line, warnings)?;
    }
    Ok(())
}

/// Parse the frontmatter at the start of `text` (lines as given by [`split_lines`]).
pub(super) fn parse(lines: &[&str]) -> Result<Parsed, ParseError> {
    if lines.first().is_none_or(|l| !is_fence(l, false)) {
        return Err(ParseError::NoFrontmatter);
    }
    let close = (1..lines.len())
        .find(|&i| is_fence(lines[i], true))
        .ok_or(ParseError::UnterminatedFrontmatter)?;
    let mut warnings = Vec::new();
    let mut root: Option<RootRef> = None;
    let mut other = Vec::new();
    let mut st = ConfigState {
        config: DocConfig::default(),
        generation: None,
        generation_text: None,
        paused: false,
        force: false,
    };
    let mut i = 1;
    while i < close {
        let line = lines[i];
        let line_no = i + 1;
        match top_key(line) {
            Some(("workflowy", rest)) => {
                let r = RootRef::parse_impl(rest)?;
                if let Some(prev) = &root
                    && (prev.short != r.short || prev.full != r.full)
                {
                    // Two different roots: refuse rather than guess (§9.1 guard 5).
                    return Err(ParseError::BadRoot(r.raw));
                }
                root.get_or_insert(r);
                i += 1;
            }
            Some(("syncflowy", rest)) => {
                let value = rest.trim();
                if value.starts_with('{') {
                    // A flow mapping, possibly continued on the following lines.
                    let mut text = value.to_owned();
                    let mut j = i + 1;
                    while flow_end(&text).is_none() && j < close {
                        text.push(' ');
                        text.push_str(lines[j].trim());
                        j += 1;
                    }
                    let end = flow_end(&text).ok_or_else(|| bad_config("syncflowy", value))?;
                    if !strip_comment(&text[end + 1..]).is_empty() {
                        return Err(bad_config("syncflowy", &text));
                    }
                    apply_flow(&mut st, &text[1..end], line_no, &mut warnings)?;
                    i = j;
                } else if strip_comment(value).is_empty() {
                    // A block mapping: the following indented lines.
                    let mut j = i + 1;
                    while j < close
                        && (lines[j].starts_with([' ', '\t']) || lines[j].trim().is_empty())
                    {
                        let entry = strip_comment(lines[j]);
                        if !entry.is_empty() {
                            let Some((k, v)) = entry.split_once(':') else {
                                return Err(bad_config(entry, ""));
                            };
                            apply_entry(&mut st, &unquote(k), v, j + 1, &mut warnings)?;
                        }
                        j += 1;
                    }
                    i = j;
                } else {
                    return Err(bad_config("syncflowy", value));
                }
            }
            Some((key, rest)) if OPTION_KEYS.contains(&key) => {
                // An option written outside `syncflowy:` is kept verbatim but warned
                // about. A top-level `paused: true` is honoured (pausing is the safe
                // direction); a top-level `force` never is.
                let value = unquote(strip_comment(rest));
                let honoured = key == "paused" && value.trim().eq_ignore_ascii_case("true");
                if honoured {
                    st.paused = true;
                }
                warnings.push(ParseWarning {
                    line: line_no,
                    kind: WarningKind::MisplacedConfigKey,
                    message: format!(
                        "{key:?} is a syncflowy option; write it as syncflowy: {{{key}: {}}}{}",
                        value.trim(),
                        if honoured {
                            " (the file is paused anyway)"
                        } else {
                            " (ignored here)"
                        }
                    ),
                });
                other.push(line.to_owned());
                i += 1;
            }
            _ => {
                other.push(line.to_owned());
                i += 1;
            }
        }
    }
    let root = root.ok_or(ParseError::NoRoot)?;
    Ok(Parsed {
        fm: Frontmatter {
            root,
            config: st.config,
            generation: st.generation,
            generation_text: st.generation_text,
            paused: st.paused,
            force: st.force,
            other,
        },
        warnings,
        body_start: close + 1,
    })
}

/// See [`Frontmatter::render`].
pub(super) fn render(fm: &Frontmatter) -> String {
    let d = DocConfig::default();
    let c = &fm.config;
    let mut entries: Vec<String> = Vec::new();
    if c.headings != d.headings {
        entries.push(format!("headings: {}", c.headings));
    }
    if c.tags != d.tags {
        entries.push(format!("ids: {}", tag_style_name(c.tags)));
    }
    if c.checkboxes != d.checkboxes {
        entries.push(format!(
            "checkboxes: {}",
            checkbox_policy_name(c.checkboxes)
        ));
    }
    if c.inline.dates != d.inline.dates {
        entries.push(format!("dates: {}", c.inline.dates));
    }
    if let Some(depth) = c.max_depth {
        entries.push(format!("depth: {depth}"));
    }
    if fm.paused {
        entries.push("paused: true".to_owned());
    }
    if fm.force {
        entries.push("force: true".to_owned());
    }
    if let Some(g) = fm.generation {
        entries.push(format!("gen: {g}"));
    }
    let mut out = String::from("---\n");
    out.push_str("workflowy: ");
    out.push_str(&fm.root.raw);
    out.push('\n');
    if !entries.is_empty() {
        out.push_str("syncflowy: {");
        out.push_str(&entries.join(", "));
        out.push_str("}\n");
    }
    for l in &fm.other {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("---\n");
    out
}
