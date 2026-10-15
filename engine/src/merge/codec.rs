//! The text conversions the merge needs, behind a trait so that the merge can be tested
//! (and used by dependants' tests) independently of the inline dialect.
//!
//! [`MdInlineCodec`] is the production codec: it delegates to [`crate::mdinline`] and
//! [`crate::wfhtml`]. [`PlainTextCodec`] is a deliberately tiny dialect for tests: Markdown
//! is plain text, WF-HTML is that text with `& < > "` escaped. Both satisfy the contract
//! the merge relies on: `md_to_wf` is total and its output passes `is_canonical`.

use crate::mdinline::{self, FieldKind, InlineConfig, Tier, WriteValue};
use crate::wfhtml;

/// Text conversions used by [`super::merge_with`]. Implementations must be pure.
pub trait TextCodec {
    /// Markdown -> canonical WF-HTML for one field (`mdinline::md_to_wf`).
    fn md_to_wf(&self, md: &str, kind: FieldKind, cfg: &InlineConfig) -> String;
    /// The §5.2 write of an edited field (`mdinline::write_value`); `lost` also when the
    /// new Markdown holds something WF cannot ([`TextCodec::drops_text`]).
    fn write_value(
        &self,
        base_wf: &str,
        new_md: &str,
        kind: FieldKind,
        cfg: &InlineConfig,
    ) -> WriteValue;
    /// The representability tier of a WF value (`mdinline::classify`).
    fn classify(&self, wf: &str, kind: FieldKind, cfg: &InlineConfig) -> Tier;
    /// True if `wf` is well-formed and a fixed point of `wfhtml::canonicalize` (used to
    /// vet line-merged notes, §6.1, before pushing them).
    fn is_canonical(&self, wf: &str) -> bool;
    /// The visible text of a WF value (`wfhtml::to_plain_text`), for similarity (§4.4)
    /// and report labels.
    fn plain_text(&self, wf: &str) -> String;
    /// True if [`TextCodec::md_to_wf`] drops part of `md` that WF cannot hold (a link
    /// title, `mdinline::drops_link_title`): the write is reported as lossy.
    fn drops_text(&self, _md: &str, _kind: FieldKind, _cfg: &InlineConfig) -> bool {
        false
    }
}

/// The production codec: [`crate::mdinline`] and [`crate::wfhtml`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MdInlineCodec;

impl TextCodec for MdInlineCodec {
    fn md_to_wf(&self, md: &str, kind: FieldKind, cfg: &InlineConfig) -> String {
        mdinline::md_to_wf(md, kind, cfg)
    }

    fn write_value(
        &self,
        base_wf: &str,
        new_md: &str,
        kind: FieldKind,
        cfg: &InlineConfig,
    ) -> WriteValue {
        let mut w = mdinline::write_value(base_wf, new_md, kind, cfg);
        w.lost |= self.drops_text(new_md, kind, cfg);
        w
    }

    fn classify(&self, wf: &str, kind: FieldKind, cfg: &InlineConfig) -> Tier {
        mdinline::classify(wf, kind, cfg)
    }

    fn is_canonical(&self, wf: &str) -> bool {
        wfhtml::is_well_formed(wf) && wfhtml::canonicalize(wf) == wf
    }

    fn plain_text(&self, wf: &str) -> String {
        wfhtml::to_plain_text(wf)
    }

    fn drops_text(&self, md: &str, kind: FieldKind, cfg: &InlineConfig) -> bool {
        mdinline::drops_link_title(md, kind, cfg)
    }
}

/// A minimal codec for tests: Markdown text is taken literally, and its WF-HTML is the
/// text with `&`, `<`, `>` and `"` escaped (no inline markup at all). WF values that are
/// not exactly escaped plain text are [`Tier::Lossy`] (and names containing a newline
/// [`Tier::ReadOnly`]); [`TextCodec::write_value`] is the naive write, flagged `lost`
/// when the base value was not representable exactly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlainTextCodec;

impl TextCodec for PlainTextCodec {
    fn md_to_wf(&self, md: &str, _kind: FieldKind, _cfg: &InlineConfig) -> String {
        escape(md)
    }

    fn write_value(
        &self,
        base_wf: &str,
        new_md: &str,
        kind: FieldKind,
        cfg: &InlineConfig,
    ) -> WriteValue {
        let wf = escape(new_md);
        let lost = self.classify(base_wf, kind, cfg) > Tier::Passthrough && wf != base_wf;
        WriteValue { wf, lost }
    }

    fn classify(&self, wf: &str, kind: FieldKind, _cfg: &InlineConfig) -> Tier {
        if kind == FieldKind::Name && (wf.contains('\n') || wf.contains('\r')) {
            Tier::ReadOnly
        } else if escape(&plain_text_lossy(wf)) == wf {
            Tier::Exact
        } else {
            Tier::Lossy
        }
    }

    fn is_canonical(&self, wf: &str) -> bool {
        escape(&plain_text_lossy(wf)) == wf
    }

    fn plain_text(&self, wf: &str) -> String {
        plain_text_lossy(wf)
    }
}

/// Escape `& < > "` (the UI's text escaping, UI:E1).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Best-effort visible text of a WF value without the WF-HTML parser: tags removed,
/// the four entities the UI writes plus `&#39;`/`&apos;` decoded. Used for labels in
/// reports ([`super::ReportEvent::describe`] has no codec) and by [`PlainTextCodec`].
pub(crate) fn plain_text_lossy(wf: &str) -> String {
    let mut stripped = String::with_capacity(wf.len());
    let mut in_tag = false;
    for c in wf.chars() {
        match (in_tag, c) {
            (false, '<') => in_tag = true,
            (true, '>') => in_tag = false,
            (false, c) => stripped.push(c),
            (true, _) => {}
        }
    }
    let mut out = String::with_capacity(stripped.len());
    let mut rest = stripped.as_str();
    'outer: while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        for (ent, ch) in [
            ("&amp;", '&'),
            ("&lt;", '<'),
            ("&gt;", '>'),
            ("&quot;", '"'),
            ("&#39;", '\''),
            ("&apos;", '\''),
        ] {
            if let Some(after) = rest.strip_prefix(ent) {
                out.push(ch);
                rest = after;
                continue 'outer;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_codec_round_trips_text() {
        let c = PlainTextCodec;
        let cfg = InlineConfig::default();
        let wf = c.md_to_wf("a < b & \"c\"", FieldKind::Name, &cfg);
        assert_eq!(wf, "a &lt; b &amp; &quot;c&quot;");
        assert_eq!(c.plain_text(&wf), "a < b & \"c\"");
        assert!(c.is_canonical(&wf));
        assert_eq!(c.classify(&wf, FieldKind::Name, &cfg), Tier::Exact);
        assert_eq!(c.classify("<b>x</b>", FieldKind::Name, &cfg), Tier::Lossy);
        assert_eq!(c.classify("a\nb", FieldKind::Name, &cfg), Tier::ReadOnly);
        assert_eq!(c.classify("a\nb", FieldKind::Note, &cfg), Tier::Exact);
        let w = c.write_value("<b>x</b>", "y", FieldKind::Name, &cfg);
        assert_eq!(w.wf, "y");
        assert!(w.lost);
    }

    #[test]
    fn lossy_plain_text_strips_tags_and_entities() {
        assert_eq!(plain_text_lossy("<b>a &amp; b</b> &x"), "a & b &x");
    }
}
