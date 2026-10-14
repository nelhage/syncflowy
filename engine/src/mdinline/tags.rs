//! Id tags and conflict tags (§4.4, §6.1, §9.1 guard 4).

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use super::chars::backslashes_before;
use super::{ConflictField, IdTag, LineTag, TagFlag, TagForm, TagMatch};
use crate::tree::ShortId;

// UNVERIFIED (our choice): keywords are case-sensitive and spaces/tabs are allowed inside
// the bracketed forms; Dataview itself is more lenient about key case.
const HEX: &str = "([0-9A-Fa-f]{12})((?:-[A-Za-z0-9]+)*)";

static PATTERNS: LazyLock<Vec<(Option<TagForm>, Regex)>> = LazyLock::new(|| {
    let p = |s: &str| Regex::new(&s.replace("HEX", HEX)).expect("tag pattern");
    vec![
        (Some(TagForm::BlockId), p(r"\^wf-HEX")),
        (Some(TagForm::Comment), p(r"%%[ \t]*wf:HEX[ \t]*%%")),
        (Some(TagForm::Dataview), p(r"\[wid::[ \t]*HEX[ \t]*\]")),
        (
            Some(TagForm::DataviewHidden),
            p(r"\(wid::[ \t]*HEX[ \t]*\)"),
        ),
        (Some(TagForm::Html), p(r"<!--[ \t]*wf:HEX[ \t]*-->")),
        (None, p(r"%%[ \t]*wf-conflict:(name|note)[ \t]*%%")),
    ]
});

fn parse_flags(s: &str) -> BTreeSet<TagFlag> {
    s.split('-')
        .filter(|f| f.eq_ignore_ascii_case(TagFlag::ReadOnly.as_str()))
        .map(|_| TagFlag::ReadOnly)
        .collect()
}

/// See [`super::find_tags`].
pub(crate) fn find_tags(line: &str) -> Vec<TagMatch> {
    let mut found: Vec<TagMatch> = Vec::new();
    for (form, re) in PATTERNS.iter() {
        for caps in re.captures_iter(line) {
            let m = caps.get(0).expect("whole match");
            if backslashes_before(line, m.start()) % 2 == 1 {
                continue; // escaped: `\^wf-…`, `\[wid:: …]`, …
            }
            let tag = match form {
                Some(form) => {
                    if *form == TagForm::BlockId {
                        // A block id ends at a word boundary: `^wf-<13 hex>` is not a tag.
                        let next = line[m.end()..].chars().next();
                        if next.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_') {
                            continue;
                        }
                    }
                    let Ok(short) = ShortId::parse(&caps[1]) else {
                        continue;
                    };
                    LineTag::Id(IdTag {
                        short,
                        flags: parse_flags(&caps[2]),
                        form: *form,
                    })
                }
                None => LineTag::Conflict(if &caps[1] == "name" {
                    ConflictField::Name
                } else {
                    ConflictField::Note
                }),
            };
            found.push(TagMatch {
                tag,
                range: m.range(),
            });
        }
    }
    found.sort_by_key(|t| (t.range.start, std::cmp::Reverse(t.range.end)));
    let mut out: Vec<TagMatch> = Vec::with_capacity(found.len());
    for t in found {
        if out.last().is_none_or(|p| p.range.end <= t.range.start) {
            out.push(t);
        }
    }
    out
}

/// See [`super::format_tag`].
pub(crate) fn format_tag(short: &ShortId, flags: &BTreeSet<TagFlag>, form: TagForm) -> String {
    let mut body = short.as_str().to_owned();
    for f in flags {
        body.push('-');
        body.push_str(f.as_str());
    }
    match form {
        TagForm::BlockId => format!("^wf-{body}"),
        TagForm::Comment => format!("%%wf:{body}%%"),
        TagForm::Dataview => format!("[wid:: {body}]"),
        TagForm::DataviewHidden => format!("(wid:: {body})"),
        TagForm::Html => format!("<!-- wf:{body} -->"),
    }
}

/// True if `s` contains a tag or an Obsidian comment delimiter `%%` (EC 4): such text
/// must never come out of `wf_to_md` unescaped.
pub(crate) fn has_hazard(s: &str) -> bool {
    s.contains("%%") || !find_tags(s).is_empty()
}
