//! Sibling-order merge for one parent (§6.3).

use std::collections::BTreeSet;

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::tree::NodeId;

/// The elements of `side` that belong to a longest common subsequence of `base` and
/// `side` (Myers' diff is minimal, so its `Equal` runs form an LCS).
pub(crate) fn lcs_members(base: &[NodeId], side: &[NodeId]) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    for op in capture_diff_slices(Algorithm::Myers, base, side) {
        if let DiffOp::Equal { new_index, len, .. } = op {
            out.extend(side[new_index..new_index + len].iter().cloned());
        }
    }
    out
}

/// Merge the visible order of one parent's children (§6.3).
///
/// * `base`: the order the Markdown user saw (children in `base_doc`); empty if the
///   parent was not visible in the base.
/// * `local`: the order in the current file (identities after the merge, temp ids
///   included); `None` if the parent is not in the file.
/// * `remote`: the remote visible order.
/// * `keep`: exactly the children the result must contain (the visible children of the
///   parent in the merged tree).
/// * `fallback`: an order containing every element of `keep` (the merged tree's sibling
///   order), used for elements that neither `remote` nor `local` place.
/// * `reinsert_local`: false in safe mode (§9.1 guard 1: no reorders).
///
/// Each side's *moved set* is the set of its elements outside the LCS of the base and
/// that side. The result starts from `remote` (restricted to `keep`) without the
/// locally moved nodes that remote did not move; then every locally moved or created
/// node (in local order) is inserted right after its nearest local predecessor already
/// in the result, or first if it has none. Move vs move: the remote position wins.
/// Remaining elements of `keep` follow in `fallback` order.
///
/// A priority tie in `remote` never causes a write: if `local` equals `base` the moved
/// set is empty and the result is the remote order itself.
pub fn merge_sibling_order(
    base: &[NodeId],
    local: Option<&[NodeId]>,
    remote: &[NodeId],
    keep: &BTreeSet<NodeId>,
    fallback: &[NodeId],
    reinsert_local: bool,
) -> Vec<NodeId> {
    let mut reinsert: BTreeSet<NodeId> = BTreeSet::new();
    if let (Some(local), true) = (local, reinsert_local) {
        let lcs_local = lcs_members(base, local);
        let lcs_remote = lcs_members(base, remote);
        let remote_set: BTreeSet<&NodeId> = remote.iter().collect();
        for x in local {
            if lcs_local.contains(x) {
                continue;
            }
            // Remote moved it too (or created it): remote wins.
            if remote_set.contains(x) && !lcs_remote.contains(x) {
                continue;
            }
            reinsert.insert(x.clone());
        }
    }
    let mut out: Vec<NodeId> = remote
        .iter()
        .filter(|x| keep.contains(*x) && !reinsert.contains(*x))
        .cloned()
        .collect();
    let mut placed: BTreeSet<NodeId> = out.iter().cloned().collect();
    if let Some(local) = local {
        for (i, x) in local.iter().enumerate() {
            if !reinsert.contains(x) || !keep.contains(x) || placed.contains(x) {
                continue;
            }
            let pred = local[..i].iter().rev().find(|y| placed.contains(*y));
            let at = match pred {
                Some(y) => out.iter().position(|z| z == y).expect("placed") + 1,
                None => 0,
            };
            out.insert(at, x.clone());
            placed.insert(x.clone());
        }
    }
    for x in fallback {
        if keep.contains(x) && !placed.contains(x) {
            out.push(x.clone());
            placed.insert(x.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(s: &str) -> Vec<NodeId> {
        s.chars().map(|c| NodeId::from_api(c.to_string())).collect()
    }

    fn run(b: &str, l: Option<&str>, r: &str, keep: &str) -> String {
        let keep_v = ids(keep);
        let keep_s: BTreeSet<NodeId> = keep_v.iter().cloned().collect();
        let l = l.map(ids);
        merge_sibling_order(&ids(b), l.as_deref(), &ids(r), &keep_s, &keep_v, true)
            .iter()
            .map(|x| x.as_str())
            .collect()
    }

    #[test]
    fn unchanged_local_takes_remote() {
        assert_eq!(run("abc", Some("abc"), "cab", "abc"), "cab");
    }

    #[test]
    fn local_move_reinserted_after_predecessor() {
        assert_eq!(run("abcd", Some("bcad"), "abcd", "abcd"), "bcad");
        // Remote inserted x after a; local moved d to the front.
        assert_eq!(run("abcd", Some("dabc"), "axbcd", "abcdx"), "daxbc");
    }

    #[test]
    fn local_create_placed_after_predecessor() {
        assert_eq!(run("ab", Some("anb"), "ab", "abn"), "anb");
        assert_eq!(run("ab", Some("nab"), "ba", "abn"), "nba");
    }

    #[test]
    fn move_vs_move_remote_wins() {
        // Both moved a: remote to the end, local to the middle.
        assert_eq!(run("abc", Some("bac"), "bca", "abc"), "bca");
    }

    #[test]
    fn safe_mode_keeps_remote_order() {
        let keep_v = ids("abcn");
        let keep: BTreeSet<NodeId> = keep_v.iter().cloned().collect();
        let out = merge_sibling_order(
            &ids("abc"),
            Some(&ids("cnba")),
            &ids("abc"),
            &keep,
            &keep_v,
            false,
        );
        assert_eq!(out, ids("abcn"));
    }
}
