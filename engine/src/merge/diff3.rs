//! Line-level three-way merge for notes (§6.1: "for notes, try a line-level diff3 first:
//! concurrent edits to different lines merge cleanly").
//!
//! Each side's edit is the list of hunks of `diff(base, side)` (Myers, from `similar`).
//! Two hunks from different sides conflict when they overlap: their base ranges
//! intersect, both insert at the same base position, or one inserts strictly inside the
//! other's replaced range. Hunks that merely touch (an edit of line 1 and an edit of
//! line 2) do not conflict. Identical hunks on both sides are applied once.
//! UNVERIFIED design choice: git's merge treats adjacent-line edits as a conflict; the
//! design asks for "different lines" to merge cleanly, so we do not.

use similar::{Algorithm, DiffOp, capture_diff_slices};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk<'a> {
    /// Start of the replaced base range.
    start: usize,
    /// End (exclusive) of the replaced base range; `start == end` is an insertion.
    end: usize,
    /// Replacement lines.
    lines: Vec<&'a str>,
}

fn hunks<'a>(base: &[&'a str], side: &[&'a str]) -> Vec<Hunk<'a>> {
    let mut out: Vec<Hunk<'a>> = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, base, side) {
        let (start, end, new) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index, old_index + old_len, new_index..new_index),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index, old_index, new_index..new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                old_index,
                old_index + old_len,
                new_index..new_index + new_len,
            ),
        };
        match out.last_mut() {
            Some(last) if last.end == start => {
                last.end = end;
                last.lines.extend_from_slice(&side[new]);
            }
            _ => out.push(Hunk {
                start,
                end,
                lines: side[new].to_vec(),
            }),
        }
    }
    out
}

fn conflicts(a: &Hunk<'_>, b: &Hunk<'_>) -> bool {
    if a == b {
        return false;
    }
    let a_ins = a.start == a.end;
    let b_ins = b.start == b.end;
    match (a_ins, b_ins) {
        (false, false) => a.start < b.end && b.start < a.end,
        (true, true) => a.start == b.start,
        (true, false) => b.start < a.start && a.start < b.end,
        (false, true) => a.start < b.start && b.start < a.end,
    }
}

/// Merge `ours` and `theirs`, both edited from `base`, line by line (`\n`-separated).
/// `None` if the edits overlap.
pub(crate) fn diff3_lines(base: &str, ours: &str, theirs: &str) -> Option<String> {
    if ours == theirs || base == theirs {
        return Some(ours.to_owned());
    }
    if base == ours {
        return Some(theirs.to_owned());
    }
    let b: Vec<&str> = base.split('\n').collect();
    let o: Vec<&str> = ours.split('\n').collect();
    let t: Vec<&str> = theirs.split('\n').collect();
    let ho = hunks(&b, &o);
    let ht = hunks(&b, &t);
    for x in &ho {
        for y in &ht {
            if conflicts(x, y) {
                return None;
            }
        }
    }
    let mut all: Vec<Hunk<'_>> = ho.clone();
    for y in ht {
        if !ho.contains(&y) {
            all.push(y);
        }
    }
    // Insertions before ranges starting at the same position.
    all.sort_by_key(|h| (h.start, h.end != h.start, h.end));
    let mut out: Vec<&str> = Vec::with_capacity(b.len());
    let mut pos = 0;
    for h in all {
        if h.start < pos {
            // Cannot happen after the conflict check; refuse rather than corrupt.
            return None;
        }
        out.extend_from_slice(&b[pos..h.start]);
        out.extend_from_slice(&h.lines);
        pos = h.end;
    }
    out.extend_from_slice(&b[pos..]);
    Some(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::diff3_lines;

    #[test]
    fn different_lines_merge() {
        assert_eq!(
            diff3_lines("a\nb\nc", "A\nb\nc", "a\nb\nC").as_deref(),
            Some("A\nb\nC")
        );
        // Adjacent lines merge too.
        assert_eq!(
            diff3_lines("a\nb\nc", "A\nb\nc", "a\nB\nc").as_deref(),
            Some("A\nB\nc")
        );
        // Insertions at different places.
        assert_eq!(
            diff3_lines("a\nb", "x\na\nb", "a\nb\ny").as_deref(),
            Some("x\na\nb\ny")
        );
    }

    #[test]
    fn same_line_conflicts() {
        assert_eq!(diff3_lines("a\nb", "A\nb", "a2\nb"), None);
        assert_eq!(diff3_lines("a", "a\nx", "a\ny"), None);
    }

    #[test]
    fn identical_edits_apply_once() {
        assert_eq!(
            diff3_lines("a\nb\nc", "a\nB\nc\nd", "a\nB\nc").as_deref(),
            Some("a\nB\nc\nd")
        );
    }

    #[test]
    fn trivial_cases() {
        assert_eq!(diff3_lines("a", "a", "b").as_deref(), Some("b"));
        assert_eq!(diff3_lines("a", "b", "a").as_deref(), Some("b"));
        assert_eq!(diff3_lines("", "x", "").as_deref(), Some("x"));
    }
}
