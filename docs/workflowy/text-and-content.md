# Text content fidelity: `name` and `note`

Status: observed against production `https://workflowy.com/api/v1` on 2026-10-12
(about 770 requests, tag `text-content` in `.prod-state/requests.jsonl`).
Experiment scripts: `experiments/text-content/*.py`. Raw results:
`experiments/text-content/results/*.json`. Appendix tables are generated from those
results by `experiments/text-content/make_tables.py`.

Labels used here:

- **Observed**: seen directly in a response, and re-run where it was surprising.
- **Inferred**: a rule I fitted to the observations. It matches every case tried, but
  edge cases outside those cases may behave differently.

---

## TL;DR

1. **Three code paths behave differently**:
   - **Update** (`POST /api/v1/nodes/:id`), for both `name` and `note`: stored **verbatim**
     except that the C0 control characters `[\x00-\x08\x0b\x0c\x0e-\x1f]` are deleted.
     The server does no markdown parsing, entity decoding, trimming, newline handling or
     HTML sanitization.
   - **Create, `note`** (`POST /api/v1/nodes`): stored **completely verbatim**, control
     characters included. One exception: a NUL (`\x00`) in the note gives
     **HTTP 500** `internal_error`, and no node is created.
   - **Create, `name`**: this is the only path that transforms text. It strips control
     characters, decodes HTML entities, splits lines into blocks (some of which become
     **child nodes**), detects block prefixes (`#`, `- `, `- [ ] `, fences) and then
     runs a **naive regex-style inline markdown** pass (`**`, `*`, `~~`, `` ` ``,
     `[t](u)`, `[YYYY-MM-DD]`). The regexes run over the raw string and ignore any HTML
     already in it, including attribute values and `<code>` contents.
2. **No HTML sanitization anywhere.** `<script>`, `<u>`, `<span>`, `<div>`, `<img>`,
   `onclick=`, `javascript:` hrefs, unclosed or misnested tags and comments are all
   stored and returned byte-for-byte. Literal `<`, `>` and `&` are **not** escaped on
   output. The API returns what was stored.
3. **Dates** become `<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>`.
   With a time: `... startHour="14" startMinute="30">Thu, Jan 15, 2026 at 2:30 PM</time>`.
   Invalid dates are left as literal text.
4. **Tags, mentions and raw URLs have no special representation**: `#tag`, `@person`,
   `https://…` and `www.…` are plain text in the API, and URLs are **not** auto-linked.
5. **No length limit found**: 1,000,000-character names and notes round-trip exactly on
   both paths, and a 20 MB request body is accepted (it reached item lookup and got a
   404).
6. Several statements in the upstream docs are **wrong or incomplete** (see
   [Upstream doc discrepancies](#upstream-doc-discrepancies)). Examples: `a\n\nb` does
   **not** create a child; `> quote` does **not** produce `quote-block`; there is an
   undocumented `layoutMode` `"p"`.
7. Non-string `name` or `note` gives HTTP 400 with a pydantic-style body. `null` on
   update is **ignored**: the field is left unchanged.

---

## 1. Unicode

Observed (batch 01, batch 09): all non-control Unicode round-trips unchanged on all four
paths. That covers emoji, including ZWJ sequences and skin-tone modifiers, CJK,
Hebrew/Arabic RTL, astral-plane characters (U+1D518…, U+10FFFF), private use (U+E000),
noncharacters (U+FFFE, U+FFFF), U+FFFD, BOM U+FEFF, ZWSP/ZWNJ/ZWJ, bidi controls
(U+200E, U+202E, U+2066), soft hyphen U+00AD, NBSP U+00A0, DEL U+007F and C1 controls
U+0080–U+009F.

- **No Unicode normalization**: `e\u0301` (NFD) and `\u00e9` (NFC) stay distinct.
- Exception: line-separator code points in a **create name** (section 5).

## 2. Control characters

Observed (batch 09, input `a<ch>b`):

| code point | create name | create note | update name | update note |
|---|---|---|---|---|
| U+0000 NUL | `ab` (stripped) | **HTTP 500** `{"code":"internal_error","message":"An internal server error occurred"}`, no node created | `ab` | `ab` |
| U+0001–U+0008, U+000E–U+001F | stripped | kept verbatim | stripped | stripped |
| U+0009 TAB | kept (internal) | kept | kept | kept |
| U+000A LF, U+000D CR | line break (section 5) | kept | kept | kept |
| U+000B VT, U+000C FF | **line break, rest becomes a child node** | kept | **stripped** | **stripped** |
| U+0085 NEL, U+2028 LS, U+2029 PS | **line break, rest becomes a child node** | kept | kept | kept |
| U+007F, U+0080–U+009F | kept | kept | kept | kept |

Notes:

- The NUL→500 result was seen twice: batch 09, once for the note alone and once with
  NUL in both fields, and batch 10 with the NUL in the note only. A NUL in the **create
  name** alone is fine (stripped). Inferred: the create-note path writes the raw string
  to a store that rejects NUL (for example Postgres `text`), and nothing handles the
  error.
- Inferred: the update path deletes the XML-1.0-invalid C0 set `[\x00-\x08\x0b\x0c\x0e-\x1f]`.
  The create-name path deletes `[\x00-\x08\x0e-\x1f]`, keeping VT and FF, which its
  line splitter then treats as line breaks. The splitter's break set matches Python's
  `str.splitlines()`.
- U+001C–U+001E are also `splitlines()` separators, but the create name strips them
  (`a\x1cb`… → `ab`). So stripping happens before splitting.

## 3. Whitespace

Observed:

| input | create name | create note | update name / note |
|---|---|---|---|
| `"  padded  "` | `"padded"` | verbatim | verbatim |
| `"   "` | `""` | verbatim | verbatim |
| `"a    b"` | verbatim | verbatim | verbatim |
| `"a\tb\t"` | `"a\tb"` | verbatim | verbatim |
| `"\u00a0x\u00a0"` (NBSP) | `"x"` | (not tested) | (not tested) |
| `"\u3000x\u3000"`, `"\u2003x\u2003"` | `"x"` | – | – |
| `"\u200bx\u200b"` (ZWSP) | verbatim | – | – |
| `"\ufeffx\ufeff"` (BOM) | verbatim | – | – |
| `"a\u00a0b"` | verbatim | verbatim | verbatim |

Inferred: the create name, and each block in it, is trimmed with **Python `str.strip()`**.
It strips NBSP, U+3000 and U+2003 but **not** U+FEFF. JavaScript's `trim()` would strip
U+FEFF, which is why I think the server is Python; the pydantic error bodies point the
same way. Internal runs of whitespace are never collapsed.

## 4. HTML, entities and special characters

### 4.1 Literal `< > & " '`

Observed: stored and returned **unescaped and unchanged** on all paths. Examples:
`a < b > c & d`, `1<2 and 3>2`, `AT&T R&D` and `say "hi" it's`.

### 4.2 Entities

| input | create name | create note | update name / note |
|---|---|---|---|
| `&amp; &lt; &gt; &quot; &#39; &nbsp; &#x41; &copy;` | `& < > " ' \u00a0 A ©` | verbatim | verbatim |
| `&#128512; &#x1F600;` | `😀 😀` | – | – |
| `&foo; &ampx` | `&foo; &x` | – | – |
| `&amp;lt;` | `&lt;` (single pass) | – | – |
| `&lt;b&gt;x&lt;/b&gt;` | `<b>x</b>` (the text becomes a real tag) | – | – |
| `[t](https://e.com/?a=1&amp;b=2)` | `<a href="https://e.com/?a=1&b=2">t</a>` | – | – |

Inferred: the create name is passed through **Python `html.unescape()`**. It is the only
decoder with these semantics: legacy no-semicolon `&amp` is decoded inside `&ampx`, and
unknown entities are left alone. This runs **before** the markdown pass, as the next
table shows.

| input (create name) | output |
|---|---|
| `&#42;x&#42;` | `<i>x</i>` |
| `&#96;x&#96;` | `<code>x</code>` |
| `&#91;2026-01-15&#93;` | `<time …>Thu, Jan 15, 2026</time>` |
| `&lt;**x**&gt;` | `<<b>x</b>>` |

The update path and the create-note path **never** decode entities: `&amp;` stays
`&amp;`.

### 4.3 Tags (allowed and disallowed)

Observed (batch 02): **every** input below is returned byte-identical on all four paths.
The API does no sanitization, normalization or tag balancing.

`<b>`, `<i>`, `<s>`, `<code>`, `<a href="…&b=2">`, `<a href='…'>` (single quotes kept),
`<a … target class onclick>`, `<a>` with no href, `<a href="javascript:…">`,
`<b class style>`, `<b><i>bi</i></b>`, misnested `<b><i>bi</b></i>`, uppercase `<B>`,
`<strong>`, `<em>`, `<u>`, `<span class="colored c-red">`, `<span>`, `<script>…</script>`,
`<div>`, `<br>`, `<br/>`, `<img …>`, unclosed `<b>`, stray `</b>`, `a <notatag b`,
`<!-- comment -->`, `<time startYear=…>` (hand-written, kept as is), `<mark>`, `<p>`.

The allowed-tag list in the upstream docs describes what the **UI renders**. The API
does not enforce it. How the UI renders the other tags is an open question (see the UI
scenarios).

## 5. Create-path `name`: block structure and multi-line text

Upstream says: "A single `\n` is joined into a space; use `\n\n` to create separate
children." **This is not what happens.** Observed (batches 06, 07, 08):

| input (create name) | node `name` / layout | children (name / layout) |
|---|---|---|
| `a\nb` | `a b` / bullets | – |
| `a\n\nb` | `a b` / bullets | **none** |
| `a\n\n\n\nb` | `a b` | – |
| `a\nb\nc` | `a b` | `c` / **p** |
| `a\n\nb\n\nc` | `a b` | `c` / p |
| `a\nb\nc\nd` | `a b` | `c d` / p |
| `a\n\nb\n\nc\n\nd\n\ne` | `a b` | `c d` / p, `e` / p |
| `a\nb\n\n\nc\nd\ne` | `a b` | `c d`, `e` |
| `a\n   \nb\n \nc` (whitespace-only lines) | `a` | `b` / p, `c` / p |
| `a\r\nb`, `a\r\n\r\nb`, `a\rb` | `a b` | – |
| `\n\na` / `a\n\n` | `a` | – |
| `a  \nb` | `a   b` (3 spaces) | – |
| `- a\nb` | `a b` / bullets | – |
| `- a\nb\nc` | `a b` | `c` / p |
| `* a\nb` | `* a b` | – |
| `# H\nb\nc` | `H b` / h1 | `c` / p |
| `# Head\n\nbody **b**` | `Head body <b>b</b>` / h1 | – |
| `a\n\n- b` | `a` | `b` / bullets |
| `a\n- b\n- c` | `a` | `b` / bullets, `c` / bullets |
| `- one\n- two\n- three` | `one` / bullets | `two`, `three` / bullets |
| `- a\n  - b\n    - c` | `a` | `b`, `c` (**flat**, not nested) |
| `parent\n\n  - child\n\n    - grandchild` | `parent` | `child`, `grandchild` (flat) |
| `parent\n\n\tchild` | `parent` | `child` / p |
| `a\n\n# b` | `a` | `b` / h1 |
| `# A\n\n## B\n\n### C` | `A` / h1 | `B` / h2, `C` / h3 |
| `a\n\n- [ ] t\n\n- [x] d` | `a` | `t` / todo, `d` / todo completed |
| `a\n\n```\ncode\n```` | `a` | `code` / code-block |
| `a\n```b```` | `a` | `b` / code-block |
| `a\n\n> q` | `a` | `> q` / p |
| `* a\n* b` | `* a` / bullets | `* b` / p |
| `1. a\n2. b` | `1. a` / bullets | `2. b` / p |
| `a\n---` | `a` | `---` / p |
| `a\n===` | `a` | `===` / p |
| `---\nb` | `--- b` / bullets | – |
| `a\n\n---\n\nb` | `a` | `--- b` / p |
| `**a\nb**` | `<b>a b</b>` | – |
| `**a\n\nb\n\nc**` | `<i></i>a b` | `c<i></i>` / p |
| `a\n\n- **b** [2026-01-15] &amp;` | `a` | `<b>b</b> <time …>Thu, Jan 15, 2026</time> &` / bullets |
| `a\n\n-   b   ` | `a` | `  b` (leading spaces kept) / bullets |
| `a\x0bb`, `a\x0cb`, `a\x85b`, `a\u2028b`, `a\u2029b` | `a` | `b` / p |
| ```` ```\nx\n```\nafter ```` | `x\n``` after` / code-block | – |

All results were deterministic: `a\n\nb`, `a\n\nb\n\nc`, `#### H4` and `> quote` were
re-run in a second batch and gave the same output.

### 5.1 Inferred algorithm (matches every case above)

1. Strip `[\x00-\x08\x0e-\x1f]`. `html.unescape()`.
2. Split into lines on the `str.splitlines()` separators (`\r\n`, `\r`, `\n`, `\v`, `\f`,
   `\x1c-\x1e`, `\x85`, `\u2028`, `\u2029`). **Drop empty lines** (`""`). Keep
   whitespace-only lines.
3. Fenced code: a line starting with ```` ``` ```` opens a code block that runs to the
   closing fence. Its content keeps its `\n`s and **gets no inline markdown**. The info
   string is **not** removed: ```` ```py\nx = 1\n``` ```` gives the name `py\nx = 1`.
   ```` ```code``` ```` on one line gives `code` / code-block. The code-block node
   is not trimmed, apart from the fence lines.
4. Group the remaining lines into blocks. Each line starts a block. A block **absorbs
   at most one** following line, joined with a single space, if that line is not a
   "block start". Block starts are lines beginning with `-`, `*`, `+`, `N.`, `#`,
   ```` ``` ````, `---`, `===` or `>`. The first line of a block may itself be a block
   start (`- a\nb` gives `a b`, `---\nb` gives `--- b`). The absorbed line is only the
   next one. A third line starts a new block (`a\nb\nc` gives blocks `a b` and `c`).
   - This is why `a\n\nb` is **one** node: the empty line is dropped, then `a` absorbs
     `b`.
   - Whitespace-only lines are absorbable, which is why `a\n   \nb` gives `a` and `b`.
   - Lines split by VT/FF/NEL/LS/PS are **not** absorbed (`a\x0bb` gives `a` plus a
     child `b`). Inferred: absorption is a regex on `\n` that runs before the general
     `splitlines()` split.
5. Trim each block with `str.strip()`, then classify it by prefix (only these are
   recognized):

   | prefix (after strip) | layoutMode | completed | name = |
   |---|---|---|---|
   | `# ` | h1 | false | rest |
   | `## ` | h2 | false | rest |
   | `### ` | h3 | false | rest |
   | `#### ` | **p** | false | `<b>` + rest **including the leading space** + `</b>` (e.g. `<b> H4</b>`) |
   | `#####` or more, or `#x` without a space | – (plain) | | literal |
   | `- [ ] ` | todo | false | rest |
   | `- [x] ` (lowercase x only) | todo | **true** (completedAt = createdAt) | rest |
   | `- [X] ` | bullets | false | `[X] …` (the `- ` is eaten) |
   | `- ` | bullets | false | rest **without stripping further spaces** (`-   b` gives `  b`) |
   | fence | code-block | false | fence content |
   | anything else, including `* `, `+ `, `1. `, `> `, `---` | plain paragraph | false | literal |

6. A **plain paragraph** gets layoutMode **`"bullets"` if it is the first block** (the
   node itself) and **`"p"` if it is a child**.
7. Inline markdown (section 6) runs on each block separately.
8. The **first block** becomes the created node (`item_id`), placed at the requested
   `position` with normal sibling priorities. **Every other block becomes a direct
   child** of that node, in order. Nesting from indentation is never created. Children
   have:
   - `priority` **0, 1, 2, …** (not 100, 200, …), observed;
   - `note` **`""`** (empty string), not `null`, even when the parent's note is `null`;
   - `createdAt`/`modifiedAt` equal to the parent's;
   - `completed`/`completedAt` set for `- [x]`.

## 6. Create-path `name`: inline markdown

Observed (batches 03, 04, 05, 13). The update path and both note paths do **none** of
this: `**bold**` stays `**bold**`.

### 6.1 Supported syntax

| input | output |
|---|---|
| `x **bold** y` | `x <b>bold</b> y` |
| `x *it* y` | `x <i>it</i> y` |
| `x ~~st~~ y` | `x <s>st</s> y` |
| ``x `co*de*` y`` | `x <code>co<i>de</i></code> y` (code content is still markdown-parsed) |
| ``x `<b>` y`` | `x <code><b></code> y` (no escaping) |
| `x [text](https://example.com/a?b=1&c=2) y` | `x <a href="https://example.com/a?b=1&c=2">text</a> y` |
| `[rel](/foo)` | `<a href="/foo">rel</a>` |
| `[**b**](https://e.com)` | `<a href="https://e.com"><b>b</b></a>` |
| `**bold *ital* bold**` | `<b>bold <i>ital</i> bold</b>` |
| `**<i>x</i>**` | `<b><i>x</i></b>` |

### 6.2 Not supported (left literal)

`__bold__`, `_it_`, `~st~`, `<https://example.com>` (kept as literal text that looks like
a tag), raw `https://…` and `www.…` URLs (**no autolink**), `me@example.com`, `#tag`,
`@mention`, `#123`, `---`, `a # b`, `a - b`, `a > b`, `[just brackets]`.

### 6.3 Regex-like quirks (inferred: non-greedy regexes applied to the whole string)

| input | output | explanation |
|---|---|---|
| `***bi***` | `<b><i>bi</b></i>` (misnested) | bold `\*\*(.*?)\*\*` first, then italic |
| `**unclosed` | `<i></i>unclosed` | italic `\*(.*?)\*` matches the two adjacent `*` with empty content |
| `a **b c` | `a <i></i>b c` | same |
| `***x` | `<i></i>*x` | |
| `****x****` | `<b></b>x<b></b>` | bold allows empty content |
| `** spaced **` | `<b> spaced </b>` | no flanking rules |
| `* spaced *` | `<i> spaced </i>` | |
| `a * b` / `*unclosed` | unchanged | needs two `*` |
| `2*3*4`, `foo*bar*baz` | `2<i>3</i>4`, `foo<i>bar</i>baz` | intraword allowed |
| `~~~x~~~` | `<s>~x</s>~` | `~~(.*?)~~` |
| `~~unclosed` | unchanged | |
| ``` ``a ` b`` ``` | `<code></code>a <code> b</code>`` ` `` | `` `(.*?)` `` with empty allowed, no multi-backtick spans |
| `` `unclosed `` | unchanged | |
| ``**a `b** c` `` | `<b>a <code>b</b> c</code>` | passes do not respect each other |
| `[w](https://en.wikipedia.org/wiki/Foo_(bar))` | `<a href="https://en.wikipedia.org/wiki/Foo_(bar">w</a>)` | url `(.*?)\)` |
| `[text](https://example.com "Title")` | `<a href="https://example.com "Title"">text</a>` | href inserted raw, quotes not escaped, title not parsed |
| `[q](https://e.com/"x")` | `<a href="https://e.com/"x"">q</a>` | |
| `[q](https://e.com/<x>)` | `<a href="https://e.com/<x>">q</a>` | |
| `[](https://e.com)` | `<a href="https://e.com"></a>` | |
| `[t]()` | `<a href="">t</a>` | |
| `[t](a b)` | `<a href="a b">t</a>` | |
| `[a [b] c](https://e.com)` | `<a href="https://e.com">a [b] c</a>` | text `\[(.*?)\]\(` |
| `![alt](https://example.com/x.png)` | `!<a href="https://example.com/x.png">alt</a>` | no images |
| `https://e.com/*a*` | `https://e.com/<i>a</i>` | runs inside URLs |
| `https://e.com/~~x~~` | `https://e.com/<s>x</s>` | |
| `#a*b*c` | `#a<i>b</i>c` | |
| `<a href="https://e.com/*a*/[2026-01-15]">l</a>` | `<a href="https://e.com/<i>a</i>/<time …>Thu, Jan 15, 2026</time>">l</a>` | **runs inside HTML attributes**, which produces broken HTML |
| `<b title="**t**">x</b>` | `<b title="<b>t</b>">x</b>` | |
| `<code>*x* **y**</code>` | `<code><i>x</i> <b>y</b></code>` | `<code>` does not protect its content |
| `<b>[t](https://e.com)</b>` | `<b><a href="https://e.com">t</a></b>` | |

### 6.4 Backslash escapes

| input | output |
|---|---|
| `\*not italic\*` | `*not italic*` |
| `\[2026-01-15\]` | `[2026-01-15]` (no date) |
| `\[t](https://e.com)` | `[t](https://e.com)` (no link) |
| ``\`x\` `` | `` `x` `` |
| `\(x\)` | `(x)` |
| `a\]b` | `a]b` |
| ``` `\*x\*` ``` | `<code>*x*</code>` (escapes are processed inside code too) |
| `\~\~x\~\~`, `\_x\_`, `\!x`, `\>x`, `\-x`, `\.x`, `\# x`, `\<b\>`, `\a\b`, `a\\b`, `x\` | **unchanged**: the backslash is kept |
| `\&amp;` | `\&` (backslash kept, entity decoded) |
| `a\b \n \*` | `a\b \n *` |

Inferred: the escapable set is exactly `* [ ] \` ( )`. `\X` for one of these becomes a
placeholder before the inline passes and literal `X` after them. A double backslash
does **not** escape (`a\\b` is unchanged).

### 6.5 Order of passes (inferred)

1. Control-character strip, then `html.unescape` (section 2, section 4.2).
2. Block split and prefix detection (section 5).
3. Hide backslash escapes.
4. Inline passes. Constraints from the evidence:
   - **date before link**: `[2026-01-15](https://e.com)` gives `<time…>…</time>(https://e.com)`;
   - **bold before italic**: `***bi***`;
   - code, strike and link order relative to bold and italic could not be told apart in
     any test (each pass sees the earlier passes' HTML output as plain text). Suggested
     order: code, bold, italic, strike, date, link. Any order that puts date before
     link and bold before italic reproduces every observation.
5. Restore escapes.

## 7. Dates (`<time>`)

Observed (batches 03, 05). Create name only. Update and notes leave `[2026-01-15]` as
literal text, and a hand-written `<time>` element is stored verbatim.

| input | output |
|---|---|
| `[2026-01-15]` | `<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>` |
| `[2026-01-15 14:30]` | `<time startYear="2026" startMonth="1" startDay="15" startHour="14" startMinute="30">Thu, Jan 15, 2026 at 2:30 PM</time>` |
| `[2026-01-15 00:00]` | `… startHour="0" startMinute="0">Thu, Jan 15, 2026 at 12:00 AM</time>` |
| `[2026-01-15 12:00]` | `… startHour="12" startMinute="0">… at 12:00 PM</time>` |
| `[2026-01-15 09:05]` | `… startHour="9" startMinute="5">… at 9:05 AM</time>` |
| `[2026-01-15 9:05]` | same as above (1-digit hour accepted) |
| `[2026-01-15 23:59]` | `… at 11:59 PM` |
| `[2026-1-5]` | `<time startYear="2026" startMonth="1" startDay="5">Mon, Jan 5, 2026</time>` |
| `[2024-02-29]` | `Thu, Feb 29, 2024` |
| `[1999-12-31]` / `[9999-12-31]` | `Fri, Dec 31, 1999` / `Fri, Dec 31, 9999` |
| `[0001-01-01]` | `<time startYear="1" startMonth="1" startDay="1">Mon, Jan 1, 1</time>` |
| `[2026-13-45]`, `[2025-02-29]`, `[2026-04-31]` | literal (invalid dates are not converted) |
| `[2026-01-15 24:00]`, `[2026-01-15 12:60]` | literal |
| `[2026-01-15 12:00:30]`, `[2026-01-15T14:30]`, `[2026-01-15 2:30 PM]` | literal |
| `2026-01-15` (no brackets) | literal |
| `[2026-01-15] - [2026-01-20]` | two `<time>` elements (no range syntax) |
| `**[2026-01-15]**` | `<b><time …>…</time></b>` |
| `[[2026-01-15]](https://e.com)` | `<a href="https://e.com"><time …>…</time></a>` |

Format (inferred, matches all cases):

- Attributes: `startYear`, `startMonth` and `startDay` as integers without zero padding,
  in that order. Add `startHour` and `startMinute` (0–23, 0–59, unpadded) only when a
  time is given.
- Text: `"{Ddd}, {Mon} {D}, {Y}"` using English 3-letter weekday and month
  abbreviations (`Sep`, not `Sept`), proleptic Gregorian weekdays (year 1 → Monday;
  Python `datetime` gives this), unpadded day and year. With a time, append
  `" at {h}:{MM} {AM|PM}"` (12-hour clock, unpadded hour, 2-digit minute, 0 → 12 AM,
  12 → 12 PM).
- Pattern (inferred): `\[(\d{4})-(\d{1,2})-(\d{1,2})(?: (\d{1,2}):(\d{2}))?\]`, plus a
  calendar validity check (hour < 24, minute < 60). Not tested: 1–3 digit years,
  single-digit minutes, and whether the time text depends on the account's
  locale/timezone setting. The output above was the same as the input wall-clock time
  with no timezone shift.

## 8. Notes

- **Create**: verbatim, with no trim, markdown, entity decoding, newline processing or
  control-character stripping. The only failure is NUL → 500. Newlines (`\n`, `\r\n`,
  `\r`), leading and trailing newlines, and `# H\n- item\n**b**` are all kept exactly.
- **Update**: verbatim except that `[\x00-\x08\x0b\x0c\x0e-\x1f]` is stripped. `\r\n`
  and lone `\r` are preserved, not normalized.
- Absent `note` on create gives `null`. `"note": null` on create gives `null`.
  `"note": ""` on create gives `""`, which stays distinct from `null`.
- Child nodes made by a multi-block create name get `note: ""`.
- Once a note is set, the update API cannot return it to `null`: `null` is ignored and
  `""` gives `""` (observed, batch 10).

## 9. Lengths

Observed (batch 11): names and notes of 10,000, 100,000 and 1,000,000 characters
round-trip exactly (`equal == True`) on create and update. Batch 12 then deleted the
four 1M-character nodes so they would not slow down the user's real account. A 2 MB,
5 MB and 20 MB `{"name": "x"*N}` update to a random non-existent UUID returned the normal
`404 {"code":"not_found","message":"Item not found"}`, so no body-size limit was hit
before routing and lookup. A mock should **not** impose a length limit (or should use a
very generous one, ≥ 20 MB).

## 10. Types, nulls, empty strings

Observed (batch 10):

| request | status | body | effect |
|---|---|---|---|
| create `"name": 42` | 400 | `{"errors":[{"type":"string_type","loc":["name"],"msg":"Input should be a valid string","input":42}]}` | none |
| create `"note": 42` | 400 | same shape, `loc:["note"]` | none |
| update `"name": 123` | 400 | `{"errors":[{"type":"string_type","loc":["name"],"msg":"Input should be a valid string","input":123}]}` | none |
| update `"note": ["x"]` | 400 | `…"loc":["note"],…,"input":["x"]` | none |
| update `"note": null` | 200 `{"status":"ok"}` | | note **unchanged**; `modifiedAt` still bumped (833→834) |
| update `"name": null` | 200 | | name unchanged |
| update `"name": ""` | 200 | | name `""` |
| create `"name": ""` | 200 | | name `""` |
| create `"name": "   "` | 200 | | name `""` (trimmed) |

## 11. Consistency

- `GET /nodes/:id` and `GET /nodes?parent_id=` return identical `name` and `note`
  strings (checked on a node containing `<b>`, `<time>` and `<u>`).
- Every write was read back immediately (list or GET in the same script, well under a
  second later) and was always visible. No eventual-consistency effects appeared in
  text fields.
- Feeding a create-generated name, `<time>` included, back through update is a
  fixed point: the name is unchanged.

---

## Upstream doc discrepancies

| Upstream says | Observed |
|---|---|
| "use `\n\n` (double newline) to create separate children" | `a\n\nb` gives one node `a b`. Blank lines are dropped, and each line absorbs at most one following line (section 5) |
| "A single `\n` is joined into a space" | true only for the **first** following line. `a\nb\nc` gives `a b` plus child `c` |
| `> text` gives `quote-block` | **not recognized**: `> quote` gives name `> quote`, layout `bullets` (deterministic) |
| layoutMode values: bullets, todo, h1–h3, code-block, quote-block | create also produces **`"p"`** (paragraph children, and `#### ` headings) |
| `[YYYY-MM-DD]` gives "date" | gives `<time startYear=… startMonth=… startDay=…>Ddd, Mon D, YYYY</time>` |
| Name "Supports the following inline HTML tags: b, i, s, code, a" | API stores **any** HTML unmodified |
| Markdown table implies standard markdown | non-CommonMark regex parser: no `_`/`__`, broken nesting, runs inside HTML attributes, link titles leak into href |
| Fenced code block | info string is kept in the name (` ```py ` gives `py\n…`) |

---

## Mock server implementation notes

Store `name` and `note` as Python `str`, and return exactly what was stored. Never escape
or unescape on output.

```python
import html, re, datetime

UPDATE_STRIP = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f]")
CREATE_NAME_STRIP = re.compile(r"[\x00-\x08\x0e-\x1f]")

def validate_str(field, v):            # create + update
    if v is not None and not isinstance(v, str):
        raise HTTP400({"errors": [{"type": "string_type", "loc": [field],
                       "msg": "Input should be a valid string", "input": v}]})

def update_text(v):                    # update name and note
    return UPDATE_STRIP.sub("", v)     # None => leave field untouched (but bump modifiedAt)

def create_note(v):                    # create note
    if v is not None and "\x00" in v:
        raise HTTP500({"code": "internal_error",
                       "message": "An internal server error occurred"})  # create nothing
    return v                           # None/absent => null

def create_name(v):                    # -> list of (name, layoutMode, completed) blocks
    v = CREATE_NAME_STRIP.sub("", v)
    v = html.unescape(v)
    blocks = split_blocks(v)           # section 5.1; at least one block (may be "")
    return [render_block(b, is_first=(i == 0)) for i, b in enumerate(blocks)]
```

`split_blocks` (section 5.1):

1. Pull out fenced code blocks first. Their content is literal: no inline pass and no
   absorption.
2. Replace `\r\n` and `\r` with `\n`. Split lines on `\n`, drop `""` lines, and walk the
   list. A line takes the next line as its continuation (joined by `" "`) unless that
   line, after `lstrip`, starts with `-`, `*`, `+`, `\d+\.`, `#`, ```` ``` ````,
   `---`, `===` or `>`. After one absorption the block is closed.
3. Then split every block again on the remaining `splitlines()` separators (`\v`, `\f`,
   `\x85`, `\u2028`, `\u2029`) **without** absorption.
4. Every resulting segment is `str.strip()`ped.

`render_block`: prefix table (section 5.1, step 5), then inline markdown:

```python
ESC = {"*": "\x00E1", "[": "\x00E2", "]": "\x00E3", "`": "\x00E4", "(": "\x00E5", ")": "\x00E6"}
def inline(s):
    s = re.sub(r"\\([*\[\]`()])", lambda m: ESC[m[1]], s)
    s = re.sub(r"`(.*?)`", r"<code>\1</code>", s)
    s = re.sub(r"\*\*(.*?)\*\*", r"<b>\1</b>", s)
    s = re.sub(r"\*(.*?)\*", r"<i>\1</i>", s)
    s = re.sub(r"~~(.*?)~~", r"<s>\1</s>", s)
    s = re.sub(r"\[(\d{4})-(\d{1,2})-(\d{1,2})(?: (\d{1,2}):(\d{2}))?\]", time_elem, s)  # leave literal if invalid
    s = re.sub(r"\[(.*?)\]\((.*?)\)", r'<a href="\2">\1</a>', s)
    for k, v in ESC.items(): s = s.replace(v, k)
    return s
```

**Verified offline**: `experiments/text-content/model_check.py` implements exactly this
pipeline (strip, `html.unescape`, `strip()`, `inline`). It reproduces **191/191**
observed single-line create-name outputs from batches 01–05, 09 and 13 byte-for-byte,
including the quirks: `<i></i>unclosed`, misnested `***bi***`, link titles leaking into
the href, `&ampx`, and markdown inside attributes. `[a [b] c](u)` gives
`<a href="u">a [b] c</a>` because the lazy match extends past `]` until it finds
`](`.

Create placement: block 0 is the node returned as `item_id`. Blocks 1..n are inserted as
its children with `priority = 0..n-1`, `note = ""`, the same timestamps, and
`completed/completedAt` set for `- [x]`. Plain paragraphs get `layoutMode = "bullets"`
for block 0 and `"p"` for children. `#### x` gives `"<b> x</b>"` with layout `"p"` in
both positions.

What a mock must **not** do: sanitize HTML, balance tags, escape `<>&` on output,
normalize Unicode, collapse whitespace, limit length, autolink URLs, or parse
`#tags`/`@mentions` into anything.

Behaviour this doc has no evidence for: whether `layoutMode` passed explicitly on create
overrides the markdown prefix, and where `note` goes when a multi-block name creates
children. See the create area docs, or treat these as open questions.

---

## Open questions

1. **Where does `note` go** when the create name produces several blocks? Presumably on
   block 0. Untested.
2. Does an explicit `layoutMode` on create override a `# ` or `- [ ] ` prefix? Does
   `layoutMode: "p"` work on update? (Overlaps with create/update areas.)
3. Inside fenced code: are HTML entities still decoded? (Probably, since decoding comes
   first.) Is a single-line ```` ```**x**``` ```` markdown-parsed?
4. Date edge cases: 1–3 digit years (`[26-01-15]`), single-digit minutes
   (`[2026-01-15 9:5]`). Does the `<time>` text depend on the account's
   date/time-format or locale settings (24h clock users)?
5. Absorption rule details: can a whitespace-only line start a block and absorb a line
   (`a\nb\n   \nc`)? Exactly which prefixes count as "block start" (`+`, `1.` and `>`
   were inferred from their single-line behaviour, `+` only partly)? Does `-x`
   (dash without a space) block absorption?
6. Does the **UI** treat API-stored HTML the same way as UI-entered text? In particular,
   are literal `<`, `>` and `&` from the API shown as text, and how do UI-typed
   `<`/`&` come back through the API (escaped as `&lt;`/`&amp;`, or raw)? This decides
   whether `name` is "HTML" or "text with a few tags", which matters a lot for a sync
   engine. See the UI scenarios.
7. Does `modifiedAt` bump on every update even when nothing changes? (Seen for
   `note: null`. Overlaps with the update area.)
8. Is the 500 on a NUL in the create note a stable contract, or a bug that may get
   fixed? A mock should copy it for now.
9. A body-size limit above 20 MB, if any, was not probed.

## UI scenarios

A human performs each action in the Workflowy web UI inside a fresh child of the
scratchpad. Then we read the node(s) with `GET /api/v1/nodes?parent_id=…` and record
`name`, `note` and `data`.

1. **Literal special characters**: type `a < b > c & d "q" 'q'` as a bullet name.
   Observe: does the API return `&lt;`/`&amp;` or raw `<`/`&`?
2. **Typed tag-like text**: type `<b>not bold</b>` literally into a bullet. Observe the
   escaping, and whether it is distinguishable from a real bold run.
3. **Bold/italic/underline/strikethrough/code via UI shortcuts** (Ctrl+B, Ctrl+I,
   Ctrl+U, strikethrough, inline code). Observe the exact tags: `<b>` vs `<strong>`,
   and **what underline becomes** (`<u>`?).
4. **Text colour / highlight**: apply a colour and a highlight to part of a name.
   Observe the `<span class="colored c-…">` (or other) representation.
5. **Links**: paste a raw URL, and also create a link with custom text via the UI link
   tool. Observe whether a raw URL is auto-wrapped in `<a>` in the stored name.
6. **Dates**: insert a date via the date picker, a date with a time, and a date range if
   the UI supports one. Observe the `<time>` attributes (end dates? `endYear`?) and the
   text format. Also change the account's date-format or 24h preference, if one exists,
   and see whether the API text changes.
7. **Tags and mentions**: type `#urgent`, `@alice`, and a tag containing `-`/`_`.
   Observe whether the API name contains plain `#urgent` or some markup.
8. **Render API-written HTML**: open the API-created sandboxes
   `text: 02_html_tags (update path)` (`f6e10a18-3647-451d-9da8-be05b9824bd7`) and
   `(create path)` (`cf56792e-0c7f-4c7d-b0f8-0d86ac07f1ed`) and report how the UI shows
   `<u>`, `<span>`, `<script>`, `<div>`, `<br>`, `<img>`, `<a href="javascript:…">`, the
   unclosed `<b>`, `<B>`, `<strong>`, comments and `<mark>`. Then **edit one character**
   in a few of them (for example the `<script>` and `<span>` nodes) and re-read through
   the API. Does the UI rewrite or sanitize the stored HTML on save?
9. **Render API-written text with `<`/`&`**: in `text: 01_unicode_whitespace (update
   path)` (`f5e56ba9-…`), check how `a < b > c & d`, `&amp; &lt; …` and the control
   character nodes display. Edit one character and re-read to see whether the UI
   normalizes them (escapes `<`, decodes `&amp;`).
10. **Notes with newlines**: in the UI, type a multi-line note (Shift+Enter) containing
    blank lines, bold text and a link. Observe the note string: `\n` vs `<br>`, and the
    HTML in notes.
11. **Multi-line paste**: paste 3 lines of plain text into a bullet in the UI. Compare
    the resulting node structure with the API's create-path behaviour.
12. **Code block / quote block / headings / paragraph (`p`) nodes**: open the batch 06
    and 07 sandboxes (`2a52c23c-…`, `be9bca4e-…`) and report how `layoutMode: "p"`
    nodes, the `#### H4` node (`<b> H4</b>` / p) and the code-block named `py\nx = 1`
    look. Create a code block in the UI with several lines and check its `name`
    (embedded `\n`?).
13. **Leading and trailing whitespace**: in the UI, type a name with leading and
    trailing spaces, and an NBSP if possible. Observe whether the UI trims it.
14. **Emoji / RTL / combining characters typed in the UI**: check that they come back
    unchanged (expected).
15. **Long text**: paste roughly 100k characters into a note in the UI. Does it truncate
    or refuse? Compare with the API, which accepts 1M.
16. **Completed-via-markdown todo**: open the `- [x] done` node (batch 06) and confirm
    the UI shows a completed todo. Uncomplete it in the UI and observe `completedAt` and
    `name`.

---

## Sandboxes created (left in place)

All are children of the scratchpad `5e2e76e4-…-a846cfce4c6d`.

| sandbox | id |
|---|---|
| text: 01_unicode_whitespace (create path) | 0bc6a493-7d8e-4804-a4f1-e5176970010f |
| text: 01_unicode_whitespace (update path) | f5e56ba9-6c8a-4d65-93f3-405859111baf |
| text: 02_html_tags (create path) | cf56792e-0c7f-4c7d-b0f8-0d86ac07f1ed |
| text: 02_html_tags (update path) | f6e10a18-3647-451d-9da8-be05b9824bd7 |
| text: 03_markdown (create path) | 1a65c18d-c1bb-417e-80f4-6b1ab5b96f50 |
| text: 03_markdown (update path) | 2f5892f3-ef80-43e9-bb4b-7bb80a0ddfae |
| text: 04_md_parser_edges (create path) | cf4f1b73-cda1-4f82-bf3c-2abb338a6bba |
| text: 05_escapes_order_dates (create path) | 244722f7-8c49-426a-8c02-e02b11ad40f7 |
| text: 06 multiline/blocks (create name) | 2a52c23c-5d30-4e7b-a33e-e0ac72761fda |
| text: 06 multiline (create note) | 309c5b02-3c72-4688-81b6-8fb06c3431dc |
| text: 06 multiline (update) | a22cdd0b-9f91-46b4-a301-ddae224eb166 |
| text: 07 multiline deeper (create name) | be9bca4e-05e5-4ff5-b879-a4eccb0d2dff |
| text: 08 multiline rules (create name) | 3a959dbb-d2b5-42dd-b337-a74af415ced1 |
| text: 09_control_chars (create path) | 8062e4ee-eab4-4722-8c74-5a28f7e797f4 |
| text: 09_control_chars (update path) | 6a623be6-42d4-4732-ae2d-d46a62f4ca1a |
| text: 10 misc (NUL, nulls, GET vs list) | 21c99a90-d878-4a50-8d46-ff9286d79a8d |
| text: 11 long text (10k/100k kept; 1M nodes deleted) | see `.prod-state/requests.jsonl` |
| text: 13_trim_and_md_in_html (create path) | 94f8ae50-1797-4cff-bfd9-73460ea28b70 |

Side observation for the ordering area: in sandbox 10, nodes created with the default
`position` (top) got priorities 100, 50, 25, 12, 6, 3, 1, halving each time. Children
created by multi-block names get priorities 0, 1, 2, ….

---

## Appendix: full input→output tables

Generated from `experiments/text-content/results/*.json` by `make_tables.py`. "(same)"
means byte-identical to the input. Strings are JSON-escaped, and invisible or space-like
code points are written `\uXXXX`. Backticks inside values are shown as `ˋ`.

#### `01_unicode_whitespace`

| case | input | create name | create note | update name | update note |
|---|---|---|---|---|---|
| plain | `hello world` | (same) | (same) | (same) | (same) |
| emoji | `emoji 😀👍🏽👨\u200d👩\u200d👧 end` | (same) | (same) | (same) | (same) |
| cjk | `中文 日本語 한국어` | (same) | (same) | (same) | (same) |
| rtl | `עברית العربية mixed` | (same) | (same) | (same) | (same) |
| combining | `é vs é` | (same) | (same) | (same) | (same) |
| zero-width | `a\u200bb\u200cc\u200dd\ufeffe` | (same) | (same) | (same) | (same) |
| astral-math | `𝔘𝔫𝔦𝔠𝔬𝔡𝔢` | (same) | (same) | (same) | (same) |
| leading-trailing-space | `  padded  ` | `padded` | (same) | (same) | (same) |
| only-spaces | `   ` | `(empty)` | (same) | (same) | (same) |
| empty | `(empty)` | (same) | (same) | (same) | (same) |
| internal-multi-space | `a    b` | (same) | (same) | (same) | (same) |
| tab | `a\tb\t` | `a\tb` | (same) | (same) | (same) |
| nbsp | `a\u00a0b` | (same) | (same) | (same) | (same) |
| control-chars | `a\u0001b\u0007c\u007f` | `abc\u007f` | (same) | `abc\u007f` | `abc\u007f` |
| lt-gt-amp | `a < b > c & d` | (same) | (same) | (same) | (same) |
| quotes | `say \"hi\" it's` | (same) | (same) | (same) | (same) |
| lt-no-space | `1<2 and 3>2` | (same) | (same) | (same) | (same) |
| entity-amp | `&amp; &lt; &gt; &quot; &#39; &nbsp; &#x41; &copy;` | `& < > \" ' \u00a0 A ©` | (same) | (same) | (same) |
| bare-amp-word | `AT&T R&D` | (same) | (same) | (same) | (same) |
| backslash | `a\\b \\n \\*` | `a\\b \\n *` | (same) | (same) | (same) |

#### `02_html_tags`

| case | input | create name | create note | update name | update note |
|---|---|---|---|---|---|
| b | `x <b>bold</b> y` | (same) | (same) | (same) | (same) |
| i | `x <i>it</i> y` | (same) | (same) | (same) | (same) |
| s | `x <s>strike</s> y` | (same) | (same) | (same) | (same) |
| code | `x <code>a<b</code> y` | (same) | (same) | (same) | (same) |
| a | `x <a href=\"https://example.com/?a=1&b=2\">link</a> y` | (same) | (same) | (same) | (same) |
| a-single-quote-attr | `x <a href='https://example.com'>link</a> y` | (same) | (same) | (same) | (same) |
| a-extra-attrs | `<a href=\"https://example.com\" target=\"_blank\" class=\"z\" onclick=\"evil()\">l</a>` | (same) | (same) | (same) | (same) |
| a-no-href | `<a>nohref</a>` | (same) | (same) | (same) | (same) |
| a-js-href | `<a href=\"javascript:alert(1)\">js</a>` | (same) | (same) | (same) | (same) |
| b-with-attr | `<b class=\"x\" style=\"color:red\">bold</b>` | (same) | (same) | (same) | (same) |
| nested-bi | `<b><i>bi</i></b>` | (same) | (same) | (same) | (same) |
| nested-misnested | `<b><i>bi</b></i>` | (same) | (same) | (same) | (same) |
| uppercase-tag | `<B>up</B>` | (same) | (same) | (same) | (same) |
| strong-em | `<strong>st</strong> <em>em</em>` | (same) | (same) | (same) | (same) |
| u | `<u>under</u>` | (same) | (same) | (same) | (same) |
| span | `<span class=\"colored c-red\">sp</span>` | (same) | (same) | (same) | (same) |
| span-plain | `<span>sp</span>` | (same) | (same) | (same) | (same) |
| script | `a<script>alert(1)</script>b` | (same) | (same) | (same) | (same) |
| div | `<div>dv</div>` | (same) | (same) | (same) | (same) |
| br | `a<br>b<br/>c` | (same) | (same) | (same) | (same) |
| img | `<img src=\"x.png\" alt=\"pic\">tail` | (same) | (same) | (same) | (same) |
| unclosed-b | `<b>unclosed` | (same) | (same) | (same) | (same) |
| stray-close | `text</b> more` | (same) | (same) | (same) | (same) |
| lt-letter | `a <notatag b` | (same) | (same) | (same) | (same) |
| comment | `a<!-- c -->b` | (same) | (same) | (same) | (same) |
| time | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Jan 15, 2026</time>` | (same) | (same) | (same) | (same) |
| mark | `<mark>hl</mark>` | (same) | (same) | (same) | (same) |
| p | `<p>para</p>` | (same) | (same) | (same) | (same) |

#### `03_markdown`

| case | input | create name | create note | update name | update note |
|---|---|---|---|---|---|
| md-bold | `x **bold** y` | `x <b>bold</b> y` | (same) | (same) | (same) |
| md-bold-underscore | `x __bold__ y` | (same) | (same) | (same) | (same) |
| md-italic | `x *it* y` | `x <i>it</i> y` | (same) | (same) | (same) |
| md-italic-underscore | `x _it_ y` | (same) | (same) | (same) | (same) |
| md-strike | `x ~~st~~ y` | `x <s>st</s> y` | (same) | (same) | (same) |
| md-single-tilde | `x ~st~ y` | (same) | (same) | (same) | (same) |
| md-code | `x ˋco*de*ˋ y` | `x <code>co<i>de</i></code> y` | (same) | (same) | (same) |
| md-code-html | `x ˋ<b>ˋ y` | `x <code><b></code> y` | (same) | (same) | (same) |
| md-link | `x [text](https://example.com/a?b=1&c=2) y` | `x <a href=\"https://example.com/a?b=1&c=2\">text</a> y` | (same) | (same) | (same) |
| md-link-title | `x [text](https://example.com \"Title\") y` | `x <a href=\"https://example.com \"Title\"\">text</a> y` | (same) | (same) | (same) |
| md-link-relative | `[rel](/foo)` | `<a href=\"/foo\">rel</a>` | (same) | (same) | (same) |
| md-autolink-angle | `<https://example.com>` | (same) | (same) | (same) | (same) |
| raw-url | `see https://example.com/path?q=1 here` | (same) | (same) | (same) | (same) |
| raw-www | `see www.example.com here` | (same) | (same) | (same) | (same) |
| email | `mail me@example.com` | (same) | (same) | (same) | (same) |
| md-image | `![alt](https://example.com/x.png)` | `!<a href=\"https://example.com/x.png\">alt</a>` | (same) | (same) | (same) |
| md-bold-italic | `***bi***` | `<b><i>bi</b></i>` | (same) | (same) | (same) |
| md-nested | `**bold *ital* bold**` | `<b>bold <i>ital</i> bold</b>` | (same) | (same) | (same) |
| md-unclosed-bold | `**unclosed` | `<i></i>unclosed` | (same) | (same) | (same) |
| md-escaped | `\\*not italic\\*` | `*not italic*` | (same) | (same) | (same) |
| date | `due [2026-01-15] ok` | `due <time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time> ok` | (same) | (same) | (same) |
| datetime | `due [2026-01-15 14:30] ok` | `due <time startYear=\"2026\" startMonth=\"1\" startDay=\"15\" startHour=\"14\" startMinute=\"30\">Thu, Jan 15, 2026 at 2:30 PM</time> ok` | (same) | (same) | (same) |
| date-invalid | `due [2026-13-45] ok` | (same) | (same) | (same) | (same) |
| date-alone | `[2026-01-15]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time>` | (same) | (same) | (same) |
| date-bare | `2026-01-15` | (same) | (same) | (same) | (same) |
| date-range | `[2026-01-15] - [2026-01-20]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time> - <time startYear=\"2026\" startMonth=\"1\" startDay=\"20\">Tue, Jan 20, 2026…` | (same) | (same) | (same) |
| hashtag | `task #urgent #two-words #a_b` | (same) | (same) | (same) | (same) |
| mention | `ping @alice and @bob.smith` | (same) | (same) | (same) | (same) |
| hashtag-number | `issue #123` | (same) | (same) | (same) | (same) |
| md-heading-midline | `a # b` | (same) | (same) | (same) | (same) |
| md-list-dash-midline | `a - b` | (same) | (same) | (same) | (same) |
| md-hr | `---` | (same) | (same) | (same) | (same) |
| md-blockquote-inline | `a > b` | (same) | (same) | (same) | (same) |
| md-html-mixed | `**<i>x</i>**` | `<b><i>x</i></b>` | (same) | (same) | (same) |

#### `04_md_parser_edges`

| case | input | create name |
|---|---|---|
| single-star-unclosed | `*unclosed` | (same) |
| single-star-mid | `a * b` | (same) |
| star-math | `2*3*4` | `2<i>3</i>4` |
| star-intraword | `foo*bar*baz` | `foo<i>bar</i>baz` |
| double-star-unclosed-mid | `a **b c` | `a <i></i>b c` |
| triple-star-unclosed | `***x` | `<i></i>*x` |
| four-stars | `****x****` | `<b></b>x<b></b>` |
| bold-space-inside | `** spaced **` | `<b> spaced </b>` |
| italic-space-inside | `* spaced *` | `<i> spaced </i>` |
| two-bolds | `**a** and **b**` | `<b>a</b> and <b>b</b>` |
| bold-across-code | `**a ˋb** cˋ` | `<b>a <code>b</b> c</code>` |
| tilde-unclosed | `~~unclosed` | (same) |
| tilde-triple | `~~~x~~~` | `<s>~x</s>~` |
| backtick-unclosed | `ˋunclosed` | (same) |
| double-backtick | `ˋˋa ˋ bˋˋ` | `<code></code>a <code> b</code>ˋ` |
| backtick-with-entity | `ˋ&amp;ˋ` | `<code>&</code>` |
| link-nested-md | `[**b**](https://e.com)` | `<a href=\"https://e.com\"><b>b</b></a>` |
| link-url-with-parens | `[w](https://en.wikipedia.org/wiki/Foo_(bar))` | `<a href=\"https://en.wikipedia.org/wiki/Foo_(bar\">w</a>)` |
| link-url-with-quote | `[q](https://e.com/\"x\")` | `<a href=\"https://e.com/\"x\"\">q</a>` |
| link-url-with-lt | `[q](https://e.com/<x>)` | `<a href=\"https://e.com/<x>\">q</a>` |
| link-empty-text | `[](https://e.com)` | `<a href=\"https://e.com\"></a>` |
| link-empty-url | `[t]()` | `<a href=\"\">t</a>` |
| link-space-in-url | `[t](a b)` | `<a href=\"a b\">t</a>` |
| brackets-no-url | `[just brackets]` | (same) |
| link-text-brackets | `[a [b] c](https://e.com)` | `<a href=\"https://e.com\">a [b] c</a>` |
| escape-backslash-bracket | `\\[2026-01-15\\]` | `[2026-01-15]` |
| escape-backslash-backtick | `\\ˋx\\ˋ` | `ˋxˋ` |
| escape-double-backslash | `a\\\\b` | (same) |
| escape-backslash-letter | `\\a\\b` | (same) |
| escape-backslash-lt | `\\<b\\>` | (same) |
| escape-hash | `\\# not heading` | (same) |
| entity-numeric-big | `&#128512; &#x1F600;` | `😀 😀` |
| entity-unknown | `&foo; &ampx` | `&foo; &x` |
| entity-lt-tag | `&lt;b&gt;x&lt;/b&gt;` | `<b>x</b>` |
| entity-double | `&amp;lt;` | `&lt;` |
| entity-in-link-url | `[t](https://e.com/?a=1&amp;b=2)` | `<a href=\"https://e.com/?a=1&b=2\">t</a>` |
| url-underscore | `https://e.com/a_b_c` | (same) |
| url-star | `https://e.com/*a*` | `https://e.com/<i>a</i>` |

#### `05_escapes_order_dates`

| case | input | create name |
|---|---|---|
| esc-tilde | `\\~\\~x\\~\\~` | (same) |
| esc-underscore | `\\_x\\_` | (same) |
| esc-parens | `\\(x\\)` | `(x)` |
| esc-bang | `\\!x` | (same) |
| esc-close-bracket | `a\\]b` | `a]b` |
| esc-gt | `\\>x` | (same) |
| esc-minus | `\\-x` | (same) |
| esc-dot | `\\.x` | (same) |
| esc-amp | `\\&amp;` | `\\&` |
| esc-star-in-code | `ˋ\\*x\\*ˋ` | `<code>*x*</code>` |
| esc-link | `\\[t](https://e.com)` | `[t](https://e.com)` |
| esc-at-end | `x\\` | (same) |
| entity-star | `&#42;x&#42;` | `<i>x</i>` |
| entity-backtick | `&#96;x&#96;` | `<code>x</code>` |
| entity-bracket-date | `&#91;2026-01-15&#93;` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time>` |
| entity-in-bold | `**&amp;**` | `<b>&</b>` |
| date-midnight | `[2026-01-15 00:00]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\" startHour=\"0\" startMinute=\"0\">Thu, Jan 15, 2026 at 12:00 AM</time>` |
| date-noon | `[2026-01-15 12:00]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\" startHour=\"12\" startMinute=\"0\">Thu, Jan 15, 2026 at 12:00 PM</time>` |
| date-morning | `[2026-01-15 09:05]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\" startHour=\"9\" startMinute=\"5\">Thu, Jan 15, 2026 at 9:05 AM</time>` |
| date-2359 | `[2026-01-15 23:59]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\" startHour=\"23\" startMinute=\"59\">Thu, Jan 15, 2026 at 11:59 PM</time>` |
| date-hour24 | `[2026-01-15 24:00]` | (same) |
| date-min60 | `[2026-01-15 12:60]` | (same) |
| date-single-digit-hour | `[2026-01-15 9:05]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\" startHour=\"9\" startMinute=\"5\">Thu, Jan 15, 2026 at 9:05 AM</time>` |
| date-seconds | `[2026-01-15 12:00:30]` | (same) |
| date-leap-valid | `[2024-02-29]` | `<time startYear=\"2024\" startMonth=\"2\" startDay=\"29\">Thu, Feb 29, 2024</time>` |
| date-leap-invalid | `[2025-02-29]` | (same) |
| date-apr31 | `[2026-04-31]` | (same) |
| date-old | `[1999-12-31]` | `<time startYear=\"1999\" startMonth=\"12\" startDay=\"31\">Fri, Dec 31, 1999</time>` |
| date-far | `[9999-12-31]` | `<time startYear=\"9999\" startMonth=\"12\" startDay=\"31\">Fri, Dec 31, 9999</time>` |
| date-year0 | `[0001-01-01]` | `<time startYear=\"1\" startMonth=\"1\" startDay=\"1\">Mon, Jan 1, 1</time>` |
| date-short-month | `[2026-1-5]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"5\">Mon, Jan 5, 2026</time>` |
| date-T | `[2026-01-15T14:30]` | (same) |
| date-ampm | `[2026-01-15 2:30 PM]` | (same) |
| date-in-bold | `**[2026-01-15]**` | `<b><time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time></b>` |
| date-as-link-text | `[[2026-01-15]](https://e.com)` | `<a href=\"https://e.com\"><time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time></a>` |
| date-followed-paren | `[2026-01-15](https://e.com)` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time>(https://e.com)` |
| date-current-year | `[2026-10-12]` | `<time startYear=\"2026\" startMonth=\"10\" startDay=\"12\">Mon, Oct 12, 2026</time>` |
| date-each-month | `[2026-01-01] [2026-02-01] [2026-03-01] [2026-04-01] [2026-05-01] [2026-06-01] [2026-07-01] [2026-08-01] [2026-09-01] [2026-10-01] [2026-11-01] [2026-12-01]` | `<time startYear=\"2026\" startMonth=\"1\" startDay=\"1\">Thu, Jan 1, 2026</time> <time startYear=\"2026\" startMonth=\"2\" startDay=\"1\">Sun, Feb 1, 2026</time…` |

#### `09_control_chars`

| case | input | create name | create note | update name | update note |
|---|---|---|---|---|---|
| U+0000 | `a\u0000b` | **HTTP 500** | **HTTP 500** | `ab` | `ab` |
| U+0001 | `a\u0001b` | `ab` | (same) | `ab` | `ab` |
| U+0008 | `a\bb` | `ab` | (same) | `ab` | `ab` |
| U+0009 | `a\tb` | (same) | (same) | (same) | (same) |
| U+000B | `a\u000bb` | `a` | (same) | `ab` | `ab` |
| U+000C | `a\fb` | `a` | (same) | `ab` | `ab` |
| U+000E | `a\u000eb` | `ab` | (same) | `ab` | `ab` |
| U+001B | `a\u001bb` | `ab` | (same) | `ab` | `ab` |
| U+001F | `a\u001fb` | `ab` | (same) | `ab` | `ab` |
| U+007F | `a\u007fb` | (same) | (same) | (same) | (same) |
| U+0080 | `a\u0080b` | (same) | (same) | (same) | (same) |
| U+0085 | `a\u0085b` | `a` | (same) | (same) | (same) |
| U+009F | `a\u009fb` | (same) | (same) | (same) | (same) |
| U+00A0 | `a\u00a0b` | (same) | (same) | (same) | (same) |
| U+00AD | `a\u00adb` | (same) | (same) | (same) | (same) |
| U+2028 | `a\u2028b` | `a` | (same) | (same) | (same) |
| U+2029 | `a\u2029b` | `a` | (same) | (same) | (same) |
| U+200E | `a\u200eb` | (same) | (same) | (same) | (same) |
| U+202E | `a\u202eb` | (same) | (same) | (same) | (same) |
| U+2066 | `a\u2066b` | (same) | (same) | (same) | (same) |
| U+FEFF | `a\ufeffb` | (same) | (same) | (same) | (same) |
| U+FFFD | `a�b` | (same) | (same) | (same) | (same) |
| U+FFFE | `a\ufffeb` | (same) | (same) | (same) | (same) |
| U+FFFF | `a\uffffb` | (same) | (same) | (same) | (same) |
| U+E000 | `a\ue000b` | (same) | (same) | (same) | (same) |
| U+1F600 | `a😀b` | (same) | (same) | (same) | (same) |
| U+10FFFF | `a\U0010ffffb` | (same) | (same) | (same) | (same) |
| all-c0-except-tab-nl-cr | `\u0000\u0001\u0002\u0003\u0004\u0005\u0006\u0007\b\u000b\f\u000e\u000f\u0010\u0011\u0012\u0013\u0014\u0015\u0016\u0017\u0018\u0019\u001a\u001b\u001c\u001d\u001e…` | **HTTP 500** | **HTTP 500** | `end` | `end` |

#### `13_trim_and_md_in_html`

| case | input | create name |
|---|---|---|
| lead-nbsp | `\u00a0x\u00a0` | `x` |
| lead-ideographic | `\u3000x\u3000` | `x` |
| lead-zwsp | `\u200bx\u200b` | (same) |
| lead-em-space | `\u2003x\u2003` | `x` |
| lead-tab | `\tx\t` | `x` |
| lead-feff | `\ufeffx\ufeff` | (same) |
| md-in-href | `<a href=\"https://e.com/*a*/[2026-01-15]\">l</a>` | `<a href=\"https://e.com/<i>a</i>/<time startYear=\"2026\" startMonth=\"1\" startDay=\"15\">Thu, Jan 15, 2026</time>\">l</a>` |
| md-in-code-tag | `<code>*x* **y**</code>` | `<code><i>x</i> <b>y</b></code>` |
| md-in-attr-b | `<b title=\"**t**\">x</b>` | `<b title=\"<b>t</b>\">x</b>` |
| md-link-in-html-text | `<b>[t](https://e.com)</b>` | `<b><a href=\"https://e.com\">t</a></b>` |
| underscore-tag | `#a_b_c_` | (same) |
| star-tag | `#a*b*c` | `#a<i>b</i>c` |
| tilde-url | `https://e.com/~~x~~` | `https://e.com/<s>x</s>` |
| lt-entity-then-md | `&lt;**x**&gt;` | `<<b>x</b>>` |
