"""Batch 1: unicode, whitespace (no newlines), HTML special chars, entities."""
from common import run_batch

CASES = [
    ("plain", "hello world"),
    ("emoji", "emoji 😀👍🏽👨‍👩‍👧 end"),
    ("cjk", "中文 日本語 한국어"),
    ("rtl", "עברית العربية mixed"),
    ("combining", "é vs é"),            # NFD vs NFC
    ("zero-width", "a​b‌c‍d﻿e"),
    ("astral-math", "𝔘𝔫𝔦𝔠𝔬𝔡𝔢"),
    ("leading-trailing-space", "  padded  "),
    ("only-spaces", "   "),
    ("empty", ""),
    ("internal-multi-space", "a    b"),
    ("tab", "a\tb\t"),
    ("nbsp", "a b"),
    ("control-chars", "a\u0001b\u0007c\u007f"),
    ("lt-gt-amp", "a < b > c & d"),
    ("quotes", "say \"hi\" it's"),
    ("lt-no-space", "1<2 and 3>2"),
    ("entity-amp", "&amp; &lt; &gt; &quot; &#39; &nbsp; &#x41; &copy;"),
    ("bare-amp-word", "AT&T R&D"),
    ("backslash", "a\\b \\n \\*"),
]
if __name__ == "__main__":
    run_batch("01_unicode_whitespace", CASES)
