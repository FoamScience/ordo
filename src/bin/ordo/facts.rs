// ---------------------------------------------------------------------- facts
// What the review says about a hunk beyond its rationale.
use crate::keys::ViewMode;
use crate::marks::note_key;
use crate::view_pos;
use crate::App;
use crate::Item;
use ordo::model::Level;
use std::collections::HashSet;

/// The hardest-pressing finding level on a hunk, if anything was found.
pub(super) fn top_level(it: &Item) -> Option<Level> {
    it.findings.iter().map(|f| f.level).max_by_key(|l| *l as u8)
}

/// A draft rule matching the shape of item `i`, as TOML the reviewer can paste
/// into `.ordo/rules.toml` (P23.6).
///
/// Every condition comes from a structural fact the engine already recorded
/// about this hunk — no LLM, no guessing, and the same facts the rules engine
/// will evaluate it against. It is deliberately a *draft*: the conditions are
/// as specific as the evidence allows, so the reviewer's job is to delete the
/// ones that were incidental rather than to invent the ones that matter.
///
/// The limits are emitted one below what this hunk actually measured, so the
/// rule fires on the hunk that prompted it.
pub(super) fn draft_rule(app: &App, i: usize) -> Vec<String> {
    let it = &app.items[i];
    let mut when: Vec<String> = vec![];

    if let Some(spec) = ordo::lang_name_for_path(&it.path) {
        when.push(format!("lang = \"{spec}\""));
    }
    let mut kinds: Vec<String> = it.symbols.iter().map(|s| s.kind.clone()).collect();
    kinds.sort();
    kinds.dedup();
    if !kinds.is_empty() {
        let list = kinds
            .iter()
            .map(|k| format!("\"{k}\""))
            .collect::<Vec<_>>()
            .join(", ");
        when.push(format!("kind = [{list}]"));
    }
    // the structural notes are already measurements; turn each into the limit
    // it just exceeded
    for n in &it.notes {
        let num = |prefix: &str, suffix: &str| -> Option<usize> {
            let rest = n.strip_prefix(prefix)?.strip_suffix(suffix)?;
            rest.trim().parse().ok()
        };
        if let Some(p) = n
            .strip_suffix(" params")
            .and_then(|v| v.parse::<usize>().ok())
        {
            when.push(format!("max-params = {}", p.saturating_sub(1)));
        } else if let Some(l) = num("large definition (", " lines)") {
            when.push(format!("max-lines = {}", l.saturating_sub(1)));
        } else if let Some(d) = num("deeply nested (depth ", ")") {
            when.push(format!("max-nesting = {}", d.saturating_sub(1)));
        }
    }
    let name = kinds
        .first()
        .map(|k| format!("no-{}", k.replace('_', "-")))
        .unwrap_or_else(|| "unnamed-rule".to_string());
    let mut out = vec![
        "# paste into .ordo/rules.toml, then delete the conditions that were".to_string(),
        "# incidental — every line below is a fact about the hunk you flagged.".to_string(),
        String::new(),
        "[[rule]]".to_string(),
        format!("name = \"{name}\""),
    ];
    out.extend(when);
    out.push("warn = \"TODO: say why this shape is unwanted\"".to_string());
    if it.symbols.is_empty() {
        out.push(String::new());
        out.push("# this hunk declares no symbol, so the draft has no `kind` to".to_string());
        out.push("# match on — it will be broader than you probably want.".to_string());
    }
    out
}

/// Everything that would lose its footing if item `i` were rejected — the
/// hunks that depend on it, and the hunks that depend on those.
///
/// The transitive closure matters more than the direct dependents: pushing
/// back on a leaf when the root is the problem sends the author round the loop
/// twice. Cycles (mutual recursion is a real def→use cycle) terminate on the
/// visited set rather than hanging.
pub(super) fn cascade(app: &App, i: usize) -> Vec<usize> {
    let mut seen: HashSet<usize> = HashSet::new();
    let mut queue = vec![i];
    while let Some(cur) = queue.pop() {
        for (j, it) in app.items.iter().enumerate() {
            if seen.contains(&j) || j == i {
                continue;
            }
            let depends = it
                .edges
                .iter()
                .any(|e| e.dependency && e.target == Some(cur));
            if depends {
                seen.insert(j);
                queue.push(j);
            }
        }
    }
    // report in reading order, which is the order the author would fix them in
    let mut out: Vec<usize> = seen.into_iter().filter(|j| app.view.contains(j)).collect();
    out.sort_by_key(|&j| view_pos(&app.view, j));
    out
}

/// Dependencies of item `i` that are part of this review but not yet reviewed
/// — the hunks defining what `i` uses. Marking `i` reviewed while any of these
/// are outstanding means a call was approved before its callee.
pub(super) fn unreviewed_deps(app: &App, i: usize) -> Vec<usize> {
    app.items[i]
        .edges
        .iter()
        .filter(|e| e.dependency)
        .filter_map(|e| e.target)
        .filter(|&t| !app.reviewed[t])
        .collect()
}

/// The dependencies hunk `i` was approved ahead of. An unreviewed hunk is
/// never out of order: nothing has been approved yet.
pub(super) fn out_of_order(app: &App, i: usize) -> Vec<usize> {
    if app.reviewed[i] {
        unreviewed_deps(app, i)
    } else {
        vec![]
    }
}

/// How much of the review is actually understood, as two numbers.
///
/// Hunk coverage is what every tool reports. Edge coverage — a def→use link
/// with *both* ends reviewed — is the one that tracks whether the relationship
/// between two places was checked, which is the thing a reading order exists to
/// make possible. Only edges whose ends are both in the current view count, so
/// filtering the review does not make the number look better than it is.
pub(super) fn coverage(app: &App) -> (usize, usize, usize, usize) {
    let done = app.view.iter().filter(|&&i| app.reviewed[i]).count();
    let mut edges = 0;
    let mut both = 0;
    for &i in &app.view {
        for e in app.items[i].edges.iter().filter(|e| e.dependency) {
            let Some(t) = e.target else { continue };
            if !app.view.contains(&t) {
                continue;
            }
            edges += 1;
            if app.reviewed[i] && app.reviewed[t] {
                both += 1;
            }
        }
    }
    (done, app.view.len(), both, edges)
}

/// The header a ledger entry shows in the list — the same sentence `pack`
/// prints, minus the location the row already carries.
fn ledger_label(e: &ordo::model::LedgerEntry) -> String {
    let change = format!("{:?}", e.change).to_lowercase();
    let from = match e.from.as_deref() {
        Some(f) => format!(" from {f}"),
        None => String::new(),
    };
    let fan = match e.used_by.len() {
        0 => String::new(),
        1 => ", used by 1 hunk".to_string(),
        n => format!(", used by {n} hunks"),
    };
    format!("{} — {change}{from}{fan}", e.name)
}

/// What holds item `i` together with its neighbours: in ledger mode the ledger
/// entry it belongs to, otherwise (or with no entry) the engine's group reason.
pub(super) fn scope_line(app: &App, i: usize) -> Option<String> {
    let it = &app.items[i];
    let entry = match app.mode {
        ViewMode::Ledger => it.ledger.and_then(|l| app.symbol_ledger.get(l)),
        ViewMode::Hunks => None,
    };
    entry
        .map(ledger_label)
        .or_else(|| app.group_reasons.get(&it.group).cloned())
}

/// The note anchored to item `i`'s symbol, if any.
pub(super) fn note_for(app: &App, i: usize) -> Option<&str> {
    let key = note_key(&app.items[i])?;
    app.notes.get(&key).map(String::as_str)
}
