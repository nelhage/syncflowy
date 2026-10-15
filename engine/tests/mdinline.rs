//! mdinline: every row of the §4.2 table, every escaping rule, tiers (§5.3), the §5.2
//! write refinement and the id-tag syntax (§4.4).

use std::collections::BTreeSet;

use syncflowy::mdinline::{
    ConflictField, FieldKind, IdTag, InlineConfig, LineTag, TagFlag, TagForm, Tier, classify,
    encode_edge_whitespace, escape_line_start, find_tags, format_conflict_tag, format_tag,
    line_is_block_safe, md_to_plain, md_to_wf, plain_to_md, trim_line, wf_to_md, write_value,
};
use syncflowy::tree::ShortId;
use syncflowy::wfhtml;

const NAME: FieldKind = FieldKind::Name;
const NOTE: FieldKind = FieldKind::Note;
const CODE: FieldKind = FieldKind::Code;

fn cfg() -> InlineConfig {
    InlineConfig::default()
}

fn no_dates() -> InlineConfig {
    InlineConfig { dates: false }
}

/// Assert the rendering of a WF value, its tier, and the EC round trip.
#[track_caller]
fn check(kind: FieldKind, cfg: &InlineConfig, wf: &str, md: &str, tier: Tier) {
    let got = wf_to_md(wf, kind, cfg);
    assert_eq!(got.text, md, "wf_to_md({wf:?})");
    assert_eq!(got.tier, tier, "tier of {wf:?}");
    if matches!(tier, Tier::Exact | Tier::Passthrough) {
        assert_eq!(md_to_wf(md, kind, cfg), wf, "md_to_wf({md:?})");
    }
    if kind != CODE {
        for line in got.text.split('\n') {
            assert!(line_is_block_safe(line), "unsafe line {line:?} for {wf:?}");
        }
    }
}

#[track_caller]
fn name(wf: &str, md: &str, tier: Tier) {
    check(NAME, &cfg(), wf, md, tier);
}

#[track_caller]
fn parses(md: &str, wf: &str) {
    assert_eq!(md_to_wf(md, NAME, &cfg()), wf, "md_to_wf({md:?})");
}

// ---------------------------------------------------------------------------------------
// §4.2 table, row by row
// ---------------------------------------------------------------------------------------

#[test]
fn row_text_entities() {
    // UI:E1/E2.
    name(
        "a &lt; b &gt; c &amp; d &quot;q&quot; 'q'",
        "a < b > c & d \"q\" 'q'",
        Tier::Exact,
    );
    name("5 &gt; 3", "5 > 3", Tier::Exact);
    name("AT&amp;T", "AT&T", Tier::Exact);
    name(
        "&lt;b&gt;not bold&lt;/b&gt;",
        "\\<b>not bold\\</b>",
        Tier::Exact,
    );
    name("a &amp;amp; b", "a \\&amp; b", Tier::Exact);
    name(
        "x &amp;#42; y &amp;#x2a;",
        "x \\&#42; y \\&#x2a;",
        Tier::Exact,
    );
    name("&amp;copy;", "\\&copy;", Tier::Exact);
    name("a &amp; b; c", "a & b; c", Tier::Exact);
    // Both the escaped and the unescaped forms parse.
    parses("a &lt; b", "a &lt; b");
    parses("a < b", "a &lt; b");
    parses("\\<b>", "&lt;b&gt;");
    parses("&#42;x&#42;", "*x*");
    parses("&#x2A;", "*");
    parses("&copy; &nbsp;", "&amp;copy; &amp;nbsp;");
    parses("&#0; &#13; &#10;", "&amp;#0; &amp;#13; &amp;#10;");
    assert_eq!(md_to_wf("a&#10;b", NOTE, &cfg()), "a\nb");
}

#[test]
fn row_formatting() {
    // UI:E3.
    name(
        "plain <b>WORD</b> plain",
        "plain **WORD** plain",
        Tier::Exact,
    );
    name("plain <i>WORD</i> plain", "plain *WORD* plain", Tier::Exact);
    name(
        "plain <s>WORD</s> plain",
        "plain ~~WORD~~ plain",
        Tier::Exact,
    );
    name(
        "plain <code>CODE</code> plain",
        "plain `CODE` plain",
        Tier::Exact,
    );
    name(
        "plain <b><i>WORD</i></b> plain",
        "plain **_WORD_** plain",
        Tier::Exact,
    );
    name("<i><b>x</b></i>", "*__x__*", Tier::Exact);
    name(
        "<b>bold <i>ital</i> bold</b>",
        "**bold *ital* bold**",
        Tier::Exact,
    );
    name("a<b>b</b>c", "a**b**c", Tier::Exact);
    name("<b>x</b>", "**x**", Tier::Exact);
    // `__` and `_` are accepted and normalised (§5.4).
    parses("__x__ and _y_", "<b>x</b> and <i>y</i>");
    parses("***x***", "<i><b>x</b></i>");
    parses("**a *b* c**", "<b>a <i>b</i> c</b>");
    parses("~~gone~~", "<s>gone</s>");
    parses("snake_case_name", "snake_case_name");
    parses("a * b * c", "a * b * c");
    parses("2*3*4", "2<i>3</i>4");
    parses("~x~ ~~~y~~~", "~x~ ~~~y~~~");
    parses("**unclosed", "**unclosed");
    parses("** spaced **", "** spaced **");
}

#[test]
fn row_formatting_needs_passthrough() {
    // Content that delimiter runs cannot hold.
    name("<b> x</b>", "<b> x</b>", Tier::Passthrough);
    name("<i>x </i>y", "<i>x </i>y", Tier::Passthrough);
    name("<b></b>", "<b></b>", Tier::Passthrough);
    name("a<i>.x</i>", "a<i>.x</i>", Tier::Passthrough);
    name("<code></code>", "<code></code>", Tier::Passthrough);
    name("<s>~</s>", "<s>~</s>", Tier::Passthrough);
}

#[test]
fn row_code_spans() {
    name("<code>a`b</code>", "``a`b``", Tier::Exact);
    name("<code>`x</code>", "`` `x ``", Tier::Exact);
    name("a <code>x``y</code>", "a ```x``y```", Tier::Exact);
    name(
        "<code>x``y</code>",
        "<code>x\\`\\`y</code>",
        Tier::Passthrough,
    );
    name("<code> x </code>", "`  x  `", Tier::Exact);
    name("<code> </code>", "` `", Tier::Exact);
    name("<code>*not em*</code>", "`*not em*`", Tier::Exact);
    name("<code>&lt;b&gt;</code>", "`<b>`", Tier::Exact);
    // A code span may not start a line with a fence of three backticks.
    name(
        "<code>a``b</code> x",
        "<code>a\\`\\`b</code> x",
        Tier::Passthrough,
    );
    // Tag-like code is passthrough so it can be escaped (EC 4).
    name(
        "<code>a %% b</code>",
        "<code>a %\\% b</code>",
        Tier::Passthrough,
    );
    parses("`a` `` b ` ``", "<code>a</code> <code>b `</code>");
    parses("`unclosed", "`unclosed");
    parses("`\\*x\\*`", "<code>\\*x\\*</code>");
}

#[test]
fn row_underline_span_passthrough() {
    // UI:E3 underline, UI:F02 colour.
    name(
        "plain <u>WORD</u> plain",
        "plain <u>WORD</u> plain",
        Tier::Passthrough,
    );
    name(
        "<span class=\"colored c-red\">red span</span>",
        "<span class=\"colored c-red\">red span</span>",
        Tier::Passthrough,
    );
    name("<u><b>x</b></u>", "<u>**x**</u>", Tier::Passthrough);
    name("<b><u>x</u></b>", "**<u>x</u>**", Tier::Passthrough);
    parses("<u>x</u>", "<u>x</u>");
    parses("<u>**x**</u>", "<u><b>x</b></u>");
    parses(
        "<span class='colored c-blue'>b</span>",
        "<span class=\"colored c-blue\">b</span>",
    );
    // Other inline HTML, unbalanced HTML and non-colour spans are literal text.
    parses("<div>x</div>", "&lt;div&gt;x&lt;/div&gt;");
    parses("<u>x", "&lt;u&gt;x");
    parses("x</u>", "x&lt;/u&gt;");
    parses("<span>x</span>", "&lt;span&gt;x&lt;/span&gt;");
    parses("<strong>x</strong>", "&lt;strong&gt;x&lt;/strong&gt;");
    parses("<b>x</b>", "<b>x</b>");
    parses("<u><i>x</u></i>", "<u>&lt;i&gt;x</u>&lt;/i&gt;");
}

#[test]
fn row_autolinks() {
    // UI:E5: the UI keeps a raw `&` in the href and escapes it in the text.
    name(
        r#"see <a href="https://example.com/a?b=1&c=2">https://example.com/a?b=1&amp;c=2</a>"#,
        "see https://example.com/a?b=1&c=2",
        Tier::Exact,
    );
    name(
        r#"<a href="https://example.net/x">https://example.net/x</a>"#,
        "https://example.net/x",
        Tier::Exact,
    );
    name(
        r#"<a href="https://example.net/x">https://example.net/x</a>, then"#,
        "https://example.net/x, then",
        Tier::Exact,
    );
    // Followed by something that would extend the URL: the angle form.
    name(
        r#"<a href="https://e.com/">https://e.com/</a>x"#,
        "<https://e.com/>x",
        Tier::Exact,
    );
    name(
        r#"(<a href="https://e.com">https://e.com</a>)"#,
        "(https://e.com)",
        Tier::Exact,
    );
    name(
        r#"<a href="mailto:a@b.c">mailto:a@b.c</a>"#,
        "<mailto:a@b.c>",
        Tier::Exact,
    );
    name(
        r#"<a href="https://e.com/*x*">https://e.com/*x*</a>"#,
        "<https://e.com/*x*>",
        Tier::Exact,
    );
    parses(
        "see https://example.com/a?b=1&c=2.",
        r#"see <a href="https://example.com/a?b=1&c=2">https://example.com/a?b=1&amp;c=2</a>."#,
    );
    parses(
        "<https://e.com>",
        r#"<a href="https://e.com">https://e.com</a>"#,
    );
    parses("xhttps://e.com", "xhttps://e.com");
    parses(
        "**https://e.com**",
        r#"<b><a href="https://e.com">https://e.com</a></b>"#,
    );
    // A URL in text that is not a link stays text.
    name(
        "see https://e.com here",
        "see https\\://e.com here",
        Tier::Exact,
    );
    name("xhttps://e.com", "xhttps://e.com", Tier::Exact);
}

#[test]
fn row_links() {
    // UI:E5 link tool.
    name(
        r#"click <a href="https://example.org/">here</a>"#,
        "click [here](https://example.org/)",
        Tier::Exact,
    );
    name(
        r#"<a href="https://e.com/a b">t</a>"#,
        "[t](<https://e.com/a b>)",
        Tier::Exact,
    );
    name(
        r#"<a href="https://en.wikipedia.org/wiki/Foo_(bar)">w</a>"#,
        "[w](<https://en.wikipedia.org/wiki/Foo_(bar)>)",
        Tier::Exact,
    );
    name(r#"<a href="x&lt;y>">t</a>"#, "[t](<x\\<y\\>>)", Tier::Exact);
    name(r#"<a href="">t</a>"#, "[t](<>)", Tier::Exact);
    name(
        r#"<a href="u">a [b] c</a>"#,
        "[a \\[b\\] c](u)",
        Tier::Exact,
    );
    name(r#"<a href="u"><b>b</b></a>"#, "[**b**](u)", Tier::Exact);
    name(r#"<a href="u"></a>"#, "[](u)", Tier::Exact);
    name(r#"<a href="a\b">t</a>"#, "[t](a\\\\b)", Tier::Exact);
    name(
        r#"<a href="u"><a href="v">in</a></a>"#,
        r#"[<a href="v">in</a>](u)"#,
        Tier::Passthrough,
    );
    name(
        r#"<a href="https://e.com/%%wf:8e1f00aa2b3c%%">t</a>"#,
        "[t](https://e.com/%\\%wf:8e1f00aa2b3c%\\%)",
        Tier::Exact,
    );
    parses(
        "[t](https://e.com/?a=1&amp;b=2)",
        r#"<a href="https://e.com/?a=1&b=2">t</a>"#,
    );
    parses("[t](<a b>)", r#"<a href="a b">t</a>"#);
    parses("[a [b] c](u)", r#"<a href="u">a [b] c</a>"#);
    parses("[a [b](c) d](e)", r#"[a <a href="c">b</a> d](e)"#);
    parses("[t](u \"title\")", "[t](u &quot;title&quot;)");
    parses("![alt](x.png)", "![alt](x.png)");
    parses("[t](javascript:alert(1))", "[t](javascript:alert(1))");
    parses(
        "[see https://e.com](u)",
        r#"<a href="u">see https://e.com</a>"#,
    );
    parses(
        "[<https://e.com>](u)",
        r#"<a href="u">&lt;https://e.com&gt;</a>"#,
    );
    parses(
        "<a href=\"v\">[t](u)</a>",
        r#"<a href="v"><a href="u">t</a></a>"#,
    );
    parses("[t] (u)", "[t] (u)");
    parses("[t]", "[t]");
}

#[test]
fn row_dates() {
    let d = r#"<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>"#;
    name(d, "📅 2026-01-15", Tier::Exact);
    let dt = r#"<time startYear="2026" startMonth="1" startDay="15" startHour="14" startMinute="30">Thu, Jan 15, 2026 at 2:30 PM</time>"#;
    name(dt, "📅 2026-01-15 14:30", Tier::Exact);
    name(
        r#"<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5, 2026</time>"#,
        "📅 2026-07-05",
        Tier::Exact,
    );
    name(
        r#"<time startYear="1" startMonth="1" startDay="1">Mon, Jan 1, 1</time>"#,
        "📅 0001-01-01",
        Tier::Exact,
    );
    // Text that does not match what the attributes regenerate: passthrough.
    let custom = r#"<time startYear="2026" startMonth="1" startDay="15">tomorrow</time>"#;
    name(custom, custom, Tier::Passthrough);
    let range = r#"<time startYear="2026" startMonth="1" startDay="15" endYear="2026" endMonth="1" endDay="16">Thu, Jan 15, 2026</time>"#;
    name(range, range, Tier::Passthrough);
    // A following time would be read as part of a date-only form.
    name(
        &format!("{d} 14:30"),
        &format!("{d} 14:30"),
        Tier::Passthrough,
    );
    // With dates off: passthrough, and `📅` is plain text.
    check(NAME, &no_dates(), d, d, Tier::Passthrough);
    check(
        NAME,
        &no_dates(),
        "📅 2026-01-15",
        "📅 2026-01-15",
        Tier::Exact,
    );
    // A literal `📅 2026-01-15` in text is escaped when dates are on.
    name("📅 2026-01-15", "&#128197; 2026-01-15", Tier::Exact);
    name("📅 soon", "📅 soon", Tier::Exact);
    parses("📅 2026-01-15", d);
    parses("📅 2026-1-15", "📅 2026-1-15");
    parses("📅 2026-02-30", "📅 2026-02-30");
    parses(
        "📅 2026-01-15 9:05 x",
        &format!(
            "{} x",
            r#"<time startYear="2026" startMonth="1" startDay="15" startHour="9" startMinute="5">Thu, Jan 15, 2026 at 9:05 AM</time>"#
        ),
    );
    parses("**📅 2026-01-15**", &format!("<b>{d}</b>"));
}

#[test]
fn row_unknown_elements_are_lossy() {
    // UI:F03/F04/F07/F09/F10/F11/F12: canonical serialisation changes these.
    name(
        "F03 <script>alert(1)</script>after",
        "F03 alert(1)after",
        Tier::Lossy,
    );
    name("<div>in div</div>", "in div", Tier::Lossy);
    name(
        r#"F07 <a href="javascript:alert(1)">js link</a>"#,
        "F07&#32;",
        Tier::Lossy,
    );
    name("<B>upper</B>", "**upper**", Tier::Lossy);
    name("<strong>strong</strong>", "**strong**", Tier::Lossy);
    name("x<!-- comment -->y", "xy", Tier::Lossy);
    name("he said \"hi\"", "he said \"hi\"", Tier::Lossy);
    name(
        r#"<a href='u' target="_blank">t</a>"#,
        "[t](u)",
        Tier::Lossy,
    );
}

#[test]
fn row_tags_and_mentions() {
    // UI:C2: tags are plain text, as in Obsidian.
    name("X #t", "X #t", Tier::Exact);
    name("#urgent call @alice", "#urgent call @alice", Tier::Exact);
    name("#a-b_c #x_y", "#a-b_c #x_y", Tier::Exact);
}

#[test]
fn malformed_is_read_only() {
    // UI:F05/F06/F08/F13/F15.
    for (wf, md) in [
        ("line<br>break", "linebreak"),
        ("<img src=\"x\">img", "img"),
        ("<b>unclosed bold", "unclosed bold"),
        ("a < b & c > d", "a  d"),
        ("nb&nbsp;sp &#39;q&#39;", "nb\u{a0}sp 'q'"),
    ] {
        check(NAME, &cfg(), wf, md, Tier::ReadOnly);
    }
    check(NAME, &cfg(), "- <b>x", "\\- x", Tier::ReadOnly);
}

#[test]
fn newline_in_name_is_read_only() {
    check(NAME, &cfg(), "a\nb", "a ⏎ b", Tier::ReadOnly);
    check(NAME, &cfg(), "<b>a\nb</b> c", "**a ⏎ b** c", Tier::ReadOnly);
    // In a note, the same value is exact.
    check(NOTE, &cfg(), "a\nb", "a\nb", Tier::Exact);
    // Newlines typed into a name become spaces.
    assert_eq!(md_to_wf("a\nb\r\nc", NAME, &cfg()), "a b c");
}

// ---------------------------------------------------------------------------------------
// Escaping (§4.2 last paragraph, EC 4, EC 5)
// ---------------------------------------------------------------------------------------

#[test]
fn line_leading_escapes_in_names() {
    for (wf, md) in [
        ("1. foo", "1\\. foo"),
        ("2) foo", "2\\) foo"),
        ("&gt; x", "\\> x"),
        ("# x", "\\# x"),
        ("## x", "\\## x"),
        ("[ ] x", "\\[ ] x"),
        ("[x] done", "\\[x] done"),
        ("- x", "\\- x"),
        ("* x", "\\* x"),
        ("+ x", "\\+ x"),
        ("---", "\\---"),
        ("***", "\\***"),
        ("___", "\\___"),
        ("==", "\\=="),
        ("```", "\\```"),
        ("~~~", "\\~~~"),
        ("[foo]: bar", "\\[foo]: bar"),
        ("-", "\\-"),
    ] {
        name(wf, md, Tier::Exact);
    }
    // Not structure: unescaped.
    for wf in [
        "#tag", "-x", "1.5 kg", "1990", "x - y", "a &gt; b", "-- x y",
    ] {
        let md = wf_to_md(wf, NAME, &cfg());
        assert_eq!(md.tier, Tier::Exact, "{wf}");
        assert!(!md.text.starts_with('\\'), "{wf} -> {}", md.text);
    }
}

#[test]
fn line_leading_escapes_in_notes() {
    check(
        NOTE,
        &cfg(),
        "intro\n- not a child\n1. not a list\n&gt; not a quote\n# not a heading\n---",
        "intro\n\\- not a child\n1\\. not a list\n\\> not a quote\n\\# not a heading\n\\---",
        Tier::Exact,
    );
    // UI:E8: the observed multi-line note.
    check(
        NOTE,
        &cfg(),
        "line one\nline two\n\nhas &lt;angle&gt; &amp; <b>bold</b>",
        "line one\nline two\n\nhas \\<angle> & **bold**",
        Tier::Exact,
    );
    check(NOTE, &cfg(), "<b>a\n- b</b>", "**a\n\\- b**", Tier::Exact);
    check(
        NOTE,
        &cfg(),
        "<b>a\n\nb</b>",
        "<b>a\n\nb</b>",
        Tier::Passthrough,
    );
    check(NOTE, &cfg(), "\n\n", "\n\n", Tier::Exact);
}

#[test]
fn edge_whitespace() {
    name(" lead", "&#32;lead", Tier::Exact);
    name("trail ", "trail&#32;", Tier::Exact);
    name("\tx", "&#9;x", Tier::Exact);
    name("   ", "&#32;&#32;&#32;", Tier::Exact);
    name("\u{a0}nbsp", "&#160;nbsp", Tier::Exact);
    name("a\\ ", "a\\\\&#32;", Tier::Exact);
    name("<b>x</b> ", "**x**&#32;", Tier::Exact);
    check(
        NOTE,
        &cfg(),
        "a\n  indented\nb ",
        "a\n&#32;&#32;indented\nb&#32;",
        Tier::Exact,
    );
    // Unencoded edge whitespace is not part of the value.
    parses("  x  ", "x");
    assert_eq!(md_to_wf(" a \n\tb", NOTE, &cfg()), "a\nb");
}

#[test]
fn inline_escapes() {
    for (wf, md) in [
        ("a * b", "a * b"),
        ("x_y_z", "x_y_z"),
        ("3 &gt; 2", "3 > 2"),
        ("2*3*4", "2\\*3\\*4"),
        ("*important", "*important"),
        ("*a and b*", "\\*a and b\\*"),
        ("done*", "done*"),
        ("it`s", "it`s"),
        ("it`s <code>x</code>", "it\\`s `x`"),
        ("_init_", "\\_init\\_"),
        ("__init__.py", "\\_\\_init\\_\\_.py"),
        ("a ~~ b", "a ~~ b"),
        ("~~x~~", "\\~\\~x\\~\\~"),
        ("~/home", "~/home"),
        ("a `b` c", "a \\`b\\` c"),
        ("a\\b", "a\\b"),
        ("a\\*", "a\\\\*"),
        ("end\\", "end\\\\"),
        ("[[Page]]", "[[Page]]"),
        ("![[embed]]", "![[embed]]"),
        ("[^1]", "[^1]"),
        ("[a](b)", "[a\\](b)"),
        ("[WIP] task", "[WIP] task"),
        ("==hl==", "==hl=="),
        ("a = b", "a = b"),
        ("$5", "$5"),
        ("$x$ and $y$", "$x$ and $y$"),
        // Prices could pair into Obsidian math: a `$` before a digit is escaped when
        // there is another `$`.
        ("$5 and $10", "\\$5 and \\$10"),
        ("$x$ costs $5", "$x$ costs \\$5"),
        ("%%comment%%", "%\\%comment%\\%"),
        ("100%", "100%"),
        ("50% off %", "50% off %"),
        ("x ^wf-8e1f00aa2b3c", "x \\^wf-8e1f00aa2b3c"),
        ("see ^abc", "see \\^abc"),
        ("x^2", "x^2"),
        ("[wid:: 8e1f00aa2b3c]", "\\[wid:: 8e1f00aa2b3c]"),
        ("(wid:: 8e1f00aa2b3c)", "\\(wid:: 8e1f00aa2b3c)"),
        (
            "&lt;!-- wf:8e1f00aa2b3c --&gt;",
            "\\<!-- wf:8e1f00aa2b3c -->",
        ),
        ("a &lt;3", "a <3"),
        ("a &lt; b", "a < b"),
        ("x&lt;y", "x\\<y"),
        ("Hello!", "Hello!"),
        ("wow![x]", "wow\\![x]"),
        ("#tag @who", "#tag @who"),
    ] {
        name(wf, md, Tier::Exact);
    }
}

#[test]
fn obsidian_only_syntax_is_literal() {
    // Obsidian syntax with no WF meaning is literal text in WF (§4.2) and is written back
    // as typed, so it keeps working in Obsidian after a sync (EC 5).
    for md in [
        "[[Page]]",
        "see [[Page|alias]]",
        "[[Page#Heading]] and [[Other]]",
        "![[embed]]",
        "![[img.png]]",
        "==hl==",
        "a ==highlight== b",
        "$math$",
        "$x$ and $y^2$",
        "[^1]",
        "text[^1]",
        "^[inline footnote]",
    ] {
        for kind in [NAME, NOTE] {
            let wf = md_to_wf(md, kind, &cfg());
            assert_eq!(wfhtml::to_plain_text(&wf), md, "{md}");
            let back = wf_to_md(&wf, kind, &cfg());
            assert_eq!(back.tier, Tier::Exact, "{md}");
            assert_eq!(back.text, md, "{md} must be re-rendered unescaped");
            assert_eq!(md_to_wf(&back.text, kind, &cfg()), wf);
        }
    }
    // A note line made of the highlight delimiter alone would be a setext underline: the
    // line-level pass still escapes it (EC 4).
    let wf = md_to_wf("x\n==", NOTE, &cfg());
    assert_eq!(wf_to_md(&wf, NOTE, &cfg()).text, "x\n\\==");
    // Obsidian comments would hide text (and our tag forms): still escaped (EC 4).
    let wf = md_to_wf("%%comment%%", NAME, &cfg());
    assert_eq!(wfhtml::to_plain_text(&wf), "%%comment%%");
    let back = wf_to_md(&wf, NAME, &cfg());
    assert_eq!(back.text, "%\\%comment%\\%");
    assert_eq!(md_to_wf(&back.text, NAME, &cfg()), wf);
    // An image is literal text in WF (it has no WF meaning, but would display as an
    // image): it stays escaped, and a `!` before a link stays escaped so the link is not
    // read as an image.
    name("![alt](x.png)", "\\![alt\\](x.png)", Tier::Exact);
    name(
        r#"wow!<a href="https://x.org">t</a>"#,
        "wow\\![t](https://x.org)",
        Tier::Exact,
    );
    // Inside a link's text brackets stay escaped (links cannot hold `[`/`]` unescaped).
    let wf = r#"<a href="https://x.org">[[Page]]</a>"#;
    let back = wf_to_md(wf, NAME, &cfg());
    assert_eq!(back.text, "[\\[\\[Page\\]\\]](https://x.org)");
    assert_eq!(md_to_wf(&back.text, NAME, &cfg()), wf);
}

// ---------------------------------------------------------------------------------------
// Code-block names
// ---------------------------------------------------------------------------------------

#[test]
fn code_kind() {
    check(CODE, &cfg(), "x = 1", "x = 1", Tier::Exact);
    check(CODE, &cfg(), "py\nx = 1", "py\nx = 1", Tier::Exact);
    check(
        CODE,
        &cfg(),
        "if a &lt; b &amp;&amp; c:",
        "if a < b && c:",
        Tier::Exact,
    );
    check(CODE, &cfg(), "**x** &amp;", "**x** &", Tier::Exact);
    check(CODE, &cfg(), "<b>x</b> = 1", "x = 1", Tier::Lossy);
    // A line that would close a ``` fence is still exact: mdblock writes a longer fence
    // (§5.3 tier 3 is only for values the dialect cannot hold).
    check(CODE, &cfg(), "a\n```\nb", "a\n```\nb", Tier::Exact);
    check(CODE, &cfg(), "a\n  ```x", "a\n  ```x", Tier::Exact);
    check(CODE, &cfg(), "```\n````", "```\n````", Tier::Exact);
    check(CODE, &cfg(), "<b>x", "x", Tier::ReadOnly);
    assert_eq!(
        md_to_wf("a < b\r\n\"q\"", CODE, &cfg()),
        "a &lt; b\n&quot;q&quot;"
    );
}

// ---------------------------------------------------------------------------------------
// §5.2 writes
// ---------------------------------------------------------------------------------------

#[test]
fn write_value_naive_when_exact() {
    let w = write_value("plain <b>x</b>", "plain **x** y", NAME, &cfg());
    assert_eq!(w.wf, "plain <b>x</b> y");
    assert!(!w.lost);
    let w = write_value("old", "__new__", NAME, &cfg());
    assert_eq!(w.wf, "<b>new</b>");
    assert!(!w.lost);
}

#[test]
fn write_value_merges_onto_lossy_code() {
    // A code-block name with markup the fence cannot show.
    let base = "x = <b>1</b> + 2";
    assert_eq!(classify(base, CODE, &cfg()), Tier::Lossy);
    let w = write_value(base, "y = 1 + 2", CODE, &cfg());
    assert_eq!(w.wf, "y = <b>1</b> + 2");
    assert!(!w.lost);
    let w = write_value(base, "x = 1 + 3", CODE, &cfg());
    assert_eq!(w.wf, "x = <b>1</b> + 3");
    assert!(!w.lost);
    // An edit touching the markup cannot be merged: naive write, reported.
    let w = write_value(base, "x = 7 + 2", CODE, &cfg());
    assert_eq!(w.wf, "x = 7 + 2");
    assert!(w.lost);
}

#[test]
fn write_value_onto_noncanonical_base() {
    // API-written `<strong>` (UI:F10): the merge goes onto the canonical form.
    let w = write_value("<strong>bold</strong> text", "**bold** text!", NAME, &cfg());
    assert_eq!(w.wf, "<b>bold</b> text!");
    assert!(w.lost);
    // Unknown element: the UI would unwrap it anyway.
    let w = write_value("a <mark>b</mark> c", "a b c d", NAME, &cfg());
    assert_eq!(w.wf, "a b c d");
    assert!(w.lost);
}

#[test]
fn write_value_notes_merge_by_line() {
    // A note whose first line is not representable as rendered (a quirk value), with an
    // edit to another line.
    let base = "keep <B>this</B>\nsecond";
    let md = wf_to_md(base, NOTE, &cfg());
    assert_eq!(md.tier, Tier::Lossy);
    let edited = md.text.replace("second", "second, edited");
    let w = write_value(base, &edited, NOTE, &cfg());
    assert_eq!(w.wf, "keep <b>this</b>\nsecond, edited");
    assert!(w.lost);
}

#[test]
fn write_value_unchanged_markdown_keeps_base() {
    let base = "x = <b>1</b>";
    let md = wf_to_md(base, CODE, &cfg()).text;
    let w = write_value(base, &md, CODE, &cfg());
    assert_eq!(w.wf, base);
    assert!(!w.lost);
}

// ---------------------------------------------------------------------------------------
// Id tags (§4.4)
// ---------------------------------------------------------------------------------------

fn short() -> ShortId {
    ShortId::parse("8e1f00aa2b3c").unwrap()
}

#[test]
fn tag_formats() {
    let none = BTreeSet::new();
    let ro: BTreeSet<_> = [TagFlag::ReadOnly].into();
    assert_eq!(
        format_tag(&short(), &none, TagForm::BlockId),
        "^wf-8e1f00aa2b3c"
    );
    assert_eq!(
        format_tag(&short(), &ro, TagForm::BlockId),
        "^wf-8e1f00aa2b3c-ro"
    );
    assert_eq!(
        format_tag(&short(), &ro, TagForm::Comment),
        "%%wf:8e1f00aa2b3c-ro%%"
    );
    assert_eq!(
        format_tag(&short(), &none, TagForm::Dataview),
        "[wid:: 8e1f00aa2b3c]"
    );
    assert_eq!(
        format_tag(&short(), &none, TagForm::DataviewHidden),
        "(wid:: 8e1f00aa2b3c)"
    );
    assert_eq!(
        format_tag(&short(), &none, TagForm::Html),
        "<!-- wf:8e1f00aa2b3c -->"
    );
    assert_eq!(
        format_conflict_tag(ConflictField::Name),
        "%%wf-conflict:name%%"
    );
    assert_eq!(
        format_conflict_tag(ConflictField::Note),
        "%%wf-conflict:note%%"
    );
}

#[test]
fn tags_round_trip_through_find() {
    for form in [
        TagForm::BlockId,
        TagForm::Comment,
        TagForm::Dataview,
        TagForm::DataviewHidden,
        TagForm::Html,
    ] {
        for flags in [BTreeSet::new(), [TagFlag::ReadOnly].into()] {
            let tag = format_tag(&short(), &flags, form);
            let line = format!("- some text {tag}");
            let found = find_tags(&line);
            assert_eq!(found.len(), 1, "{line}");
            assert_eq!(&line[found[0].range.clone()], tag);
            assert_eq!(
                found[0].tag,
                LineTag::Id(IdTag {
                    short: short(),
                    flags: flags.clone(),
                    form
                })
            );
        }
    }
    for field in [ConflictField::Name, ConflictField::Note] {
        let line = format!("⚠ x {}", format_conflict_tag(field));
        assert_eq!(find_tags(&line)[0].tag, LineTag::Conflict(field));
    }
}

#[test]
fn find_tags_rules() {
    // Anywhere on the line, left to right, hex case-insensitive.
    let line = "a ^wf-8E1F00AA2B3C b %%wf:0c0ffee12345%% c";
    let found = find_tags(line);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].range, 2..18);
    let LineTag::Id(t) = &found[0].tag else {
        panic!()
    };
    assert_eq!(t.short, short());
    // Unknown flags are dropped.
    let found = find_tags("x ^wf-8e1f00aa2b3c-zz-ro");
    let LineTag::Id(t) = &found[0].tag else {
        panic!()
    };
    assert_eq!(t.flags, [TagFlag::ReadOnly].into());
    // A block link is a reference to a tag, not a tag (the form of a mirror stub, §15.1).
    assert!(find_tags("⧉ [[#^wf-8e1f00aa2b3c|Plan]]").is_empty());
    assert_eq!(
        find_tags("[[#^wf-8e1f00aa2b3c]] ^wf-0c0ffee12345-ro").len(),
        1
    );
    // Spaces inside the bracketed forms.
    assert_eq!(find_tags("%% wf:8e1f00aa2b3c %%").len(), 1);
    assert_eq!(find_tags("[wid::8e1f00aa2b3c]").len(), 1);
    assert_eq!(find_tags("<!--wf:8e1f00aa2b3c-->").len(), 1);
    // Not tags: escaped, too long, wrong keyword case, too short.
    for line in [
        "\\^wf-8e1f00aa2b3c",
        "%\\%wf:8e1f00aa2b3c%%",
        "\\[wid:: 8e1f00aa2b3c]",
        "\\(wid:: 8e1f00aa2b3c)",
        "\\<!-- wf:8e1f00aa2b3c -->",
        "^wf-8e1f00aa2b3cd",
        "^wf-8e1f00aa2b3",
        "^WF-8e1f00aa2b3c",
        "^wf-8e1f00aa2b3c_x",
        "%%wf-conflict:other%%",
    ] {
        assert!(find_tags(line).is_empty(), "{line}");
    }
    // An escaped backslash does not escape the tag.
    assert_eq!(find_tags("\\\\^wf-8e1f00aa2b3c").len(), 1);
}

#[test]
fn user_text_never_contains_a_tag() {
    for wf in [
        "^wf-8e1f00aa2b3c",
        "%%wf:8e1f00aa2b3c%%",
        "%%wf-conflict:name%%",
        "[wid:: 8e1f00aa2b3c]",
        "(wid:: 8e1f00aa2b3c)",
        "&lt;!-- wf:8e1f00aa2b3c --&gt;",
        "<b>^wf-8e1f00aa2b3c</b>",
        "<code>^wf-8e1f00aa2b3c</code>",
        "<u>%%</u>",
        "%%%",
    ] {
        for kind in [NAME, NOTE] {
            let md = wf_to_md(wf, kind, &cfg());
            assert!(find_tags(&md.text).is_empty(), "{wf} -> {}", md.text);
            assert!(!md.text.contains("%%"), "{wf} -> {}", md.text);
            assert!(matches!(md.tier, Tier::Exact | Tier::Passthrough), "{wf}");
            assert_eq!(md_to_wf(&md.text, kind, &cfg()), wf);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Helpers for mdblock
// ---------------------------------------------------------------------------------------

#[test]
fn helpers() {
    assert_eq!(
        plain_to_md("- a *b* <c>", NAME, &cfg()),
        "\\- a \\*b\\* \\<c>"
    );
    assert_eq!(plain_to_md("1. x\n# y", NOTE, &cfg()), "1\\. x\n\\# y");
    assert_eq!(md_to_plain("\\- **a** [b](u)", NAME, &cfg()), "- a b");
    assert_eq!(trim_line(" \t x \u{a0}"), "x");
    assert_eq!(escape_line_start("- x"), "\\- x");
    assert_eq!(encode_edge_whitespace(" x "), "&#32;x&#32;");
}
