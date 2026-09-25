// -------------------------------------------------------------------- reload
//! What a reload keeps. Reading the change again — `:e`, `r`, or `--watch` —
//! builds a fresh `App`; without this the reviewer lands back on the first
//! hunk with every filter, search and jump gone. The selection is carried by
//! hunk identity (its symbol and file, as the since-last-look snapshot keys
//! it), everything positional relative to that hunk.

use crate::commands::run_strategy;
use crate::commands::set_filters;
use crate::keys::{Pane, ViewMode};
use crate::marks::snap_key;
use crate::search::text_matches;
use crate::{
    auto_scroll, clamp_cursor, select, set_mode, App, Cursor, Item, PathGlobs, Search, SearchKind,
};
use ordo::model::{LedgerEntry, SymbolChange};

/// A hunk named so it can be found in another load: its snapshot key and
/// which of the hunks sharing that key it is, and — for when the key no longer
/// matches because the symbol was renamed — its file and symbol names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HunkId {
    key: String,
    nth: usize,
    path: String,
    names: Vec<String>,
}

pub(super) fn hunk_id(items: &[Item], i: usize) -> HunkId {
    let key = snap_key(&items[i]);
    HunkId {
        nth: items[..i].iter().filter(|it| snap_key(it) == key).count(),
        key,
        path: items[i].path.clone(),
        names: items[i].symbols.iter().map(|s| s.name.clone()).collect(),
    }
}

/// The hunk `id` names in `items`; failing that, the one defining what the
/// engine's ledger says one of its symbols was renamed to.
pub(super) fn find_hunk(items: &[Item], ledger: &[LedgerEntry], id: &HunkId) -> Option<usize> {
    let same = items
        .iter()
        .enumerate()
        .filter(|(_, it)| snap_key(it) == id.key)
        .nth(id.nth)
        .map(|(i, _)| i);
    same.or_else(|| {
        let renamed: Vec<&str> = ledger
            .iter()
            .filter(|e| e.change == SymbolChange::Renamed)
            .filter(|e| e.from.as_ref().is_some_and(|f| id.names.contains(f)))
            .map(|e| e.name.as_str())
            .collect();
        items.iter().position(|it| {
            it.path == id.path
                && it
                    .symbols
                    .iter()
                    .any(|s| renamed.contains(&s.name.as_str()))
        })
    })
}

pub(super) struct Place {
    sel: HunkId,
    /// where the selection sat in the reading order: the fallback when its
    /// hunk is gone, so the reviewer lands on what came next
    pos: usize,
    /// scroll and cursor, relative to the hunk
    scroll_by: i64,
    cursor_by: (i64, usize),
    focus: Pane,
    mode: ViewMode,
    comments_only: bool,
    show_all: bool,
    path_filter: Option<(String, PathGlobs)>,
    strategy: String,
    show_groups: bool,
    zoom: bool,
    /// a text search is carried by its pattern; a symbol search is tied to
    /// the tree under the old cursor and is not
    search: Option<String>,
    jumps: Vec<(HunkId, Cursor)>,
}

pub(super) fn place_of(app: &App) -> Place {
    let it = &app.items[app.sel];
    let top = it.new_range[0].saturating_sub(1) as i64;
    Place {
        sel: hunk_id(&app.items, app.sel),
        pos: app.view.iter().position(|&i| i == app.sel).unwrap_or(0),
        scroll_by: app.scroll as i64 - auto_scroll(it) as i64,
        cursor_by: (app.cursor.line as i64 - top, app.cursor.col),
        focus: app.focus,
        mode: app.mode,
        comments_only: app.comments_only,
        show_all: app.show_all,
        path_filter: app.path_filter.clone(),
        strategy: app.strategy.clone(),
        show_groups: app.show_groups,
        zoom: app.zoom,
        search: app
            .search
            .as_ref()
            .filter(|s| s.kind == SearchKind::Text)
            .map(|s| s.pattern.clone()),
        jumps: app
            .jumps
            .iter()
            .map(|(i, c)| (hunk_id(&app.items, *i), *c))
            .collect(),
    }
}

/// Put the reviewer back where `p` says, in the review `app` now holds.
pub(super) fn restore(app: &mut App, p: Place) {
    if p.strategy != app.strategy {
        let _ = run_strategy(app, &p.strategy);
    }
    if p.mode != app.mode {
        let led = app.symbol_ledger.clone();
        set_mode(app, p.mode, &led);
    }
    // a filter that would now leave nothing to review is dropped rather than
    // kept: an empty review is not a place
    if set_filters(app, p.comments_only, p.show_all, p.path_filter.clone()).is_err() {
        let _ = set_filters(app, false, true, None);
    }
    let to = find_hunk(&app.items, &app.symbol_ledger, &p.sel)
        .filter(|i| app.view.contains(i))
        .unwrap_or_else(|| app.view[p.pos.min(app.view.len() - 1)]);
    select(app, to);
    let it = &app.items[to];
    app.scroll = (auto_scroll(it) as i64 + p.scroll_by).clamp(0, u16::MAX as i64) as u16;
    if let Some((_, lines)) = app.sources.get(&it.path) {
        let top = it.new_range[0].saturating_sub(1) as i64;
        let line = (top + p.cursor_by.0).max(0) as usize;
        app.cursor = clamp_cursor(
            Cursor {
                line,
                col: p.cursor_by.1,
            },
            lines,
        );
        app.search = p.search.map(|pattern| Search {
            kind: SearchKind::Text,
            matches: text_matches(lines, &pattern),
            pattern,
            index: 0,
        });
    }
    app.jumps = p
        .jumps
        .into_iter()
        .filter_map(|(id, c)| Some((find_hunk(&app.items, &app.symbol_ledger, &id)?, c)))
        .collect();
    app.focus = if p.focus == Pane::Deps {
        Pane::Code
    } else {
        p.focus
    };
    app.show_groups = p.show_groups;
    app.zoom = p.zoom;
}
