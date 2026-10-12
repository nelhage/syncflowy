"""Batch 2: allowed/disallowed/malformed HTML tags, via create and update."""
from common import run_batch

CASES = [
    ("b", "x <b>bold</b> y"),
    ("i", "x <i>it</i> y"),
    ("s", "x <s>strike</s> y"),
    ("code", "x <code>a<b</code> y"),
    ("a", 'x <a href="https://example.com/?a=1&b=2">link</a> y'),
    ("a-single-quote-attr", "x <a href='https://example.com'>link</a> y"),
    ("a-extra-attrs", '<a href="https://example.com" target="_blank" class="z" onclick="evil()">l</a>'),
    ("a-no-href", "<a>nohref</a>"),
    ("a-js-href", '<a href="javascript:alert(1)">js</a>'),
    ("b-with-attr", '<b class="x" style="color:red">bold</b>'),
    ("nested-bi", "<b><i>bi</i></b>"),
    ("nested-misnested", "<b><i>bi</b></i>"),
    ("uppercase-tag", "<B>up</B>"),
    ("strong-em", "<strong>st</strong> <em>em</em>"),
    ("u", "<u>under</u>"),
    ("span", '<span class="colored c-red">sp</span>'),
    ("span-plain", "<span>sp</span>"),
    ("script", "a<script>alert(1)</script>b"),
    ("div", "<div>dv</div>"),
    ("br", "a<br>b<br/>c"),
    ("img", '<img src="x.png" alt="pic">tail'),
    ("unclosed-b", "<b>unclosed"),
    ("stray-close", "text</b> more"),
    ("lt-letter", "a <notatag b"),
    ("comment", "a<!-- c -->b"),
    ("time", '<time startYear="2026" startMonth="1" startDay="15">Jan 15, 2026</time>'),
    ("mark", "<mark>hl</mark>"),
    ("p", "<p>para</p>"),
]
if __name__ == "__main__":
    run_batch("02_html_tags", CASES)
