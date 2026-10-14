//! A small diff3 over token sequences (characters or lines), built on `similar` (§5.2).

use std::hash::Hash;
use std::ops::Range;

use similar::{Algorithm, DiffTag, capture_diff_slices};

/// One change of a side relative to the base: base range -> range in the side.
#[derive(Debug, Clone)]
struct Hunk {
    base: Range<usize>,
    side: Range<usize>,
    ours: bool,
}

fn hunks<T: Eq + Hash + Ord>(base: &[T], side: &[T], ours: bool) -> Vec<Hunk> {
    capture_diff_slices(Algorithm::Myers, base, side)
        .into_iter()
        .filter_map(|op| {
            let (tag, b, s) = op.as_tag_tuple();
            (tag != DiffTag::Equal).then_some(Hunk {
                base: b,
                side: s,
                ours,
            })
        })
        .collect()
}

/// The text a side gives to `base[start..end]`, given its hunks inside that range.
fn side_text<T: Clone>(base: &[T], side: &[T], hs: &[&Hunk], start: usize, end: usize) -> Vec<T> {
    let mut out = Vec::new();
    let mut p = start;
    for h in hs {
        out.extend_from_slice(&base[p..h.base.start]);
        out.extend_from_slice(&side[h.side.clone()]);
        p = h.base.end;
    }
    out.extend_from_slice(&base[p..end]);
    out
}

/// Merge the changes `base -> ours` and `base -> theirs`. Changes that overlap or touch
/// in the base must be identical, otherwise the merge conflicts (`None`). Conservative:
/// two insertions at the same point conflict unless they are equal.
pub(crate) fn merge3<T: Clone + Eq + Hash + Ord>(
    base: &[T],
    ours: &[T],
    theirs: &[T],
) -> Option<Vec<T>> {
    if ours == base || ours == theirs {
        return Some(theirs.to_vec());
    }
    if theirs == base {
        return Some(ours.to_vec());
    }
    let mut all = hunks(base, ours, true);
    all.extend(hunks(base, theirs, false));
    all.sort_by_key(|h| (h.base.start, h.base.end, !h.ours));
    let mut out = Vec::new();
    let mut p = 0;
    let mut i = 0;
    while i < all.len() {
        let start = all[i].base.start;
        let mut end = all[i].base.end;
        let mut j = i + 1;
        while j < all.len() && all[j].base.start <= end {
            end = end.max(all[j].base.end);
            j += 1;
        }
        let group = &all[i..j];
        out.extend_from_slice(&base[p..start]);
        let o: Vec<&Hunk> = group.iter().filter(|h| h.ours).collect();
        let t: Vec<&Hunk> = group.iter().filter(|h| !h.ours).collect();
        let text_o = side_text(base, ours, &o, start, end);
        let text_t = side_text(base, theirs, &t, start, end);
        if o.is_empty() {
            out.extend(text_t);
        } else if t.is_empty() || text_o == text_t {
            out.extend(text_o);
        } else {
            return None;
        }
        p = end;
        i = j;
    }
    out.extend_from_slice(&base[p..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(b: &str, o: &str, t: &str) -> Option<String> {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        merge3(&c(b), &c(o), &c(t)).map(|v| v.into_iter().collect())
    }

    #[test]
    fn merges() {
        assert_eq!(m("abc", "abc", "aXbc").as_deref(), Some("aXbc"));
        assert_eq!(m("abc", "aYbc", "abc").as_deref(), Some("aYbc"));
        assert_eq!(
            m("hello world", "Hello world", "hello world!").as_deref(),
            Some("Hello world!")
        );
        assert_eq!(m("abc", "aXc", "aYc"), None);
        assert_eq!(m("abc", "aXc", "aXc").as_deref(), Some("aXc"));
        assert_eq!(m("ab", "aXb", "aYb"), None);
        assert_eq!(
            m("plain bold", "plain bold!", "plain <b>bold</b>").as_deref(),
            None,
            "an insertion touching a change conflicts"
        );
        assert_eq!(
            m("x = 1 + 2", "y = 1 + 2", "x = 1 + <b>2</b>").as_deref(),
            Some("y = 1 + <b>2</b>")
        );
    }
}
