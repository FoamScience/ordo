//! What the browser is sent: the review as data it only has to lay out.
//! Every sentence here comes from the same functions the terminal draws with.

use super::{Phase, Slot};
use crate::code_view::{apply_theme_colors, theme, theme_names, theme_role_color, THEME_ROLES};
use crate::commands::COMMANDS;
use crate::draw::{
    cascade, churn_line, coverage, note_for, scope_line, top_level, unreviewed_deps,
};
use crate::highlight::{highlight_roles, RoleSpans};
use crate::keys::{key_label, ACTION_NAMES};
use crate::*;
use ratatui::style::Color;
use serde_json::{json, Value};

pub(super) fn slot(slot: &Slot) -> Value {
    match &slot.phase {
        Phase::Loading(status) => json!({
            "state": "loading", "status": status, "version": slot.version,
            "rev": slot.session.rev,
        }),
        Phase::Empty(msg) => json!({
            "state": "empty", "status": msg, "version": slot.version,
            "rev": slot.session.rev,
        }),
        Phase::Ready(app) => {
            let mut v = review(app);
            v["state"] = "ready".into();
            v["version"] = slot.version.into();
            v
        }
    }
}

fn level(l: Option<ordo::model::Level>) -> Value {
    l.map_or(Value::Null, |l| l.as_str().into())
}

fn delta(d: Option<&Delta>) -> Value {
    match d {
        Some(Delta::New) => "new".into(),
        Some(Delta::Changed) => "changed".into(),
        Some(Delta::Moved) => "moved".into(),
        Some(Delta::Same) | None => Value::Null,
    }
}

/// The whole review, one entry per hunk plus the list's rows. The facts that
/// cost more than a row should (the cascade, line churn) are `item`'s.
pub(super) fn review(app: &App) -> Value {
    // a first run compares against nothing, and "everything is new" says
    // nothing (the same rule `delta_line` keeps)
    let first_run = app.deltas.iter().all(|d| *d == Delta::New);
    let items: Vec<Value> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let what: Vec<&str> = if it.rationale == "change" {
                vec![]
            } else {
                it.rationale.split("; ").collect()
            };
            json!({
                "path": it.path,
                "old": it.old_range,
                "new": it.new_range,
                "category": format!("{:?}", it.cat).to_lowercase(),
                "rationale": it.rationale,
                "what": what,
                "details": it.details,
                "notes": it.notes,
                "edges": it.edges.iter().map(|e| json!({
                    "label": e.label,
                    "target": e.target,
                    "needs": e.dependency,
                })).collect::<Vec<_>>(),
                "symbol": app.groups.get(&it.bucket),
                "bucket": it.bucket,
                "grouped_by": scope_line(app, i),
                "noise": it.noise,
                "comment": it.comment,
                "symbols": it.symbols.iter().map(|s| &s.name).collect::<Vec<_>>(),
                "enclosing": it.enclosing,
                "findings": it.findings,
                "level": level(top_level(it)),
                "uses_at": it.uses_at,
                "executed": it.executed,
                "cluster": it.cluster,
                "wave": it.wave,
                "reviewed": app.reviewed[i],
                "delta": if first_run { Value::Null } else { delta(app.deltas.get(i)) },
                "note": note_for(app, i),
            })
        })
        .collect();
    let rows: Vec<Value> = display_rows(
        &app.view,
        &app.items,
        &app.groups,
        app.show_groups,
        &app.collapsed,
    )
    .into_iter()
    .map(|r| match r {
        DisplayRow::File(path, n) => json!({ "kind": "file", "path": path, "n": n }),
        DisplayRow::Gap => json!({ "kind": "gap" }),
        DisplayRow::Item(i, opens) => json!({ "kind": "item", "i": i, "opens": opens }),
        DisplayRow::Folded(hidden) => json!({ "kind": "folded", "items": hidden }),
    })
    .collect();
    let (done, total, both, edges) = coverage(app);
    let count = |d: Delta| app.deltas.iter().filter(|&&x| x == d).count();
    let mut levels = serde_json::Map::new();
    for &i in &app.view {
        if let Some(l) = top_level(&app.items[i]) {
            let n = levels.get(l.as_str()).and_then(Value::as_u64).unwrap_or(0);
            levels.insert(l.as_str().to_string(), (n + 1).into());
        }
    }
    json!({
        "rev": app.rev,
        "subject": app.subject,
        "uncommitted": app.uncommitted,
        "mode": match app.mode { ViewMode::Ledger => "ledger", ViewMode::Hunks => "hunks" },
        "show_groups": app.show_groups,
        "strategy": app.strategy,
        "watch": format!("{:?}", app.watch).to_lowercase(),
        "only_wave": app.only_wave,
        "wave_intents": app.waves.intents,
        "stale": app.stale.as_ref().map(Stale::line),
        "path_filter": app.path_filter.as_ref().map(|(g, _)| g),
        "comments_only": app.comments_only,
        "show_all": app.show_all,
        "view": app.view,
        "rows": rows,
        "items": items,
        "coverage": { "done": done, "total": total, "both": both, "edges": edges },
        "levels": levels,
        "delta": {
            "first_run": first_run,
            "new": count(Delta::New),
            "changed": count(Delta::Changed),
            "moved": count(Delta::Moved),
            "gone": app.delta_gone,
        },
        "rules": {
            "catalog": app.catalog,
            "own": app.rules.len(),
            "includes": app.includes,
            "disabled": app.disables,
            "report": app.rules_report,
        },
        "file_churn": app.file_churn,
        "sel": app.sel,
        "keys": app.keys.binds.iter().filter_map(|(pre, key, action)| {
            let name = ACTION_NAMES.iter().find(|(_, a)| a == action)?.0;
            Some(json!({ "prefix": pre.map(key_label), "key": key_label(*key), "action": name }))
        }).collect::<Vec<_>>(),
        "commands": COMMANDS.iter().map(|c| json!({
            "name": c.name, "args": c.args, "help": c.help,
        })).collect::<Vec<_>>(),
    })
}

/// The why pane's costlier half for hunk `i`.
pub(super) fn item(app: &App, i: usize) -> Value {
    let it = &app.items[i];
    let comments: Vec<Value> = app
        .comments
        .iter()
        .filter(|c| {
            let [a, b] = it.new_range;
            c.path == it.path && c.start <= b.max(a) && a <= c.end
        })
        .map(|c| json!({ "start": c.start, "end": c.end, "text": c.text, "stale": c.stale }))
        .collect();
    let key = (it.path.clone(), it.new_range[0], it.new_range[1]);
    let line_churn = app.churn_cache.get(&key).and_then(|c| c.as_ref()).map(|c| {
        json!({
            "commits": c.commits,
            "capped": c.capped,
            "last": c.last.as_ref().map(|(who, when)| json!({ "author": who, "date": when })),
        })
    });
    json!({
        "i": i,
        "cascade": cascade(app, i),
        "out_of_order": if app.reviewed[i] { unreviewed_deps(app, i) } else { vec![] },
        "churn": churn_line(app, i),
        "line_churn": line_churn,
        "comments": comments,
    })
}

fn spans(lines: Option<Vec<RoleSpans>>) -> Value {
    lines.map_or(Value::Null, |ls| {
        ls.into_iter()
            .map(|l| {
                l.into_iter()
                    .map(|(t, r)| json!([t, r.name()]))
                    .collect::<Vec<_>>()
            })
            .collect()
    })
}

/// One file as the code pane draws it: both sides, highlighted by role, and
/// every hunk of the review that falls in it.
pub(super) fn file(app: &App, path: &str) -> Value {
    let Some((old, new)) = app.sources.get(path) else {
        return json!({ "path": path, "missing": true });
    };
    let hunks: Vec<Value> = app
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.path == path)
        .map(|(i, it)| {
            json!({
                "i": i,
                "old": it.old_range,
                "new": it.new_range,
                "removed": it.refined.removed,
                "added": it.refined.added,
            })
        })
        .collect();
    let comments: Vec<Value> = app
        .comments
        .iter()
        .filter(|c| c.path == path)
        .map(|c| json!({ "start": c.start, "end": c.end, "text": c.text, "stale": c.stale }))
        .collect();
    json!({
        "path": path,
        "old": old,
        "new": new,
        "old_spans": spans(highlight_roles(path, &old.join("\n"))),
        "new_spans": spans(highlight_roles(path, &new.join("\n"))),
        "hunks": hunks,
        "comments": comments,
    })
}

pub(super) fn popup(p: &Popup) -> Value {
    let lines: Vec<String> = p
        .lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    json!({ "title": p.title, "lines": lines })
}

fn hex(c: Color) -> Value {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}").into(),
        _ => Value::Null,
    }
}

/// Every theme by name, each role's colour (or null where the theme leaves it
/// to the terminal, which the browser's own default then decides), with
/// `tui.toml`'s `[theme]` overrides on top — the same file the terminal reads.
pub(super) fn themes() -> Value {
    let cfg = config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_key_config(&t));
    let chosen = cfg.as_ref().and_then(|c| c.theme.clone());
    let mut out = serde_json::Map::new();
    for name in theme_names().into_iter().filter(|n| n != "terminal") {
        let Some(t) = theme(&name) else { continue };
        let t = match &cfg {
            Some(c) => apply_theme_colors(t, &c.colors),
            None => t,
        };
        let roles: serde_json::Map<String, Value> = THEME_ROLES
            .iter()
            .map(|r| (r.to_string(), hex(theme_role_color(&t, r))))
            .collect();
        out.insert(name, Value::Object(roles));
    }
    json!({ "themes": out, "chosen": chosen })
}
