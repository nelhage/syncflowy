"""Batch 9: which control / special code points are stripped, per path and field."""
from common import run_batch

CPS = [0x00, 0x01, 0x08, 0x09, 0x0b, 0x0c, 0x0e, 0x1b, 0x1f, 0x7f, 0x80, 0x85, 0x9f,
       0xa0, 0xad, 0x2028, 0x2029, 0x200e, 0x202e, 0x2066, 0xfeff, 0xfffd, 0xfffe, 0xffff,
       0xe000, 0x1f600, 0x10ffff]
CASES = [(f"U+{cp:04X}", f"a{chr(cp)}b") for cp in CPS]
CASES.append(("all-c0-except-tab-nl-cr", "".join(chr(i) for i in range(0x20) if i not in (9, 10, 13)) + "end"))
if __name__ == "__main__":
    run_batch("09_control_chars", CASES)
