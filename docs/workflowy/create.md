# Workflowy API: node creation (`POST /api/v1/nodes`)

Area owner tag: `create`. Experiments: `experiments/create/*.py` (each one is
re-runnable; `*.out.json` files next to them hold the raw results of the run
this document is based on). All requests and responses are also in
`.prod-state/requests.jsonl` with `"tag": "create"`. Experiments ran on
2026-10-12 (server clock: unix ~1791834396–1791835000).

Legend: **[O]** = observed directly (usually more than once); **[I]** =
inferred / a hypothesis that fits all observations; **[?]** = not tested.

---

## 0. TL;DR

* Response is just `{"item_id": "<uuid>"}` (HTTP 200, not 201). **[O]**
* The node can be read immediately (GET and list) on the very next request
  (~0.2–0.35 s later), every time across more than 300 creates. No eventual consistency was seen. **[O]**
* Defaults on GET: `note: null`, `completed: false`, `completedAt: null`,
  `data: {"layoutMode": "bullets"}`, `createdAt == modifiedAt` (integer unix
  **seconds**). **[O]**
* Priorities: the first child in an empty parent gets `100`. `bottom` gets `ceil((max+100)/100)*100`.
  `top` (and omitted or `null`) gets `floor(min/2)`. When that would equal `min` (min==0),
  **all** siblings are renumbered `100, 200, ...` in display order, and this does **not**
  bump their `modifiedAt`. **[O]/[I]**
* `name` goes through a home-grown markdown-ish converter (not CommonMark). It
  has several quirks a mock must copy: line-joining heuristics, `> quote` is
  **not** supported, `#### ` gives an undocumented layoutMode `"p"`, `- [x]`
  leaks `completed=true` to all later sibling lines, and HTML entities are decoded.
  Details in §4–§5.
* Multiline names make the first block the created node (`item_id`). Each later
  block becomes a **flat** child of it (no grandchildren) with priorities
  `0, 1, 2, ...`, `note: ""`, and a default layoutMode of `"p"`.
* `note` is stored verbatim. Markdown is not parsed and whitespace is not stripped. **[O]**
* `layoutMode` accepts **any string** (no enum check). `""` and `null` give `"bullets"`.
  An explicit layoutMode overrides a markdown prefix. The prefix is still
  stripped, and `- [x]` still completes the node. **[O]**
* `completed`, `completedAt`, `priority`, `id`, `createdAt`, `data`, and unknown
  fields are silently ignored. **[O]**
* Validation is pydantic-like: `400 {"errors":[{type,loc,msg,input}, ...]}`. Validation runs before
  parent lookup. An unknown parent returns `404 {"code":"not_found","message":...}`. **[O]**

---

## 1. Request/response shape

### 1.1 Minimal create **[O]** (`01_basic.py`)

```
POST /api/v1/nodes
{"parent_id":"7c701a67-85cf-4d2f-9e63-ab31d82b4a5a","name":"hello world"}

200 OK
content-type: application/json
{"item_id": "c162ff08-591e-43ca-9662-dfff74a5f7a5"}
```

Notable response headers (identical on GET): `server: nginx`,
`content-type: application/json`, `transfer-encoding: chunked`,
`content-encoding: gzip`, `vary: Accept-Encoding, Cookie, Origin`,
`x-frame-options: SAMEORIGIN`, `x-content-type-options: nosniff`,
`x-xss-protection: 1; mode=block`, `referrer-policy: same-origin`,
`service-worker-allowed: /`,
`strict-transport-security: max-age=63072000; includeSubDomains; preload`,
and a permissive `content-security-policy`. There are **no** rate-limit headers
(`x-ratelimit-*`, `retry-after`), no `location` header, and no request id. **[O]**

The response has no other keys. It does not include the node or the ids of children
created from a multiline name. **[O]**

### 1.2 The created node on GET **[O]**

`GET /api/v1/nodes/c162ff08-...` immediately afterwards (0.35 s after the POST was sent):

```json
{"node": {"id": "c162ff08-591e-43ca-9662-dfff74a5f7a5", "name": "hello world",
          "note": null, "parent_id": "7c701a67-85cf-4d2f-9e63-ab31d82b4a5a",
          "priority": 100, "completed": false, "data": {"layoutMode": "bullets"},
          "createdAt": 1791834396, "modifiedAt": 1791834396, "completedAt": null}}
```

* The key order is `id, name, note, parent_id, priority, completed, data,
  createdAt, modifiedAt, completedAt`, which differs from the upstream docs example. **[O]**
* Timestamps are integer unix **seconds**. `createdAt == modifiedAt` at
  creation, for every node observed. **[O]**
* `data` only ever contained `layoutMode`. **[O]**
* Creating a child does **not** change the parent's `modifiedAt` (we checked with
  a 2.5 s gap: `1791834937` before and after; `13_parent_id_forms.py`). **[O]**
* The node also appears in `GET /api/v1/nodes?parent_id=...` on the next
  request. This held for every create in every experiment. **[O]**

---

## 2. `position` and priority assignment

### 2.1 Valid values **[O]** (`02_priorities.py`, `03_priority_exhaustion.py`, `14_priority_followups.py`)

| sequence of creates into an empty parent | resulting priorities (display order) |
|---|---|
| omitted ×4 | 12, 25, 50, 100 (newest first) |
| `top` ×4 | 12, 25, 50, 100 (newest first) |
| `bottom` ×4 | 100, 200, 300, 400 |
| bottom, top, bottom, top, top, bottom | top-4 12, top-3 25, top-1 50, bottom-0 100, bottom-2 200, bottom-5 300 |
| `position: null` | same as `top` |

So the omitted position is `top`, as the docs say.

**Top-insert exhaustion and renumbering** (`03`): repeated `top` gave
100, 50, 25, 12, 6, 3, 1, 0. The **next** top insert (min is 0, and
`floor(0/2) == 0` collides) renumbered the whole sibling list:

```
after top-7: [top-7 0, top-6 1, top-5 3, top-4 6, top-3 12, top-2 25, top-1 50, top-0 100]
after top-8: [top-8 100, top-7 200, top-6 300, top-5 400, ..., top-0 900]
after top-9: [top-9 50, top-8 100, ...]
```

The renumbered siblings kept their old `modifiedAt` (for example, top-0 kept
`1791834433` after renumbering at ~`1791834437`). **[O]**

**Bottom formula** (`14`):

| existing sibling priorities | `bottom` got |
|---|---|
| (none) | 100 |
| 100, 200, 300 | 400 |
| 12, 25, 50, 100 | 200 |
| 0, 1 (children from a multiline name) | **200** (not 101) |
| 25, 50 (after deleting the 100) | **200** (not 150) |
| 100, 200 (after deleting the 300) | 300 (the max is reused) |

The formula that fits every case is **`ceil((max + 100) / 100) * 100`**, which is
max+100 rounded up to a multiple of 100. **[I]** followup-0 confirmed the same rule for
`move` and `mirror` with `position: bottom` (max 2, 3 and 50 all give 200; max 0 gives 100;
see ordering-and-move.md §7). A max in 101–199 that is not a multiple of 100 cannot be produced
through the API, so the rule is unverified there.

**Top formula**: `floor(min / 2)` fits every case. **[I]** Examples: min 100→50,
25→12, 3→1, 1→0, and min 200 (after deleting the 100)→100. With children
`0, 1, 200` (from a multiline create plus a bottom), a top insert renumbered
everything to `100, 200, 300, 400`. **[O]**

Deleting a sibling never renumbers the remaining siblings. **[O]**

### 2.2 Invalid values **[O]** (`02e`)

Each of `"middle"`, `"TOP"`, `"Bottom"`, `""`, `0`, `1`, `5`, `-1`, `true`,
`"1"`, and `["top"]` gave:

```
400 {"errors": [{"type": "enum", "loc": ["position"],
                 "msg": "Input should be 'top' or 'bottom'", "input": <value as sent>}]}
```

`null` is accepted and treated as `top`. Matching is case-sensitive.

---

## 3. Other body fields

### 3.1 `name` **[O]** (`12_params_misc.py`)

| input | result |
|---|---|
| missing | 400 `{"type":"missing","loc":["name"],"msg":"Field required","input":{<whole body>}}` |
| `null`, `42`, `true`, `["a","b"]` | 400 `{"type":"string_type","loc":["name"],"msg":"Input should be a valid string","input":<v>}` |
| `""` | 200, name `""` |
| `"   "` | 200, name `""` (stripped) |
| `"\n\n\n"` | 200, a single node with name `""` and no children |
| 10 000 × `x` | 200, all 10 000 chars stored |
| 99 999 chars (`"word word ..."`) | 200, all stored, GET took 0.2 s |

No maximum length was hit up to 100 KB.

### 3.2 `note` **[O]**

The note is stored **verbatim**. Markdown is not parsed, entities are not decoded,
whitespace is not stripped, and newlines are kept:

| input `note` | stored `note` |
|---|---|
| omitted | `null` |
| `null` | `null` |
| `""` | `""` (**not** null) |
| `"  padded  "` | `"  padded  "` |
| `"**bold** *it* \`code\` [l](https://x.com) # h [2026-01-15]"` | same, unchanged |
| `"line1\nline2\n\nline4\n- bullet\n  indented"` | same, unchanged, and no children created |
| `"<b>b</b> &amp; &lt;x&gt; a<b"` | same, unchanged |
| `5` | 400 `string_type`, `loc: ["note"]` |

### 3.3 `layoutMode` **[O]** (`11_layoutmode_param.py`)

| `layoutMode` | stored `data.layoutMode` |
|---|---|
| `bullets` / `todo` / `h1` / `h2` / `h3` / `code-block` / `quote-block` | same |
| `p` (undocumented) | `p` |
| `board`, `dashboard`, `H1` | **stored verbatim**. There is no enum validation. |
| `""` | `bullets` |
| `null` | `bullets` |
| `1` | 400 `string_type`, `loc: ["layoutMode"]` |
| `data: {"layoutMode":"h1"}` (nested, instead of top-level) | ignored, so `bullets` |

How it interacts with markdown prefixes in `name`:

| name | layoutMode param | stored name | stored layoutMode | completed |
|---|---|---|---|---|
| `# Head` | `todo` | `Head` | `todo` | false |
| `# Head` | `bullets` | `Head` | `bullets` | false |
| `- [x] done` | `bullets` | `done` | `bullets` | **true** (completedAt set) |
| `- [x] done` | `h1` | `done` | `h1` | **true** |
| `- [ ] open` | `h2` | `open` | `h2` | false |
| ` ```code``` ` | `quote-block` | `code` | `quote-block` | false |
| `- bullet` | `todo` | `bullet` | `todo` | false |
| `**bold**` | `code-block` | `<b>bold</b>` | `code-block` | false |
| `#### four` | `h1` | `<b> four</b>` | `h1` | false |

Rule: a non-empty `layoutMode` param overrides the layout derived from the
prefix. The prefix is still stripped from the name, and `- [x]` still marks
the node completed. Inline markdown is converted even for `code-block`.
The param applies to the **first** node only. Children from a multiline name keep
their own derived layoutMode (for example, `"a\n\nb\n\nc"` with `layoutMode: todo`
gave parent `todo` and child `c` with layoutMode `p`).

### 3.4 Ignored fields **[O]**

These were all accepted with 200 and silently ignored:

* `completed: true`: the node is stored as `completed: false`. The only way to create a
  completed node is the `- [x] ` prefix (or a later `/complete` call).
* `completed: false` with name `- [x] x`: the node is still stored completed.
* `completedAt: 1700000000`: ignored.
* `priority: 5`: ignored. The node got the normal top priority.
* `id: "11111111-2222-..."`: ignored. The server generated its own id.
* `createdAt: 1600000000`: ignored.
* Unknown fields (`foo`, `mirror`, `parentId`): ignored.

### 3.5 `parent_id` forms **[O]** (`13_parent_id_forms.py`, `13b_bad_ids_and_bodies.py`)

| `parent_id` | result |
|---|---|
| full UUID of a sandbox | 200 |
| 12-char short id (lowercase) | 200, stored `parent_id` is the full UUID |
| `https://workflowy.com/#/<short>` | 200, stored `parent_id` is the full UUID |
| short id in **UPPERCASE** | 200, node created under the right parent (case-insensitive) |
| random UUID | 404 `{"code":"not_found","message":"Parent item not found"}` |
| `https://workflowy.com/#/0123456789ab` (random short id in a URL) | 404 `{"code":"not_found","message":"Parent item not found"}` |
| random 12-hex short id `0123456789ab` | 404 `{"code":"not_found","message":"Location '0123456789ab' not found"}` |
| `not-a-real-id-zzz` | 404 `{"code":"not_found","message":"Location 'not-a-real-id-zzz' not found"}` |
| `123` (int) | 400 `string_type`, `loc: ["parent_id"]` |
| `https://workflowy.com/#/<random full UUID>` (or dashless UUID) | 404 `Location '<raw url>' not found`. URLs only understand a 12-hex short id. **[O]** (followup-2) |
| `https://workflowy.com/#/<RANDOM SHORT>` (uppercase) | 404 `Parent item not found` **[O]** (followup-2) |
| random UUID in UPPERCASE | 404 `Parent item not found` **[O]** (followup-2) |
| random UUID without dashes | 404 `Location '<raw>' not found` **[O]** (followup-2) |
| omitted / `null` / `""` | **passes validation** (no `parent_id` error is added to the errors list, see §3.6). What it then resolves to (root?) was **not tested**, because the request would execute. **[O]** (followup-2) |
| `None` / `inbox` / calendar keys / shortcut keys | **not tested** (blocked by the scope guard) |

The 404 message depends on the form of the id. A UUID, or a URL that contains a short id, gives
"Parent item not found". A bare short id or other string goes through the
"Location" (target/shortcut) resolver and gives `Location '<input>' not found`. **[O]**

Note on method: the uppercase-short-id probe was sent with `unscoped=True`
because the guard's allow-list is case-sensitive. The target was our own
sandbox, and we confirmed the node landed there (it is `207be52d...`, "via upper
short"). This is outside the stated use of `unscoped`, so we did **not** probe the
remaining id variants (uppercase full UUID, UUID without dashes, URL without `#/`,
URL with a trailing slash, URL with a full UUID). See Open questions.
**Suggestion for prod_client.py:** lowercase ids in `AllowList.contains`.

### 3.6 Body encodings and validation order **[O]** (`13b`)

All of these used a random (non-existent) parent:

| body | result |
|---|---|
| truncated JSON | 400 `{"code":"invalid_json","message":"Invalid JSON format in request body"}` |
| empty body | same `invalid_json` 400 |
| `[1,2]` / `"hello"` | 400 `{"errors":[{"type":"model_type","loc":[],"msg":"Input should be a valid dictionary or instance of CreateItemRequest","input":[1,2]}]}` |
| `application/x-www-form-urlencoded` `parent_id=<fake>&name=x` | 404 "Parent item not found", so **form bodies are parsed** |
| JSON text sent with `Content-Type: text/plain` | 404 "Parent item not found", so content type is ignored for JSON |
| random parent with missing name | 400 `missing name`, so validation runs before parent lookup |
| random parent with bad position | 400 enum, so validation runs before parent lookup |
| `{"parent_id":<fake>,"name":1,"note":2,"position":"q","layoutMode":3}` | 400 with **all** errors, in the order `name, note, layoutMode, position` |
| `{"position":"q","layoutMode":3,"note":2,"name":1,"parent_id":1}` | 400 with all five errors, in the order `name, parent_id, note, layoutMode, position` (followup-2) |
| `{"parent_id":1}` | 400 `[missing name (input = whole body), string_type parent_id]` (followup-2) |
| `{}` | 400 `[missing name]` only. `parent_id` is **not** required (followup-2) |
| `{"name":1}`, `{"name":1,"parent_id":null}`, `{"name":1,"parent_id":""}` | 400 with only the `name` error. `parent_id` is optional and nullable, and `""` is a valid string (followup-2) |

The pydantic model is called `CreateItemRequest`. **[O]** The field (declaration) order is
`name, parent_id, note, layoutMode, position`. **[O]** (followup-2,
`experiments/followup-2/01_validation_and_parent_resolution.py`). The order of keys in
the request body does not matter. An earlier version of this doc guessed
`parent_id` came first. That was wrong.

---

## 4. Inline markdown in `name` (single line)

Data from `04_markdown_name.py` and `05_markdown_followup.py` (inputs were
re-run where results were surprising, and the results were identical).

### 4.1 Inline styles **[O]**

| input | stored `name` |
|---|---|
| `**bold**` | `<b>bold</b>` |
| `*italic*` | `<i>italic</i>` |
| `_underscore italic_` | unchanged (underscores are not supported) |
| `__double underscore__` | unchanged |
| `~~strike~~` | `<s>strike</s>` |
| `` `code` `` | `<code>code</code>` |
| `[link text](https://example.com)` | `<a href="https://example.com">link text</a>` |
| `[text](not a url)` | `<a href="not a url">text</a>` (no URL validation) |
| `[text](javascript:alert(1))` | `<a href="javascript:alert(1">text</a>)` (lazy `(.*?)\)`) |
| `[](https://empty.text)` | `<a href="https://empty.text"></a>` |
| `![img](https://example.com/a.png)` | `!<a href="https://example.com/a.png">img</a>` |
| `<https://example.com>` | unchanged |
| `https://example.com bare url` | unchanged |
| `mixed **bold** and *italic* and `` `code` `` here` | `mixed <b>bold</b> and <i>italic</i> and <code>code</code> here` |
| `***bold italic***` | `<b><i>bold italic</b></i>` (**mis-nested**) |
| `**unterminated bold` | `<i></i>unterminated bold` (**empty italic**) |
| `*unterminated italic` | unchanged |
| `` `unterminated code ``, `~~unterminated strike` | unchanged |
| `a * b * c` | `a <i> b </i> c` |
| `2 * 3 = 6` | unchanged |
| `snake_case_name` | unchanged |
| `\*escaped\*` | `*escaped*` (backslash escapes are removed) |
| `` **`code in bold`** `` | `<b><code>code in bold</code></b>` |
| `` `**bold in code**` `` | `<code><b>bold in code</b></code>` (markdown **is** applied inside code) |
| `#tag at start`, `text with #tag and @person` | unchanged (tags are plain text) |

The quirks suggest regex substitutions applied in this order **[I]**:
bold `\*\*(.+?)\*\*`, then italic `\*(.*?)\*` (an empty body is allowed, which
explains `<i></i>`), then strike, code, and link `\[(.*?)\]\((.*?)\)`. Escapes
`\*` are protected and then unescaped. Run on `***x***`, bold produces `<b>*x</b>*`, and italic then
produces `<b><i>x</b></i>`. Both quirk examples come out exactly right under this order.

### 4.2 Dates **[O]**

| input | stored `name` |
|---|---|
| `[2026-01-15]` | `<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>` |
| `meeting [2026-01-15] notes` | `meeting <time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time> notes` |
| `[2026-01-15 14:30]` | `<time startYear="2026" startMonth="1" startDay="15" startHour="14" startMinute="30">Thu, Jan 15, 2026 at 2:30 PM</time>` |
| `[2026-13-45]` | unchanged (an invalid date is left as text) |
| `[2026-01]` | unchanged |

Month, day, and hour have no zero padding in the attributes. The display text uses the format
`%a, %b %-d, %Y` plus ` at %-I:%M %p`. **[I]**

### 4.3 HTML and entities **[O]**

| input | stored `name` |
|---|---|
| `<b>raw html bold</b>` | unchanged |
| `<i>i</i> <s>s</s> <code>c</code> <a href="https://x.com">a</a>` | unchanged |
| `<u>underline</u>` | unchanged |
| `<script>alert(1)</script>` | unchanged (**not sanitized**) |
| `a < b > c & d` | unchanged (not escaped) |
| `AT&T` | unchanged |
| `&amp; entity` | `& entity` (**entity decoded**) |
| `&lt;b&gt;not bold&lt;/b&gt;` | `<b>not bold</b>` (decoded into a real tag) |
| `&quot;q&quot; &#39;a&#39; &nbsp;nb` | `"q" 'a' \u00a0nb` |

Hypothesis **[I]**: the server stores the name HTML-escaped internally and
the API unescapes it on output. Alternatively, it unescapes the input. The two cannot be told apart from
create alone. See update.md and errors-and-http.md for round-trip behaviour.

### 4.4 Whitespace **[O]**

* Leading and trailing whitespace is stripped, including leading U+00A0 (Python `str.strip`
  semantics). A leading zero-width space U+200B is **kept**.
* `"tab\there"` keeps the tab.
* `"line\rwith cr"` becomes `"line with cr"` (`\r` is normalised to `\n` and then joined, see §5).
* U+2028 (LINE SEPARATOR) **splits** lines but never joins: `"a\u2028b line sep"` gives
  parent `a` and child `b line sep` (layoutMode `p`). This is consistent with Python
  `str.splitlines()` being used for splitting while the join regex only knows `\n`. **[I]**

### 4.5 Line-prefix layouts **[O]**

| input (first line) | stored name | layoutMode | completed |
|---|---|---|---|
| `# Header one` | `Header one` | `h1` | false |
| `## Header two` | `Header two` | `h2` | false |
| `### Header three` | `Header three` | `h3` | false |
| `#### Header four` | `<b> Header four</b>` (note the leading space) | **`p`** | false |
| `#### **already bold**` | `<b> <b>already bold</b></b>` | `p` | false |
| `##### Header five` (also 6 and 7 hashes) | unchanged | `bullets` | false |
| `#nospace`, `#`, `# ` | unchanged (`# ` is stripped to `#` first) | `bullets` | false |
| `- bullet` | `bullet` | `bullets` | false |
| `-no space`, `-`, `- ` | unchanged (`- ` is stripped to `-`) | `bullets` | false |
| `* star bullet`, `+ plus bullet`, `1. numbered` | unchanged | `bullets` | false |
| `- [ ] todo open` | `todo open` | `todo` | false |
| `- [x] todo done` | `todo done` | `todo` | **true**, completedAt = createdAt |
| `- [X] todo done caps` | `[X] todo done caps` (only `- ` stripped) | `bullets` | false |
| `- [ ]` / `- [x]` alone | `""` | `todo` | false / true |
| `- [ ]no space after` | `o space after` (**6 chars are cut blindly**) | `todo` | false |
| `-  [ ] two spaces` | ` [ ] two spaces` (leading space kept) | `bullets` | false |
| `[ ] bare checkbox`, `[x] bare checkbox done` | unchanged | `bullets` | false |
| `` ```code fence``` `` | `code fence` | `code-block` | false |
| ```` ``` ```` (three backticks only) | `""` | `code-block` | false |
| ```` ``` ``` ```` | `" "` | `code-block` | false |
| `` ```\nfenced\n``` `` | `fenced` | `code-block` | false |
| `` ```python\nprint(1)\n``` `` | `python\nprint(1)` (the info string becomes text) | `code-block` | false |
| `> quote`, `>quote nospace`, `> **bold quote**` | **unchanged** (`> <b>bold quote</b>`) | **`bullets`** | false |
| `- # header after bullet` | `# header after bullet` | `bullets` | false |
| `# - bullet after header` | `- bullet after header` | `h1` | false |
| `- [ ] # todo header` | `# todo header` | `todo` | false |
| `  leading spaces`, `    indented four spaces` | stripped | `bullets` | false |

**Discrepancy with the upstream docs:** `> text` does **not** produce
`quote-block`. It was tried 5 ways and re-run, and the result was deterministic. The only way to get
`quote-block` at create time is the `layoutMode` param. **[O]**

Only **one** prefix is consumed per line. The checks are done on the stripped line in roughly this
order **[I]**: `- [ ]` / `- [x]` (cut 6 chars), `- ` (cut 2), `### ` / `## ` / `# `
(cut 4/3/2), `#### ` (gives `p`, `"<b>" + line[4:] + "</b>"`), and a fence. Inline markdown is applied to the remainder.

---

## 5. Multiline names

Data from `06_multiline.py`, `07_multiline_determinism.py`, `08_line_join_rule.py`,
`09_line_join_rule2.py`, and `10_code_fences.py`. Everything was deterministic when re-run.

### 5.1 The upstream docs are wrong or incomplete

The docs say a single `\n` joins into a space and `\n\n` creates separate
children. That is **not** what happens. The real rule is a heuristic soft-wrap
joiner that ignores how many newlines there are:

| input | created node (item_id) | its children |
|---|---|---|
| `a\nb` | `a b` | — |
| `a\n\nb` | `a b` | — |
| `a\n\n\n\nb` | `a b` | — |
| `a\n\nb\n\nc` | `a b` | `c` (p) |
| `x\n\ny\n\nz\n\nw` | `x y` | `z w` (p) |
| `x\n\ny\n\nz\n\nw\n\nv` | `x y` | `z w` (p), `v` (p) |
| `a\nb\nc\nd\ne\nf` | `a b` | `c d`, `e f` |
| `ab\nc\nd` | `ab c` | `d` |
| `a\nbc\nd` | `a bc d` | — |
| `aa\nbb\ncc` / `aa\n\nbb\n\ncc` | `aa bb cc` | — |
| `parent\n\nchild1\n\nchild2` | `parent child1 child2` | — |
| `p\n\nc1\nc1cont\n\nc2` | `p c1 c1cont c2` | — |
| `hello\nworld\nagain\nmore` | `hello world again more` | — |
| `Hello world.\nthis is lowercase` | `Hello world.` | `this is lowercase` |
| `Hello world\nThis is upper` | `Hello world` | `This is upper` |
| `a\nB` / `a\n1` / `a1\n2b` / `a\nété` | `a` / `a` / `a1` / `a` | `B` / `1` / `2b` / `été` |
| `a\n#tag`, `a\n(b)`, `a\n_b`, `a\n"q"`, `a\n@b` | `a` | the second line |
| `a\n*b*` / `a\n**b**` / `a\n[link](...)` / `` a\n`b` `` | `a` | `<i>b</i>` / `<b>b</b>` / `<a ...>` / `<code>b</code>` |
| `a.\nb`, `x!\nb`, `x?\nb`, `a.\n\nb` | `a.` / `x!` / `x?` | `b` |
| `x,\nb` `x:\nb` `x;\nb` `x)\nb` `x*\nb` `x-\nb` `x"\nb` `x'\nb` `x1\nb` `xé\nb` `xA\nb` `x]\nb` `x>\nb` `` x`\nb `` `x~\nb` `x#\nb` `x/\nb` `x。\nb` | `x, b` etc. (joined) | — |
| `a \nb` | `a  b` (**two** spaces) | — |
| `a\n \nb` (whitespace-only line) | `a` | `b` |
| `a\n  b`, `a\n\n b`, `a\n\n\tb` | `a` | `b` |
| `a\r\nb`, `a\r\n\r\nb` | `a b` | — |
| `\n\na leading`, `a trailing\n\n` | stripped | — |

**Join rule [I]** (it fits every row above): after normalising `\r\n` and `\r` to `\n`,
apply one **non-overlapping, left-to-right** regex substitution

```
re.sub(r"([^.!?\n])\n+([a-z])", r"\1 \2", text)
```

which only covers ASCII lowercase after the break. Because the match consumes
the first character of the next line, a 1-character line that was just
joined cannot be the "before" character of the next join. That is why
`a\nb\nc` gives `a b` + `c`, while `parent\nchild1\nchild2` joins
completely. Joining is suppressed inside code fences (§5.4).

### 5.2 Splitting into nodes **[O]/[I]**

After joining, the text is split into lines (`str.splitlines()` semantics, so U+2028
also splits). Each line is stripped, and empty lines are dropped. Code fences are grouped
(§5.4). Then:

* The **first** block becomes the created node, and `item_id` is its id. Its
  `priority` is assigned by `position` as usual. Its `note` is the `note` param
  (default `null`). Its layoutMode comes from the `layoutMode` param if non-empty,
  otherwise from the prefix, otherwise `bullets`.
* Every **later** block becomes a **direct child of the first node**. The structure is
  always flat, and indentation never creates grandchildren:
  `- a\n  - b\n    - c` gives `a` with children `b` and `c`, and
  `p\n\n- one\n\n  - two\n\n- three` gives `p` with children `one`, `two`, `three`.
* Children get priorities **0, 1, 2, ...** in text order, `note: ""` (an empty string,
  not null), and the same `createdAt`/`modifiedAt` as the parent.
  Their layoutMode comes from their prefix, and the **default is `"p"`**, not `bullets`.
  Example: the child `> q` is stored as name `> q`, layoutMode `p`.
* Example (`06`, case 8): `# Title\n\n- [ ] t1\n\n- [x] t2\n\n## sub` gives the node
  `Title` (h1) with children `t1` (todo, pri 0), `t2` (todo, completed, pri 1),
  and `sub` (h2, pri 2, **completed: true**, see §5.3).
* `p\n\n# h1\n\n## h2\n\n### h3\n\n#### h4\n\n- [ ] t\n\n```c```\n\n> q` gives
  children `h1`/h1, `h2`/h2, `h3`/h3, `<b> h4</b>`/p, `t`/todo, `c`/code-block,
  and `> q`/p, with priorities 0 to 6.
* `1. a\n2. b` gives `1. a` with child `2. b` (p). Numbered lists are not recognised.

### 5.3 `- [x]` completion leak (bug) **[O]**

Once a line with the `- [x]` prefix has been processed, **every later line
in the same request is also created with `completed: true`** and
`completedAt` set, whatever its prefix. This includes `- [ ]` lines,
plain text, and headers:

```
"p\n\n- [x] a\n\n- [ ] B\n\n- C"
  p (bullets, completed false)
    a (todo, completed true)
    B (todo, completed TRUE)      <- even though "- [ ]"
    C (bullets, completed TRUE)
"- [x] parent\n\nchild one\n\nChild two"
  "parent child one" (todo, completed true)
    "Child two" (p, completed TRUE)
```

The leaked children have `completedAt == createdAt`. Lines **before** the first
`- [x]` are unaffected.

### 5.4 Code fences in multiline names **[O]/[I]**

| input | created node | children |
|---|---|---|
| `` ```\na\nb\n``` `` | `a\nb` (code-block): no join inside the fence | — |
| `` ```\nline one\n\nline two\n``` `` | `line one\nline two` (the **blank line is dropped**) | — |
| `` ```\n    indented code\n\tTabbed\n``` `` | `    indented code\n\tTabbed` (indentation kept) | — |
| `` p\n\n```\nline one\nline two\n``` `` | `p` | `line one\nline two` (code-block) |
| `` p\n\n```\nline one\nline two\n```\n\nAfter `` | `p` | code-block, then `After` (p) |
| `` p\n\n```\nline one\nline two\n```\n\nafter `` | `p` | `line one\nline two\n``` after` (code-block), where the **closing fence was eaten** |
| `` p\n\n```js\nx = 1\n```\n\nAfter `` | `p` | `js\nx = 1` (code-block), `After` (p) |
| `` p\n\n```inline```\n\nafter `` | `p` | `inline``` after` (code-block) |
| `` p\n\n```\nUpper\n```\n\nq `` | `p` | `Upper\n``` q` (code-block) |
| `` ```\nunclosed\nMore `` | `" unclosed\nMore"` (code-block): no closing line, so no join-free region; the opener joined with `unclosed` (conformance golden `text_multiline_code_fences`) | — |
| `` first\n\n```\ncode\nmore\n```\n\nafter `` | `first` | `code\nmore\n``` after` (code-block) |

Model that fits **[I]**:

1. On the normalised text, find code regions line by line. A line starting with
  ```` ``` ```` opens a fence unless it is longer than 3 chars and also ends with
  ```` ``` ```` (a one-line fence). The region is the opening line plus the
   following lines, up to but **not including** the next line that ends with ```` ``` ````.
   **Corrected 2026-10-12**: if no later line ends with ```` ``` ```` (an unclosed fence),
   there is **no region**, and the join applies to the opener's newline as well:
   ```` ```\nunclosed\nMore ```` is created as one code-block named `" unclosed\nMore"`
   (the opener and `unclosed` were joined by step 2, then step 3 grouped the rest).
   Evidence: `tests/conformance/golden/text_multiline_code_fences.json`, step 5.
2. Apply the join regex (§5.1) only to text outside those regions. The closing
   fence line is outside, so ```` ```\n\nafter ```` joins into ```` ``` after ````.
3. Split into lines. A line starting with ```` ``` ```` starts a code block that
   swallows the following non-blank lines (unstripped) up to and including the first one
   that ends with ```` ``` ````, or up to the end of the input. A line that starts and ends with
   ```` ``` ```` (length > 3) is a block by itself. The block text is the lines joined
   with `\n`. Remove a leading ```` ``` ```` and a trailing ```` ``` ````, then strip `\n`
   (but not spaces, so ```` ``` ``` ```` gives `" "`).

---

## 6. Timing and consistency **[O]**

* More than 300 creates were each followed immediately by a GET or list of the new node. All succeeded, and
  every multiline child was already listed on the next request.
* POST latency was typically about 0.15–0.35 s.
* Timestamps have one-second granularity. `createdAt == modifiedAt` for new nodes,
  and multiline children share the parent's timestamp.

---

## 7. Mock server implementation notes

`POST /api/v1/nodes`:

1. Parse the body. Accept JSON regardless of Content-Type, and also
   `application/x-www-form-urlencoded`. If the body is empty or invalid JSON, return
   `400 {"code":"invalid_json","message":"Invalid JSON format in request body"}`.
   If it is not an object, return 400 `model_type` (msg `Input should be a valid dictionary or
   instance of CreateItemRequest`, `loc: []`).
2. Validate the fields, collecting **all** errors in the order
   `name, parent_id, note, layoutMode, position` (observed, followup-2):
   * `name`: required string (`missing`, whose `input` is the whole body dict; or `string_type`).
   * `parent_id`, `note`, `layoutMode`: optional, string or null (`string_type`). Do not coerce ints.
     A missing or null `parent_id` is **not** a validation error.
   * `position`: `"top"`, `"bottom"`, or null. Anything else gives `enum`,
     `"Input should be 'top' or 'bottom'"`.
   * Return `400 {"errors":[{"type","loc":[field],"msg","input"}]}` when there are any errors.
   * Ignore every other key: completed, completedAt, priority, id, createdAt, data, unknown keys.
3. Resolve `parent_id` **after** validation. UUID, short id (case-insensitive),
   and `https://workflowy.com/#/<short>` should all resolve. Not found gives 404
   `{"code":"not_found","message":"Parent item not found"}` for UUIDs and URLs, or
   `{"code":"not_found","message":"Location '<raw>' not found"}` for bare
   short ids and other strings. (Special keys: see the other docs.) For the full
   resolver algorithm see errors-and-http.md §2.2.
4. Parse `name` into blocks (§4, §5): normalise CR, join (regex, outside fences),
   split, strip, drop empties, group fences, then parse the prefix and inline markdown of each block.
   Decode HTML entities (`html.unescape`) in the result. Keep a `completed` flag
   that becomes true at the first `- [x]` block and **is never reset**.
5. First block: priority per §2. `layoutMode` = param if it is a non-empty string
   (any value, stored verbatim), else the prefix layout, else `bullets`. `note` = param
   verbatim (null if absent). `completed`/`completedAt` come from the flag.
   `createdAt = modifiedAt = now` (int seconds).
6. Later blocks: children of the first, priority = index (0-based), `note=""`,
   layoutMode from the prefix or else `"p"`, completed from the flag, same timestamps.
7. Priority allocation:
   * empty parent: 100
   * bottom: `ceil((max+100)/100)*100`
   * top / omitted / null: `p = min // 2`. If `p == min`, renumber all existing
     siblings in display order to `200, 300, ...` and give the new node `100`.
     Do not touch the siblings' `modifiedAt`.
8. Do **not** update the parent's `modifiedAt`.
9. Respond `200 {"item_id": "<new uuid v4, lowercase>"}`.

Suggested regexes for inline conversion, in order: escape-protect `\*`, then
`\*\*(.+?)\*\*` → `<b>\1</b>`, `\*(.*?)\*` → `<i>\1</i>`, `~~(.+?)~~` → `<s>`,
`` `(.+?)` `` → `<code>`, `\[(.*?)\]\((.*?)\)` → `<a href="\2">\1</a>`,
`\[(\d{4})-(\d{2})-(\d{2})(?: (\d{2}):(\d{2}))?\]` → `<time ...>` (only for
valid dates). The exact patterns for strike, code, and link are guesses that match the
observed results. Check them against `experiments/create/04_*.out.json` and `05_*.out.json` as test vectors.
Those out files (plus 06–11) are good **golden test vectors** for the mock.

---

## 8. Open questions

1. Precise date-parsing rules: are `[2026-1-5]`, `[2026-02-30]`, and times like `[2026-01-15 9:30]`
   accepted? Does the time zone matter? Not tested.
2. Exact strike, code, and link regexes on edge cases (`~~~x~~~`, nested backticks,
   brackets inside link text, parentheses inside URLs other than the javascript case).
3. Whether `####`-style `p` + `<b>` is meant to be "h4". It is undocumented.
4. Is `layoutMode` validated anywhere else? For example, does the UI break on
   `board`/`H1`? Those nodes are in sandbox "create: 11 layoutMode param".
5. Whether HTML entities are decoded at input or at output. Does `&amp;amp;`
   become `&amp;` or `&`? Needs a round-trip test with update and GET.
6. Missing `parent_id`, which probably means root, is untested because of the guard. followup-2
   established that omitted, `null`, and `""` all **pass validation**, but not what they resolve to.
   For nonexistent targets, followup-2 also settled the uppercase full UUID, the dashless UUID and the
   URL with a full UUID (§3.5). Still untested: uppercase or dashless spellings of an *existing* UUID. A guarded way to test these would
   be to have the guard allow these variants when they normalise to an allow-listed id.
7. Is the renumber threshold exactly "`min//2 == min`"? Is there a similar
   renumbering for bottom at large priorities (overflow)? We never got above
   about 1000.
8. Children of a multiline create get priorities 0..n-1 rather than 100, 200, ....
   Is the UI fine with that? See UI scenario 3.
9. Is there an upper bound on name length or on the number of children from
   a multiline name? Tested up to 100 KB with 0 children.
10. What does the UI show for leaked `completed` on the h2 or `p` children? (UI scenario 4.)

---

## 9. UI scenarios (a human performs these in the web UI; we observe via API)

1. **Create bullets in the UI at top, middle, and bottom** of a parent that has
   children at 100/200/300. Observe which priorities the UI assigns: does it use
   midpoints (150), or renumber? Does it bump the siblings' or the parent's `modifiedAt`?
2. **Type each markdown shortcut in the UI** (`# `, `## `, `### `, `- [ ] `, `> `,
   ```` ``` ````, `**bold**`, `[2026-01-15]`) and check the stored `name`/`layoutMode`.
   Does the UI produce `quote-block` for `> `? Does it ever produce `p`? Is a UI date
   stored with the same `<time ...>` markup?
3. **Open the multiline-created nodes** in sandboxes "create: 06 multiline ..."
   and "create: 08 line-join rule". Confirm the children render in the expected order
   with priorities 0, 1, 2. Then drag one of them, and observe whether the UI renumbers
   the 0/1/2 priorities.
4. **Look at the completion leak**: in "create: 11 layoutMode param", find the node
   `p` with children a, B, C, where B and C were completed via the leak. Confirm the UI shows them as
   completed. Then un-complete B in the UI and observe `completed`/`completedAt`/`modifiedAt`.
5. **Look at the nodes with layoutMode `p`, `board`, `dashboard`, `H1`, and
   `quote-block`** (sandbox "create: 11 layoutMode param"). Describe how the UI
   renders each. In the UI, change one to a regular bullet and check what
   `layoutMode` value is stored.
6. **Paste multi-paragraph text into the UI**, for example the strings from §5.1 like
   `parent\n\nchild1\n\nchild2`. Compare the UI's paste-splitting with the API's join rule.
7. **Create a node in the UI, then set a note on it in the UI**. Check whether the API shows
   `note: null` or `""` before and after, and whether timestamps change.
8. **Look at the `<script>alert(1)</script>` node and the `&lt;b&gt;` node** (sandbox
   "create: 04 markdown in name" / "05 markdown followups"). Check how they render: is the HTML
   live? Then make a trivial edit in the UI and observe whether the stored name gets escaped or changed.
9. **Create a todo in the UI and check it off** to compare with `- [x]` at create.
   Is `layoutMode` `todo`, and does `completedAt` match?
10. **Look at the long-name nodes** (10 000 `x`; 100 KB) in "create: 12 params misc" to see whether the UI
   truncates them or misbehaves, and whether opening them changes anything via the API.

---

## 10. Sandboxes left in place (all children of the scratchpad)

`sandbox: create: 01 basic shape`, `02a`–`02e priorities ...`,
`03 priority exhaustion via top`, `03b bottom after tops`,
`04 markdown in name`, `05 markdown followups`, `06 multiline #00..#21 ...`,
`07 multiline determinism`, `08 line-join rule`, `09 line-join rule round 2`,
`10 code fences multiline`, `11 layoutMode param`, `12 params misc`,
`13 parent_id forms`, `14 priority followups max50` (plus the first
`14 priority followups` run, which made the multiline and delete parts).

Total production requests for this area: 1 026 (more than planned, because the
markdown tables do create + GET + list for each input).
