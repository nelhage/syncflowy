//! §9.1 guard 6: other writers on the clients. The Tasks plugin appends a done date to
//! a line when its box is ticked; pushed as is, the date would land in the Workflowy name
//! just before the node is completed. Pure.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::mdblock::{Checkbox, ItemKind, LocalDoc, LocalItem};
use crate::tree::ShortId;

/// A Tasks-plugin done-date field at the end of a line: the emoji form `✅ 2026-10-13`
/// and the Dataview form `[completion:: 2026-10-13]`, with the whitespace before it.
static TASKS_DONE_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|\s+)(?:✅\x{FE0F}?\s*\d{4}-\d{2}-\d{2}|\[completion::\s*\d{4}-\d{2}-\d{2}\])\s*$",
    )
    .expect("Tasks field pattern")
});

/// Strip the Tasks-plugin done-date fields that `name` (the Markdown name of a ticked
/// line) has at its end and `base` (the same item's name in the base render, if any)
/// does not: they were added by the plugin when the box was ticked, not typed. `None`
/// if nothing is stripped.
pub fn strip_done_fields(name: &str, base: Option<&str>) -> Option<String> {
    let mut s = name.to_owned();
    let mut changed = false;
    while let Some(m) = TASKS_DONE_FIELD.find(&s) {
        let field = m.as_str().trim().to_owned();
        let here = s.matches(field.as_str()).count();
        let before = base.map_or(0, |b| b.matches(field.as_str()).count());
        if here <= before {
            break;
        }
        s.truncate(m.start());
        let trimmed = s.trim_end().len();
        s.truncate(trimmed);
        changed = true;
    }
    changed.then_some(s)
}

/// §9.1 guard 6: the Tasks plugin appends `✅ YYYY-MM-DD` to a line when its box is
/// ticked. Pushed as is, the date would land in the Workflowy name just before the node
/// is completed. Returns `local` with those fields removed from ticked items (only the
/// ones the base render of the item did not have), and one note per item changed;
/// `None` if no item changed. Other Tasks fields (`📅` due dates are this engine's date
/// syntax, `⏫` priorities, `🔁` recurrences) are left alone. UNVERIFIED: the plugin's
/// exact output on synced lines (with a trailing `%%wf:…%%` or `^wf-…` tag) has not been
/// observed; the parser removes tags wherever they are on the line, so the field is at
/// the end of the name either way.
pub fn strip_tasks_fields(local: &LocalDoc, base: &LocalDoc) -> Option<(LocalDoc, Vec<String>)> {
    let base_names: BTreeMap<ShortId, &str> = base
        .walk()
        .into_iter()
        .filter(|(_, it)| !it.is_marker())
        .filter_map(|(_, it)| {
            it.tag
                .as_ref()
                .map(|t| (t.short.clone(), it.name_md.as_str()))
        })
        .collect();
    fn rec(items: &mut [LocalItem], base: &BTreeMap<ShortId, &str>, notes: &mut Vec<String>) {
        for it in items {
            if it.checkbox == Checkbox::Ticked
                && !it.is_marker()
                && !matches!(it.kind, ItemKind::Code | ItemKind::Placeholder)
            {
                let before = it.tag.as_ref().and_then(|t| base.get(&t.short).copied());
                if let Some(name) = strip_done_fields(&it.name_md, before) {
                    notes.push(format!(
                        "line {}: Tasks-plugin done date removed before completing",
                        it.line
                    ));
                    it.name_md = name;
                }
            }
            rec(&mut it.children, base, notes);
        }
    }
    let mut doc = local.clone();
    let mut notes = Vec::new();
    rec(&mut doc.items, &base_names, &mut notes);
    (!notes.is_empty()).then_some((doc, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_fields() {
        assert_eq!(
            strip_done_fields("Buy milk ✅ 2026-10-13", None).as_deref(),
            Some("Buy milk")
        );
        assert_eq!(
            strip_done_fields("Buy milk [completion:: 2026-10-13]", Some("Buy milk")).as_deref(),
            Some("Buy milk")
        );
        // Present in the base: the user's own text.
        assert_eq!(
            strip_done_fields("Paid ✅ 2026-01-01", Some("Paid ✅ 2026-01-01")),
            None
        );
        // A second date added by the plugin.
        assert_eq!(
            strip_done_fields(
                "Paid ✅ 2026-01-01 ✅ 2026-10-13",
                Some("Paid ✅ 2026-01-01")
            )
            .as_deref(),
            Some("Paid ✅ 2026-01-01")
        );
        assert_eq!(strip_done_fields("Buy milk", None), None);
        assert_eq!(strip_done_fields("Release ✅ 2026-10-13 notes", None), None);
    }
}
