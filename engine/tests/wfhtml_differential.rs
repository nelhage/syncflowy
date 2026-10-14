//! Differential tests of the Rust `wfhtml` port against the Python oracle
//! (`src/syncflowy/wfhtml.py`), plus property tests.
//!
//! One oracle process (`engine/tests/fixtures/wfhtml/oracle.py`, run with
//! `uv run python -I` from the repository root) serves the whole test binary; each
//! input is one JSON line. Every divergence is a bug in the port; the quirks the port
//! reproduces on purpose are listed in the `wfhtml` module docs and are covered here
//! like any other input (the oracle has them too).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use proptest::prelude::*;
use proptest::test_runner::Config;
use serde_json::{Value, json};
use syncflowy::testsupport::repo_root;
use syncflowy::wfhtml::{
    Inline, append_typed_text, canonicalize, escape_text, is_well_formed, parse, serialize,
    to_plain_text,
};

// ---------------------------------------------------------------------------------
// The oracle process
// ---------------------------------------------------------------------------------

struct Oracle {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug, PartialEq, Eq)]
struct Answer {
    well_formed: bool,
    canonical: String,
    plain: String,
    append: Option<String>,
}

impl Oracle {
    fn start() -> Oracle {
        let mut child = Command::new("uv")
            .args([
                "run",
                "--quiet",
                "python",
                "-I",
                "engine/tests/fixtures/wfhtml/oracle.py",
            ])
            .current_dir(repo_root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn the wfhtml oracle with `uv run python` (is uv installed?)");
        let stdin = child.stdin.take().expect("oracle stdin");
        let stdout = BufReader::new(child.stdout.take().expect("oracle stdout"));
        Oracle {
            _child: child,
            stdin,
            stdout,
        }
    }

    fn query(&mut self, s: &str, typed: Option<&str>) -> Answer {
        let req = json!({"s": s, "typed": typed});
        writeln!(self.stdin, "{req}").expect("write to oracle");
        self.stdin.flush().expect("flush oracle stdin");
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).expect("read from oracle");
        assert!(n > 0, "oracle exited (input {s:?})");
        let v: Value = serde_json::from_str(&line).expect("oracle JSON");
        if let Some(e) = v.get("error") {
            panic!("oracle raised for {s:?}: {e}");
        }
        Answer {
            well_formed: v["well_formed"].as_bool().expect("well_formed"),
            canonical: v["canonical"].as_str().expect("canonical").to_owned(),
            plain: v["plain"].as_str().expect("plain").to_owned(),
            append: v.get("append").and_then(|a| a.as_str()).map(str::to_owned),
        }
    }
}

fn oracle(s: &str, typed: Option<&str>) -> Answer {
    static ORACLE: OnceLock<Mutex<Oracle>> = OnceLock::new();
    let m = ORACLE.get_or_init(|| Mutex::new(Oracle::start()));
    let mut o = m.lock().unwrap_or_else(|p| p.into_inner());
    o.query(s, typed)
}

fn rust(s: &str, typed: Option<&str>) -> Answer {
    Answer {
        well_formed: is_well_formed(s),
        canonical: canonicalize(s),
        plain: to_plain_text(s),
        append: typed.map(|t| append_typed_text(s, t)),
    }
}

// ---------------------------------------------------------------------------------
// Hand-written corpus
// ---------------------------------------------------------------------------------

/// Inputs covering every rule of wfhtml.py and of the parser, and every row of the
/// UI:E and UI:F tables of docs/workflowy/ui-findings.md.
const CORPUS: &[&str] = &[
    // --- UI:E1/E2 typed text, raw and as stored
    "a < b > c & d \"q\" 'q'",
    "a &lt; b &gt; c &amp; d &quot;q&quot; 'q'",
    "5 > 3",
    "5 &gt; 3",
    "AT&T",
    "AT&amp;T",
    "<b>not bold</b>",
    "&lt;b&gt;not bold&lt;/b&gt;",
    // --- UI:E3/E5/E8 stored values
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
    // --- UI:F1/F2 table
    "F01 <u>underline</u>",
    r#"F02 <span class="colored c-red">red span</span>"#,
    "F03 <script>alert(1)</script>after script",
    "F04 <div>in div</div>",
    "F05 line<br>break",
    r#"F06 <img src="x">img"#,
    r#"F07 <a href="javascript:alert(1)">js link</a>"#,
    "F08 <b>unclosed bold",
    "F09 <B>upper</B>",
    "F10 <strong>strong</strong>",
    "F11 x<!-- comment -->y",
    "F12 <mark>mark</mark>",
    "F13 a < b & c > d",
    "F14 a &amp; b &lt;tag&gt;",
    "F15 nb&nbsp;sp &#39;q&#39; &quot;dq&quot;",
    r#"F16 he said "hi""#,
    // --- tests/test_wfhtml.py vectors
    "",
    "plain",
    "<br/>",
    "a<br/>b",
    "<b>x</B>",
    "<b>x</i>",
    "<a href=x>y</a>",
    r#"<b class="x">w</b>"#,
    r#"<a href="u" target="_blank">w</a>"#,
    "<a>w</a>",
    r#"<span class="other">w</span>"#,
    "<em>w</em>",
    r#"<a href="  JavaScript:x">w</a>tail"#,
    r#"see <a href="u">a &amp; b</a> <b>x</b>"#,
    r#"<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>"#,
    "a < b & c",
    r#"<a href="https://x/">x</a>"#,
    "<b><i>w</i></b>",
    r#"<b><a href="u">w</a></b>"#,
    // --- references in text
    "&amp;",
    "&lt;",
    "&gt;",
    "&quot;",
    "&apos;",
    "&#39;",
    "&#x41;",
    "&#0;",
    "&#9;",
    "&#10;",
    "&#13;",
    "a&#13;b",
    "&#x7f;",
    "&#128;",
    "&#x85;",
    "&#xD7FF;",
    "&#xD800;",
    "&#xDFFF;",
    "&#xE000;",
    "&#xFFFD;",
    "&#xFFFE;",
    "&#xFFFF;",
    "&#x10000;",
    "&#x10FFFF;",
    "&#x110000;",
    "&#99999999999;",
    "&#0000065;",
    "&#x0000041;",
    "&#X41;",
    "&#65",
    "&#;",
    "&#x;",
    "&#xG;",
    "&#1a;",
    "&nbsp;",
    "&copy;",
    "&AMP;",
    "&Amp;",
    "&amp",
    "x &amp y",
    "&;",
    "&a-b;",
    "&\u{e9};",
    "& ",
    "&&",
    "&",
    "a & b",
    "&amp ;",
    // --- tags and names
    "<b>x</b>y</b>",
    "</b>",
    "</_wfroot>",
    "x</_wfroot>",
    "<_wfroot>x</_wfroot>",
    "<_wfroot/>",
    "< b>x</b>",
    "<b >x</b >",
    "<b\t\r\n>x</b\n>",
    "<b>x</ b>",
    "<b/>",
    "<b />",
    "<b/ >",
    "<b//>",
    "<1>x</1>",
    "<>",
    "</>",
    "<-a/>",
    "<.a/>",
    "<a-b.c_d:e/>",
    "<:x/>",
    "<x:y>t</x:y>",
    "<_/>",
    "<\u{e9}>t</\u{e9}>",
    "<\u{b7}/>",
    "<a\u{b7}/>",
    "<\u{1c5}/>",
    "<\u{4e2d}>t</\u{4e2d}>",
    "<\u{10000}/>",
    "<a\u{10000}/>",
    "<a\u{300}/>",
    "<\u{300}/>",
    "<\u{212a}/>",
    "<b>",
    "<b",
    "<",
    "a<b",
    "<b>x<i>y</b></i>",
    "<b><i><u><s><code>deep</code></s></u></i></b>",
    "<x><y><z>t</z></y></x>",
    "<b></b>",
    "<b>\u{2028}</b>",
    // --- attributes
    "<b a=\"1\">x</b>",
    "<b a='1'>x</b>",
    "<b a='\"'>x</b>",
    "<b a=\"'\">x</b>",
    "<b a>x</b>",
    "<b a=>x</b>",
    "<b a = '1' >x</b>",
    "<b a='1'c='2'>x</b>",
    "<b a='1' a='2'>x</b>",
    "<b a='1' A='2'>x</b>",
    "<b a='<'>x</b>",
    "<b a='>'>x</b>",
    "<b a='&amp;&lt;&gt;&quot;&apos;&#65;&#x42;'>x</b>",
    "<b a='&nbsp;'>x</b>",
    "<b a='x&y'>z</b>",
    "<b a='&#X41;'>z</b>",
    "<b a='&a-b;'>z</b>",
    "<b a='&#;'>z</b>",
    "<b a='&#0;'>z</b>",
    "<time a='x\ty\nz\r\nw\rv'>t</time>",
    "<time a='&#9;&#10;&#13;'>t</time>",
    "<b a='1>x</b>",
    "<b 1a='1'>x</b>",
    "<b a='\u{1}'>x</b>",
    "<b a='\u{FFFE}'>x</b>",
    "<b a=\"1\"/>",
    "<b a=\"1\"/ >",
    "<b xmlns='u' xmlns:p='v' p:q='w' xml:lang='en'>x</b>",
    // --- comments
    "<!---->",
    "<!--->",
    "<!-->",
    "<!-- a -- b -->",
    "<!-- a --->",
    "a<!--x-->b<!--y-->c",
    "<!-- x",
    "<!-- \u{1} -->",
    "<!-- <b> & -->",
    "<! -- x -->",
    "<!-x-->",
    "<b>x<!--c--></b>",
    // --- processing instructions
    "<?x?>",
    "<?x y?>",
    "a<?x y z?>b",
    "<?xml?>",
    "<?xml version='1.0'?>",
    "<?XML x?>",
    "<?xMl?>",
    "<?xmlx?>",
    "<?xml-stylesheet href='x'?>",
    "<?x?y?>",
    "<? x?>",
    "<?x",
    "<?x y",
    "<?1?>",
    "<?x \u{1}?>",
    // --- CDATA
    "<![CDATA[x]]>",
    "<![CDATA[<b>&amp;</b>]]>",
    "<![CDATA[a\r\nb\rc]]>",
    "<![CDATA[]]>",
    "<![CDATA[x",
    "<![cdata[x]]>",
    "<![CDATA[x]]>]]>",
    "<![CDATA[<x &>]]>",
    "<![CDATA[\u{1}]]>",
    "a<![CDATA[b]]>c",
    // --- markup declarations
    "<!DOCTYPE x>",
    "<!ELEMENT x ANY>",
    "<!",
    "<![",
    // --- characters in text
    "a]]>b",
    "a]]b",
    "a]>b",
    "]]]>",
    "a > b",
    "a ' b \" c",
    "a\r\nb\rc\n",
    "\r",
    "\t",
    "a\u{0}b",
    "a\u{1}b",
    "a\u{b}b",
    "a\u{1c}b",
    "a\u{7f}b",
    "a\u{85}b",
    "a\u{a0}b",
    "\u{feff}",
    "\u{fffe}",
    "\u{ffff}",
    "\u{fdd0}",
    "\u{1f600}",
    "\u{10ffff}",
    // --- canonical tag handling
    "<STRONG>x</STRONG>",
    "<Em>x</Em>",
    "<EM>x</EM>",
    "<CODE>x</CODE>",
    "<u id='1'>x</u>",
    "<A HREF='u'>x</A>",
    "<a href='1' HREF='2'>x</a>",
    "<a HREF='1' href='2'>x</a>",
    "<a href=''>x</a>",
    "<a href='\"<>&amp;'>x</a>",
    "<a href='JAVASCRIPT:x'>x</a>t",
    "<a href=' javascript:x'>x</a>t",
    "<a href='\u{a0}javascript:x'>x</a>t",
    "<a href='\u{3000}javascript:x'>x</a>t",
    "<a href='java script:x'>x</a>t",
    "<a href='javascript :x'>x</a>t",
    "<a href='jav&#x61;script:x'>x</a>t",
    "<a href='javascript'>x</a>t",
    "<a href='&#9;javascript:x'>x</a>t",
    "<a href='u'><b>x</b></a>",
    "<b><a>x</a></b>",
    "<a href='u'><a href='javascript:'>x</a>y</a>",
    "<span class='colored'>x</span>",
    "<span class='Colored'>x</span>",
    "<span class=' colored c-red'>x</span>",
    "<span class='\u{a0}colored x'>x</span>",
    "<span class='xcolored'>x</span>",
    "<span class='c-red colored'>x</span>",
    "<span CLASS='colored c-blue'>x</span>",
    "<span class='colored a' Class='b'>x</span>",
    "<span>x</span>",
    "<span class='colored \"q\" <'>x</span>",
    "<SPAN class='colored'>x</SPAN>",
    "<time>x</time>",
    "<TIME startYear='2026'>x</TIME>",
    "<time b='1' a='2' B='3'>x</time>",
    "<time a='&quot;&lt;&amp;'>x</time>",
    "<style>p{}</style>",
    "<p>para</p><p>two</p>",
    "<img src='x'/>after",
    "<br></br>",
    "<x/><y/>",
    // --- typing simulation shapes
    "<code>x</code>",
    "<b>x</b>y",
    "<b>x</b>\n",
    "<b>x</b>\n\n",
    "<time a='1'>x</time>",
    "<b><time a='1'>x</time></b>",
    "<span class='colored c-red'><a href='u'>x</a></span>",
    "<i><b>x</b></i>",
    "x\n",
    // --- to_plain_text fallback (malformed inputs)
    "<b>&copy; &hellip; &notit; &amp",
    "<b>&#128; &#0; &#x1; &#xD800; &#1114112;",
    "a<b x>c",
    "a>b<",
    "a<b<c>d",
    "<b>&abcdefghijklmnopqrstuvwxyzabcdefghij;",
    "<b>&#x1F600; &#x1f600 &#65x",
    "<b>&AMP &lt &gt; &quot",
    "<b>&#9;&#10;&#13;&#11;",
];

const TYPED: &[&str] = &[" z", "z", " & <b>\"'", ""];

#[test]
fn hand_corpus_matches_python() {
    assert!(CORPUS.len() >= 150, "corpus has {} entries", CORPUS.len());
    // The oracle really is the Python module (UI:F05 is malformed there).
    let f05 = oracle("F05 line<br>break", None);
    assert!(!f05.well_formed && f05.canonical.is_empty() && f05.append.is_none());
    let mut failures = Vec::new();
    for (i, s) in CORPUS.iter().enumerate() {
        let typed = TYPED[i % TYPED.len()];
        let want = oracle(s, Some(typed));
        let got = rust(s, Some(typed));
        if want != got {
            failures.push(format!(
                "input {s:?} typed {typed:?}\n  python: {want:?}\n  rust:   {got:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} divergences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------------
// Generated corpora
// ---------------------------------------------------------------------------------

/// Fragments for the "token soup" generator: rich in markup characters, tags,
/// entities and the constructs the parser handles specially.
const TOKENS: &[&str] = &[
    "<",
    ">",
    "&",
    "\"",
    "'",
    "/",
    "=",
    ";",
    "#",
    "x",
    "a",
    "b",
    "B",
    "z",
    "1",
    " ",
    "\n",
    "\r",
    "\t",
    "-",
    "?",
    "!",
    "[",
    "]",
    ":",
    "_",
    ".",
    "\u{e9}",
    "\u{b7}",
    "\u{a0}",
    "\u{1}",
    "\u{fffe}",
    "\u{1f600}",
    "<b>",
    "</b>",
    "<B>",
    "</B>",
    "<i>",
    "</i>",
    "<u>",
    "</u>",
    "<code>",
    "</code>",
    "<strong>",
    "</strong>",
    "<em>",
    "</em>",
    "<a href=\"",
    "<a href='",
    "javascript:",
    "</a>",
    "<a>",
    "<span class=\"colored c-red\">",
    "<span class='",
    "colored",
    "</span>",
    "<time startYear=\"2026\">",
    "<time ",
    "</time>",
    "<div>",
    "</div>",
    "<br>",
    "<br/>",
    "<x:y>",
    "</x:y>",
    "&amp;",
    "&lt;",
    "&gt;",
    "&quot;",
    "&apos;",
    "&nbsp;",
    "&copy;",
    "&#39;",
    "&#x41;",
    "&#X41;",
    "&#0;",
    "&#13;",
    "&#10;",
    "&#9;",
    "&#128;",
    "&amp",
    "<!--",
    "-->",
    "--",
    "<![CDATA[",
    "]]>",
    "<?",
    "?>",
    "<?x",
    "<?xml",
    "</_wfroot>",
    " href=",
    " class=",
    "=\"",
    "=\'",
];

fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(TOKENS), 0..24).prop_map(|v| v.concat())
}

/// Tokens mixed with arbitrary characters (any Unicode scalar value, so also
/// characters XML forbids and non-ASCII name characters).
fn char_soup() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => prop::sample::select(TOKENS).prop_map(str::to_owned),
        1 => any::<char>().prop_map(String::from),
        1 => prop::char::range('\u{80}', '\u{3000}').prop_map(String::from),
    ];
    prop::collection::vec(piece, 0..24).prop_map(|v| v.concat())
}

const TAGS: &[&str] = &[
    "b", "B", "i", "u", "s", "code", "strong", "STRONG", "em", "a", "A", "span", "SPAN", "time",
    "div", "br", "x:y", "_wfroot", "mark", "script",
];
const ATTR_NAMES: &[&str] = &[
    "href",
    "HREF",
    "class",
    "Class",
    "startYear",
    "startMonth",
    "id",
    "target",
    "x",
];
const VALUE_PIECES: &[&str] = &[
    "u",
    "https://e.com/a?b=1&c=2",
    "colored",
    "colored c-red",
    " ",
    "\u{a0}",
    "c-blue",
    "javascript:",
    " JavaScript:",
    "&amp;",
    "&lt;",
    "&quot;",
    "&apos;",
    "&#9;",
    "&#10;",
    "&#13;",
    "&#38;",
    "&#60;",
    "&amp;amp;",
    "&amp;nbsp;",
    "&",
    "\t",
    "\n",
    "\r\n",
    ">",
    "'",
    "\"",
    "<",
    "2026",
    "&nbsp;",
];
const TEXT_PIECES: &[&str] = &[
    "w",
    "a b",
    " ",
    "\n",
    "\r\n",
    "\r",
    "&amp;",
    "&lt;",
    "&gt;",
    "&quot;",
    "&apos;",
    "&#39;",
    "&#x1F600;",
    "&#13;",
    "'",
    "\"",
    ">",
    "]]",
    "<!--c-->",
    "<?p d?>",
    "<![CDATA[<&>]]>",
    "\u{e9}",
    "\u{2028}",
    "#t",
    "@u",
];

fn attr_value() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(VALUE_PIECES), 0..4).prop_map(|v| v.concat())
}

/// Mostly well-formed fragments (the quoting of attribute values is not checked, so
/// some are malformed), exercising canonical serialisation.
fn structured() -> impl Strategy<Value = String> {
    let leaf = prop::collection::vec(prop::sample::select(TEXT_PIECES), 0..3)
        .prop_map(|v| v.concat())
        .boxed();
    leaf.prop_recursive(4, 24, 4, |inner| {
        (
            prop::sample::select(TAGS),
            prop::collection::vec(
                (
                    prop::sample::select(ATTR_NAMES),
                    attr_value(),
                    any::<bool>(),
                ),
                0..3,
            ),
            prop::collection::vec(inner, 0..4),
            any::<bool>(),
        )
            .prop_map(|(tag, attrs, children, empty)| {
                let mut s = format!("<{tag}");
                for (k, v, dq) in attrs {
                    let q = if dq { '"' } else { '\'' };
                    s.push_str(&format!(" {k}={q}{v}{q}"));
                }
                if empty && children.is_empty() {
                    s.push_str("/>");
                } else {
                    s.push('>');
                    s.push_str(&children.concat());
                    s.push_str(&format!("</{tag}>"));
                }
                s
            })
            .boxed()
    })
}

fn typed() -> impl Strategy<Value = String> {
    prop::sample::select(TYPED).prop_map(str::to_owned)
}

/// `cases` test cases, or `WFHTML_CASES` from the environment (for long local runs).
fn config(cases: u32) -> Config {
    let cases = std::env::var("WFHTML_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(cases);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

proptest! {
    #![proptest_config(config(4000))]

    #[test]
    fn soup_matches_python(s in soup(), t in typed()) {
        prop_assert_eq!(rust(&s, Some(&t)), oracle(&s, Some(&t)), "input {:?}", s);
    }

    #[test]
    fn char_soup_matches_python(s in char_soup(), t in typed()) {
        prop_assert_eq!(rust(&s, Some(&t)), oracle(&s, Some(&t)), "input {:?}", s);
    }
}

proptest! {
    #![proptest_config(config(3000))]

    #[test]
    fn structured_matches_python(s in structured(), t in typed()) {
        prop_assert_eq!(rust(&s, Some(&t)), oracle(&s, Some(&t)), "input {:?}", s);
    }
}

// ---------------------------------------------------------------------------------
// Properties (no oracle)
// ---------------------------------------------------------------------------------

/// Inputs on which canonicalisation is knowingly not a fixed point (the oracle quirks
/// in the module docs): an attribute value containing `&`, tab, LF or CR, or text
/// containing CR (only reachable through `&#13;`).
fn has_known_quirk(nodes: &[Inline]) -> bool {
    nodes.iter().any(|n| match n {
        Inline::Text(t) => t.contains('\r'),
        Inline::Element {
            attrs, children, ..
        } => {
            attrs
                .iter()
                .any(|(_, v)| v.contains(['&', '\t', '\n', '\r']))
                || has_known_quirk(children)
        }
    })
}

fn check_canonical_fixed_point(s: &str) -> Result<(), TestCaseError> {
    let c = canonicalize(s);
    let tree = parse(s);
    if let Ok(f) = &tree {
        prop_assert_eq!(&serialize(f), &c);
        if has_known_quirk(f) {
            return Ok(());
        }
    } else {
        prop_assert_eq!(&c, "");
    }
    prop_assert!(
        is_well_formed(&c),
        "canonical {:?} of {:?} is malformed",
        c,
        s
    );
    prop_assert_eq!(&canonicalize(&c), &c, "not idempotent on {:?}", s);
    Ok(())
}

/// Plain text without characters XML forbids, and without CR (normalised to LF).
fn plain_text() -> impl Strategy<Value = String> {
    let chars = prop_oneof![
        any::<char>(),
        prop::sample::select(vec!['&', '<', '>', '"', '\'', ';', '#', ' ', '\n', ']']),
    ];
    prop::collection::vec(chars, 0..40).prop_map(|v| {
        v.into_iter()
            .filter(|&c| {
                matches!(c, '\t' | '\n' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(config(3000))]

    #[test]
    fn canonicalize_is_idempotent_and_well_formed_soup(s in soup()) {
        check_canonical_fixed_point(&s)?;
    }

    #[test]
    fn canonicalize_is_idempotent_and_well_formed_char_soup(s in char_soup()) {
        check_canonical_fixed_point(&s)?;
    }

    #[test]
    fn canonicalize_is_idempotent_and_well_formed_structured(s in structured()) {
        check_canonical_fixed_point(&s)?;
    }

    #[test]
    fn escaped_plain_text_round_trips(t in plain_text()) {
        let x = escape_text(&t);
        prop_assert!(is_well_formed(&x), "{:?}", x);
        prop_assert_eq!(&to_plain_text(&x), &t);
        prop_assert_eq!(&escape_text(&to_plain_text(&x)), &x);
        prop_assert_eq!(&canonicalize(&x), &x);
        prop_assert_eq!(parse(&x).unwrap(), if t.is_empty() { vec![] } else { vec![Inline::Text(t.clone())] });
    }
}

#[test]
fn cr_in_plain_text_is_normalised() {
    assert_eq!(to_plain_text(&escape_text("a\r\nb\rc")), "a\nb\nc");
}
