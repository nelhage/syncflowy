//! Port of tests/test_wfhtml.py (the observed UI tables of docs/workflowy/ui-findings.md)
//! plus tests of the element-tree API that `mdinline` walks.

use syncflowy::wfhtml::{
    Inline, append_typed_text, canonicalize, escape_text, is_well_formed, parse, serialize,
    to_plain_text,
};

/// UI:E1/E2: text typed in the UI -> stored name.
const E_TYPED: &[(&str, &str)] = &[
    (
        "a < b > c & d \"q\" 'q'",
        "a &lt; b &gt; c &amp; d &quot;q&quot; 'q'",
    ),
    ("5 > 3", "5 &gt; 3"),
    ("AT&T", "AT&amp;T"),
    ("<b>not bold</b>", "&lt;b&gt;not bold&lt;/b&gt;"),
];

/// UI:E3/E5/E8: names and notes the UI produced; all well-formed and canonical.
const E_STORED: &[&str] = &[
    "plain <b>WORD</b> plain",
    "plain <i>WORD</i> plain",
    "plain <u>WORD</u> plain",
    "plain <s>WORD</s> plain",
    "plain <b><i>WORD</i></b> plain",
    "plain <code>CODE</code> plain",
    r#"see <a href="https://example.com/a?b=1&c=2">https://example.com/a?b=1&amp;c=2</a>"#,
    r#"click <a href="https://example.org/">here</a>"#,
    r#"<a href="https://example.net/x">https://example.net/x</a>"#,
    "line one\nline two\n\nhas &lt;angle&gt; &amp; <b>bold</b>",
    "X #t",
    r#"<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5, 2026</time>"#,
    r#"<span class="colored c-red">red span</span>"#,
];

/// UI:F1/F2: API-written name -> (renders?, canonical form after a UI save).
const F_TABLE: &[(&str, bool, &str)] = &[
    ("F01 <u>underline</u>", true, "F01 <u>underline</u>"),
    (
        r#"F02 <span class="colored c-red">red span</span>"#,
        true,
        r#"F02 <span class="colored c-red">red span</span>"#,
    ),
    (
        "F03 <script>alert(1)</script>after script",
        true,
        "F03 alert(1)after script",
    ),
    ("F04 <div>in div</div>", true, "F04 in div"),
    ("F05 line<br>break", false, ""),
    (r#"F06 <img src="x">img"#, false, ""),
    (
        r#"F07 <a href="javascript:alert(1)">js link</a>"#,
        true,
        "F07 ",
    ),
    ("F08 <b>unclosed bold", false, ""),
    ("F09 <B>upper</B>", true, "F09 <b>upper</b>"),
    ("F10 <strong>strong</strong>", true, "F10 <b>strong</b>"),
    ("F11 x<!-- comment -->y", true, "F11 xy"),
    ("F12 <mark>mark</mark>", true, "F12 mark"),
    ("F13 a < b & c > d", false, ""),
    (
        "F14 a &amp; b &lt;tag&gt;",
        true,
        "F14 a &amp; b &lt;tag&gt;",
    ),
    ("F15 nb&nbsp;sp &#39;q&#39; &quot;dq&quot;", false, ""),
    (r#"F16 he said "hi""#, true, "F16 he said &quot;hi&quot;"),
];

/// UI:F step 2 exactly as observed, where the user appended " z".
const F_APPEND_OBSERVED: &[(&str, &str)] = &[
    ("F01 <u>underline</u>", "F01 <u>underline z</u>"),
    (
        r#"F02 <span class="colored c-red">red span</span>"#,
        r#"F02 <span class="colored c-red">red span z</span>"#,
    ),
    (
        "F03 <script>alert(1)</script>after script",
        "F03 alert(1)after script z",
    ),
    ("F04 <div>in div</div>", "F04 in div z"),
    ("F05 line<br>break", " z"),
    (r#"F06 <img src="x">img"#, " z"),
    ("F09 <B>upper</B>", "F09 <b>upper z</b>"),
    ("F10 <strong>strong</strong>", "F10 <b>strong z</b>"),
    ("F11 x<!-- comment -->y", "F11 xy z"),
    ("F12 <mark>mark</mark>", "F12 mark z"),
    ("F13 a < b & c > d", " z"),
    ("F14 a &amp; b &lt;tag&gt;", "F14 a &amp; b &lt;tag&gt; z"),
    ("F15 nb&nbsp;sp &#39;q&#39; &quot;dq&quot;", " z"),
    (r#"F16 he said "hi""#, "F16 he said &quot;hi&quot; z"),
];

#[test]
fn escape_text_matches_ui() {
    for &(typed, stored) in E_TYPED {
        assert_eq!(escape_text(typed), stored);
        assert!(is_well_formed(stored), "{stored}");
        assert_eq!(to_plain_text(stored), typed);
    }
}

#[test]
fn ui_output_is_well_formed_and_canonical() {
    for &stored in E_STORED {
        assert!(is_well_formed(stored), "{stored}");
        assert_eq!(canonicalize(stored), stored);
    }
}

#[test]
fn f_table() {
    for &(name, renders, canonical) in F_TABLE {
        assert_eq!(is_well_formed(name), renders, "{name}");
        assert_eq!(canonicalize(name), canonical, "{name}");
        // canonicalize is idempotent
        assert_eq!(canonicalize(canonical), canonical, "{name}");
    }
}

#[test]
fn append_typed_text_matches_f_step2() {
    for &(name, after) in F_APPEND_OBSERVED {
        assert_eq!(append_typed_text(name, " z"), after, "{name}");
    }
}

#[test]
fn append_text_escapes_and_respects_links() {
    assert_eq!(append_typed_text("a", " & <b>"), "a &amp; &lt;b&gt;");
    // Typing after a link does not extend it (UNVERIFIED choice).
    assert_eq!(
        append_typed_text(r#"<a href="https://x/">x</a>"#, "!"),
        r#"<a href="https://x/">x</a>!"#
    );
    // Nested formatting runs are all extended.
    assert_eq!(
        append_typed_text("<b><i>w</i></b>", "z"),
        "<b><i>wz</i></b>"
    );
    // The text goes after the link but stays inside the enclosing bold run.
    assert_eq!(
        append_typed_text(r#"<b><a href="u">w</a></b>"#, "z"),
        r#"<b><a href="u">w</a>z</b>"#
    );
}

#[test]
fn entities() {
    for ok in [
        "&amp;", "&lt;", "&gt;", "&quot;", "&apos;", "&#39;", "&#x41;",
    ] {
        assert!(is_well_formed(ok), "{ok}");
    }
    for bad in ["&nbsp;", "&copy;", "&", "a & b", "&#0;"] {
        assert!(!is_well_formed(bad), "{bad}");
    }
}

#[test]
fn misc_well_formedness() {
    assert!(is_well_formed(""));
    assert!(is_well_formed("plain"));
    assert!(is_well_formed("<br/>")); // void tag closed XML-style: fine (then dropped)
    assert_eq!(canonicalize("a<br/>b"), "ab");
    assert!(!is_well_formed("<b>x</B>")); // case-mismatched close
    assert!(!is_well_formed("<b>x</i>"));
    assert!(!is_well_formed("<a href=x>y</a>")); // unquoted attribute
}

#[test]
fn canonicalize_attributes() {
    assert_eq!(canonicalize(r#"<b class="x">w</b>"#), "<b>w</b>");
    assert_eq!(
        canonicalize(r#"<a href="u" target="_blank">w</a>"#),
        r#"<a href="u">w</a>"#
    );
    assert_eq!(canonicalize("<a>w</a>"), "w");
    assert_eq!(canonicalize(r#"<span class="other">w</span>"#), "w");
    assert_eq!(canonicalize("<em>w</em>"), "<i>w</i>");
    assert_eq!(
        canonicalize(r#"<a href="  JavaScript:x">w</a>tail"#),
        "tail"
    );
}

#[test]
fn plain_text() {
    assert_eq!(
        to_plain_text(r#"see <a href="u">a &amp; b</a> <b>x</b>"#),
        "see a & b x"
    );
    assert_eq!(
        to_plain_text(
            r#"<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>"#
        ),
        "Thu, Jan 15, 2026"
    );
    // Malformed input: best-effort salvage (the UI itself would show nothing).
    assert_eq!(to_plain_text("F08 <b>unclosed bold"), "F08 unclosed bold");
    assert_eq!(to_plain_text("a < b & c"), "a < b & c");
}

// ---------------------------------------------------------------------------------
// Element-tree API
// ---------------------------------------------------------------------------------

fn text(s: &str) -> Inline {
    Inline::Text(s.to_owned())
}

fn el(tag: &str, attrs: &[(&str, &str)], children: Vec<Inline>) -> Inline {
    Inline::Element {
        tag: tag.to_owned(),
        attrs: attrs
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        children,
    }
}

#[test]
fn parse_builds_decoded_tree() {
    let f = parse(r#"a &amp; <B Class="x">b<!--c-->c<![CDATA[<d>]]></B><br/>&#x41;"#).unwrap();
    assert_eq!(
        f,
        vec![
            text("a & "),
            el("B", &[("Class", "x")], vec![text("bc<d>")]),
            el("br", &[], vec![]),
            text("A"),
        ]
    );
    assert_eq!(parse("").unwrap(), vec![]);
    assert_eq!(parse("<!--x-->").unwrap(), vec![]);
    // Attribute values: decoded, normalised, raw '&' fixed up (UI:E5).
    let f = parse("<a href='x?b=1&c=2&amp;d\te\r\nf&#10;'>t</a>").unwrap();
    assert_eq!(
        f,
        vec![el("a", &[("href", "x?b=1&c=2&d e f\n")], vec![text("t")])]
    );
    // End-of-line handling in text.
    assert_eq!(parse("a\r\nb\rc").unwrap(), vec![text("a\nb\nc")]);
}

#[test]
fn parse_errors_carry_an_offset() {
    let e = parse("ab <b>x</i>").unwrap_err();
    assert_eq!(e.offset, 7);
    assert!(e.message.contains("mismatched"), "{e}");
    let e = parse("x &nbsp;").unwrap_err();
    assert_eq!(e.offset, 2);
    assert!(e.to_string().contains("undefined entity"), "{e}");
}

#[test]
fn serialize_is_canonicalize() {
    for s in [
        "F09 <B>upper</B>",
        r#"<a href="u" HREF="v">x</a>"#,
        r#"<span CLASS="colored c-blue" class="colored c-red">x</span>"#,
        r#"<time startYear="2026" x='a"b'>d</time>"#,
        "<x><y>a &lt; b</y></x>",
    ] {
        assert_eq!(serialize(&parse(s).unwrap()), canonicalize(s), "{s}");
    }
}

#[test]
fn inline_accessors() {
    let b = el("STRONG", &[("Class", "x")], vec![text("w")]);
    assert_eq!(b.canonical_tag().as_deref(), Some("b"));
    assert_eq!(b.canonical_start_tag().as_deref(), Some("<b>"));
    assert_eq!(b.attr("Class"), Some("x"));
    assert_eq!(b.attr("class"), Some("x"));
    assert_eq!(b.attr("CLASS"), Some("x"));
    assert_eq!(b.attr("id"), None);
    assert!(!b.is_dropped());

    let a = el("a", &[("href", r#"u?a=1&b="2"<"#)], vec![]);
    assert_eq!(
        a.canonical_start_tag().as_deref(),
        Some(r#"<a href="u?a=1&b=&quot;2&quot;&lt;">"#)
    );
    let js = el("A", &[("HREF", " javascript:x")], vec![text("t")]);
    assert_eq!(js.canonical_tag().as_deref(), Some("a"));
    assert_eq!(js.canonical_start_tag(), None);
    assert!(js.is_dropped());
    assert_eq!(el("a", &[], vec![]).canonical_start_tag(), None);

    let span = el("span", &[("class", "colored c-red")], vec![]);
    assert_eq!(
        span.canonical_start_tag().as_deref(),
        Some(r#"<span class="colored c-red">"#)
    );
    assert_eq!(
        el("span", &[("class", "c-red colored")], vec![]).canonical_start_tag(),
        None
    );
    let time = el("time", &[("startYear", "2026"), ("x", "<")], vec![]);
    assert_eq!(
        time.canonical_start_tag().as_deref(),
        Some(r#"<time startYear="2026" x="&lt;">"#)
    );
    assert_eq!(el("div", &[], vec![]).canonical_start_tag(), None);
    assert_eq!(
        el("Div", &[], vec![]).canonical_tag().as_deref(),
        Some("div")
    );

    let t = text("x");
    assert_eq!(t.canonical_tag(), None);
    assert_eq!(t.canonical_start_tag(), None);
    assert_eq!(t.attr("x"), None);
    assert!(!t.is_dropped());
}

/// Python quirks kept on purpose (see the module docs).
#[test]
fn documented_oracle_quirks() {
    // A raw '&' is kept in attribute values, so decoded reference-shaped text in an
    // href is not a fixed point (and may even become malformed).
    assert_eq!(
        canonicalize(r#"<a href="&amp;amp;">x</a>"#),
        r#"<a href="&amp;">x</a>"#
    );
    let out = canonicalize(r#"<a href="&amp;nbsp;">x</a>"#);
    assert_eq!(out, r#"<a href="&nbsp;">x</a>"#);
    assert!(!is_well_formed(&out));
    // Character references to CR survive canonicalisation raw.
    assert_eq!(canonicalize("a&#13;b"), "a\rb");
    assert_eq!(canonicalize("a\rb"), "a\nb");
    // _fix_attr_amps also rewrites '&' inside CDATA that looks like a start tag.
    assert_eq!(to_plain_text("<![CDATA[<x &>]]>"), "<x &amp;>");
    // Python's `$` matches before a final newline.
    assert_eq!(append_typed_text("<b>x</b>\n", "Z"), "<b>xZ</b>\n");
}
