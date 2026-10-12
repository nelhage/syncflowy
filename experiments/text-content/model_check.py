"""Offline check: does the inferred create-name model (docs §5–7) reproduce the
observed production outputs for single-line inputs? No network access."""
import datetime, html, json, re
from pathlib import Path

R = Path(__file__).resolve().parent / "results"
STRIP = re.compile(r"[\x00-\x08\x0e-\x1f]")
ESC = {c: f"\x00E{i}\x00" for i, c in enumerate("*[]`()")}
MON = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split()
DOW = "Mon Tue Wed Thu Fri Sat Sun".split()


def time_elem(m):
    y, mo, d = int(m[1]), int(m[2]), int(m[3])
    try:
        dt = datetime.date(y, mo, d)
    except ValueError:
        return m[0]
    attrs = f'startYear="{y}" startMonth="{mo}" startDay="{d}"'
    text = f"{DOW[dt.weekday()]}, {MON[mo-1]} {d}, {y}"
    if m[4] is not None:
        h, mi = int(m[4]), int(m[5])
        if h > 23 or mi > 59:
            return m[0]
        attrs += f' startHour="{h}" startMinute="{mi}"'
        text += f" at {(h % 12) or 12}:{mi:02d} {'AM' if h < 12 else 'PM'}"
    return f"<time {attrs}>{text}</time>"


def inline(s):
    s = re.sub(r"\\([*\[\]`()])", lambda m: ESC[m[1]], s)
    s = re.sub(r"`(.*?)`", r"<code>\1</code>", s)
    s = re.sub(r"\*\*(.*?)\*\*", r"<b>\1</b>", s)
    s = re.sub(r"\*(.*?)\*", r"<i>\1</i>", s)
    s = re.sub(r"~~(.*?)~~", r"<s>\1</s>", s)
    s = re.sub(r"\[(\d{4})-(\d{1,2})-(\d{1,2})(?: (\d{1,2}):(\d{2}))?\]", time_elem, s)
    s = re.sub(r"\[(.*?)\]\((.*?)\)", r'<a href="\2">\1</a>', s)
    for k, v in ESC.items():
        s = s.replace(v, k)
    return s


def create_name_single(v):
    v = html.unescape(STRIP.sub("", v)).strip()
    return inline(v)


ok = bad = 0
for name in ["01_unicode_whitespace", "02_html_tags", "03_markdown", "04_md_parser_edges",
             "05_escapes_order_dates", "09_control_chars", "13_trim_and_md_in_html"]:
    for rec in json.loads((R / f"{name}.json").read_text())["cases"]:
        if rec.get("create_status") != 200 or re.search(r"[\n\r\x0b\x0c\x85\u2028\u2029]", rec["input"]):
            continue
        if re.match(r"\s*(#|- |```)", rec["input"]):
            continue
        got, want = create_name_single(rec["input"]), rec["create_name"]
        if got == want:
            ok += 1
        else:
            bad += 1
            print(f"MISMATCH {name}/{rec['label']}: input={rec['input']!r}\n   want={want!r}\n   got ={got!r}")
print(f"ok={ok} bad={bad}")
