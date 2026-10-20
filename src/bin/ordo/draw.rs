// -------------------------------------------------------------------- draw
use crate::clamp_cursor;
use crate::code_view::code_view;
use crate::code_view::LineMarks;
use crate::code_view::Theme;
use crate::code_view::GUTTER_W;
use crate::commands::comment_lines;
use crate::commands::reveal;
use crate::commands::Cmd;
use crate::commands::COMMANDS;
use crate::compute_view;
use crate::config::draw_config;
use crate::display_row_of;
use crate::display_rows;
use crate::highlight::LineSpans;
use crate::history::FILE_CHURN_WINDOW;
use crate::keys::chord_label;
use crate::keys::Action;
use crate::keys::Fold;
use crate::keys::Keymap;
use crate::keys::Pane;
use crate::keys::ViewMode;
use crate::last_line;
use crate::ledger_label;
use crate::marks::note_key;
use crate::marks::Delta;
use crate::popup_width;
use crate::prose;
use crate::select;
use crate::stack_pop_valid;
use crate::stack_push;
use crate::view_pos;
use crate::App;
use crate::Canvas;
use crate::Card;
use crate::DisplayRow;
use crate::Item;
use crate::Popup;
use crate::SearchKind;
use ordo::model::Level;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::collections::HashSet;

/// One line of the why pane's content, independent of rendering — shared by
/// `draw` and the edge actions (`preview_edge`/`jump_to_edge`) below so both
/// agree on what line `why_sel` is pointing at.
pub(super) enum WhyKind {
    Text,
    /// a `dep` line; `Some(idx)` is its target hunk's index into `app.items`,
    /// `None` when the referenced hunk isn't part of this review
    Edge(Option<usize>),
}

pub(super) struct WhyRow {
    /// what the row is, in the label column; empty on a continuation row
    pub(super) label: &'static str,
    pub(super) text: String,
    style: Style,
    pub(super) kind: WhyKind,
}

// Actionable dep lines get a distinct look: bold+underlined when the target
// resolves to a hunk in this review, dimmed when it doesn't — honest at a
// glance about there being nothing to jump to.
fn edge_style(target: Option<usize>, theme: &Theme) -> Style {
    if target.is_some() {
        Style::default()
            .fg(theme.fg)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(theme.dim)
    }
}

/// The why pane's facts column, in render order — the reviewer's annotations,
/// what the hunk does, what it needs and what needs it, then the rest; the
/// findings sit beside it (`finding_lines`). A pure function of `Item` (plus `view`, so
/// a dep line whose target is currently filtered out renders — and resolves
/// — the same as one that was never part of the review) so it doubles as the
/// source of truth for what `why_sel` is currently sitting on.
/// What the app knows about a hunk that the `Item` itself does not: how it
/// compares to the last run, where it sits in the order, and anything asked
/// for on demand. Bundled so `why_rows` keeps taking the hunk and its
/// annotations rather than a growing list of loose options.
#[derive(Default)]
pub(super) struct WhyContext<'a> {
    note: Option<&'a str>,
    /// the reviewer's line comments inside this hunk, already worded
    comments: &'a [String],
    out_of_order: &'a [String],
    delta: Option<&'a str>,
    cascade: Option<&'a str>,
    churn: Option<&'a str>,
    /// what holds this hunk together with its neighbours (see `scope_line`)
    scope: Option<&'a str>,
    /// dep targets `:only-wave` hides, each with its wave and reviewed state:
    /// still followable, since `gd` switches to their wave
    other_waves: &'a [(usize, String)],
    /// what this hunk's wave was asked (`Waves::asked`)
    wave_asked: Option<&'a str>,
}

pub(super) fn why_rows(it: &Item, view: &[usize], theme: &Theme, ctx: &WhyContext) -> Vec<WhyRow> {
    let mut rows = annotation_rows(ctx, theme);
    rows.extend(rationale_rows(it, theme));
    for needs in [true, false] {
        let mut side: Vec<WhyRow> = it
            .edges
            .iter()
            .filter(|e| e.dependency == needs)
            .map(|e| {
                let elsewhere = e
                    .target
                    .and_then(|t| ctx.other_waves.iter().find(|(i, _)| *i == t));
                let target = e.target.filter(|t| view.contains(t) || elsewhere.is_some());
                let text = match elsewhere {
                    Some((_, wave)) => format!("{} · {wave}", e.label),
                    None => e.label.clone(),
                };
                WhyRow {
                    label: "",
                    text,
                    style: edge_style(target, theme),
                    kind: WhyKind::Edge(target),
                }
            })
            .collect();
        if side.is_empty() {
            side.push(fact("", "nothing in this change".to_string(), theme.dim));
        }
        side[0].label = if needs { "needs" } else { "needed by" };
        rows.extend(side);
    }
    // Churn sits with coverage: both answer "how much weight does this hunk
    // deserve" rather than "what does it do".
    if let Some(c) = ctx.churn {
        rows.push(fact("history", c.to_string(), theme.mark));
    }
    rows.extend(signal_rows(it, theme));
    if let Some(s) = ctx.scope {
        rows.push(fact("scope", s.to_string(), theme.fg));
    }
    if let Some(w) = it.wave {
        let row = match ctx.wave_asked {
            Some(asked) => format!("wave {w} · asked: {asked}"),
            None => format!("wave {w}"),
        };
        rows.push(fact("wave", row, theme.accent));
    }
    rows
}

fn fact(label: &'static str, text: String, color: Color) -> WhyRow {
    WhyRow {
        label,
        text,
        style: Style::default().fg(color),
        kind: WhyKind::Text,
    }
}

// what the reviewer and the last run said about this hunk, ahead of what the
// engine derived
fn annotation_rows(ctx: &WhyContext, theme: &Theme) -> Vec<WhyRow> {
    let mut rows = vec![];
    // a review note leads: it is the reviewer's own words about this symbol,
    // and it outranks anything the engine derived
    if let Some(n) = ctx.note {
        rows.push(fact("note", n.to_string(), theme.warn));
    }
    for (i, c) in ctx.comments.iter().enumerate() {
        rows.push(fact(
            if i == 0 { "comment" } else { "" },
            c.clone(),
            theme.warn,
        ));
    }
    // approving a call before its callee is the one review-order mistake the
    // graph can actually prove
    if !ctx.out_of_order.is_empty() {
        rows.push(fact(
            "order",
            format!(
                "marked reviewed, but depends on unreviewed {}",
                ctx.out_of_order.join(", ")
            ),
            theme.warn,
        ));
    }
    if let Some(d) = ctx.delta {
        rows.push(fact("since", d.to_string(), theme.accent));
    }
    if let Some(c) = ctx.cascade {
        rows.push(fact("cascade", c.to_string(), theme.accent));
    }
    rows
}

// The engine joins independent clauses with "; " to fit one line, which is
// right for `hunks[].rationale` in the JSON and wrong here: a busy hunk
// arrived as one wrapped paragraph ("adds a, b, c, and 5 more; adds CFG (L30,
// L37, …), HOST (…), READY (…)") that has to be read word by word. One clause
// per row restores what the join flattened. The detail layer follows, as more
// of the same answer.
fn rationale_rows(it: &Item, theme: &Theme) -> Vec<WhyRow> {
    // The engine's terminal fallback rationale: it found nothing to say about
    // the hunk, so a "what: change" line says nothing either — leave it out.
    let clauses = it
        .rationale
        .split("; ")
        .filter(|_| it.rationale != "change");
    let mut rows: Vec<WhyRow> = clauses
        .map(|frag| fact("", frag.to_string(), theme.fg))
        .chain(it.details.iter().map(|d| fact("", d.clone(), theme.accent)))
        .collect();
    if let Some(first) = rows.first_mut() {
        first.label = "what";
    }
    rows
}

// coverage and the engine's notes
fn signal_rows(it: &Item, theme: &Theme) -> Vec<WhyRow> {
    let mut rows = vec![];
    // Coverage leads the derived rows: "nothing here ran" changes how the rest
    // of the hunk should be read.
    if let Some((run, total)) = it.executed {
        let cold = total - run;
        let text = if cold == 0 {
            format!("all {total} executable lines here are covered by tests")
        } else {
            format!("{cold} of {total} executable lines here are executed by no test")
        };
        rows.push(fact(
            "coverage",
            text,
            if cold == 0 {
                theme.reviewed
            } else {
                theme.warn
            },
        ));
    }
    if !it.notes.is_empty() {
        rows.push(fact("notes", it.notes.join("; "), theme.mark));
    }
    rows
}

/// The why pane's right column: every finding, the hardest-pressing first,
/// whoever found it — its level and name as chips, where it came from (and
/// where its level came from, when `rules.toml` moved it), then its message
/// beneath, wrapped to `width` under its own indent.
pub(super) fn finding_lines(it: &Item, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut found: Vec<&ordo::model::Finding> = it.findings.iter().collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.level));
    let mut lines = vec![];
    for f in found {
        let chip = |strong| {
            Style::default()
                .fg(theme.fg)
                .bg(theme.level_bg(f.level, strong))
                .add_modifier(Modifier::BOLD)
        };
        let origin = match f.original_level {
            Some(was) if f.level > was => format!(", raised from {} in rules.toml", was.as_str()),
            Some(was) => format!(", lowered from {} in rules.toml", was.as_str()),
            None => String::new(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", f.level.as_str()), chip(true)),
            Span::styled(format!(" {} ", f.name), chip(false)),
            Span::styled(
                format!("  {}{origin}", f.source.as_str()),
                Style::default().fg(theme.dim),
            ),
        ]));
        let room = width.saturating_sub(3);
        for ml in f.message.lines() {
            lines.extend(hang(ml, room, room).into_iter().map(|l| {
                Line::from(Span::styled(
                    format!("   {l}"),
                    Style::default().fg(theme.fg),
                ))
            }));
        }
    }
    lines
}

/// `text` broken at spaces into lines of at most `first` columns, then `rest`
/// for every line after it — the caller indents those, which `Wrap` cannot.
/// A word longer than the room keeps its own line and is clipped by the pane.
pub(super) fn hang(text: &str, first: usize, rest: usize) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    let mut cur = String::new();
    for word in text.split(' ') {
        let room = if lines.is_empty() { first } else { rest };
        let need = cur.chars().count() + 1 + word.chars().count();
        if !cur.is_empty() && need > room {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    lines.push(cur);
    lines
}

/// The hardest-pressing finding level on a hunk, if anything was found.
pub(super) fn top_level(it: &Item) -> Option<Level> {
    it.findings.iter().map(|f| f.level).max_by_key(|l| *l as u8)
}

// the dep-line target at `app.why_sel`, if the cursor is on one at all
fn edge_at_cursor(app: &App) -> Option<Option<usize>> {
    match why_content(app).get(app.why_sel)?.kind {
        WhyKind::Edge(target) => Some(target),
        WhyKind::Text => None,
    }
}

/// A syntax-highlighted excerpt of `lines[n0..=n1]` (1-based, inclusive) with a
/// line-number gutter, for the dep preview. Falls back to unstyled text for a
/// file with no grammar, the same way the code pane does.
pub(super) fn excerpt(
    lines: &[String],
    hl: Option<&Vec<LineSpans>>,
    n0: usize,
    n1: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let num = Style::default().fg(theme.dim);
    (n0..=n1)
        .filter_map(|ln| {
            let text = lines.get(ln - 1)?;
            let mut spans = vec![Span::styled(format!("{ln:>5} "), num)];
            match hl.and_then(|h| h.get(ln - 1)) {
                Some(segs) if !segs.is_empty() => spans.extend(
                    segs.iter()
                        .map(|(t, c)| Span::styled(t.clone(), Style::default().fg(*c))),
                ),
                _ => spans.push(Span::raw(text.clone())),
            }
            Some(Line::from(spans))
        })
        .collect()
}

/// `K`/`F12` while the why pane is focused: preview the dep line's target
/// hunk — its location, rationale, and a code excerpt — without leaving the
/// current hunk. No-op when `why_sel` isn't on a dep line. When the target
/// isn't part of this review, says so plainly rather than showing nothing.
pub(super) fn preview_edge(app: &mut App) {
    let Some(target) = edge_at_cursor(app) else {
        return;
    };
    let Some(idx) = target else {
        app.popup = Some(Popup::new(
            "dep",
            vec![
                prose("the referenced change isn't part of this review"),
                prose("(excluded by a glob, --only-comments, or a file not sent to ordo)"),
            ],
        ));
        return;
    };
    let t = &app.items[idx];
    let mut lines = vec![prose(format!("{}:L{}", t.path, t.new_range[0]))];
    if t.rationale != "change" {
        lines.push(prose(""));
        lines.push(Line::from(Span::styled(
            format!("reason: {}", t.rationale),
            Style::default().fg(app.theme.border_focus),
        )));
    }
    if let Some((_, nl)) = app.sources.get(&t.path) {
        let [n0, n1] = t.new_range;
        if n0 >= 1 && n0 <= nl.len() {
            lines.push(prose(""));
            lines.extend(excerpt(
                nl,
                app.highlights.get(&t.path),
                n0,
                n1.min(nl.len()),
                &app.theme,
            ));
        }
    }
    app.popup = Some(Popup::new("dep", lines));
}

/// `Enter`/`gd` (vim), `C-Enter` (vscode) while the why pane is focused:
/// select the dep line's target hunk and focus the code pane, pushing the
/// current position first so `JumpBack` can return. No-op — never a guess —
/// when `why_sel` isn't on a dep line, or its target isn't part of this
/// review; `preview_edge` (`K`/`F12`) is what explains why in that case.
/// The cards the canvas shows for one hunk: what it needs on the left, what
/// needs it on the right, each an edge whose target is a hunk in this review.
///
/// Rebuilt every frame rather than stored, so a reload or a filter change can
/// never leave the canvas showing a hunk that is no longer there.
fn cards_for(app: &App, anchor: usize) -> Vec<Card> {
    let Some(it) = app.items.get(anchor) else {
        return vec![];
    };
    let mut out: Vec<Card> = vec![];
    for side in [true, false] {
        for e in it.edges.iter().filter(|e| e.dependency == side) {
            let Some(idx) = e.target else { continue };
            if idx >= app.items.len() {
                continue;
            }
            let t = &app.items[idx];
            // basename, not the full path: a card is 42 columns wide and the
            // full path pushed the line number — the part a reviewer needs to
            // find it — off the end of the title
            let file = t.path.rsplit('/').next().unwrap_or(&t.path);
            out.push(Card {
                idx,
                needs: side,
                label: format!("{} · {file}:L{}", card_name(t), t.new_range[0]),
            });
        }
    }
    out
}

/// What to call a card: the symbol it defines, else the definition holding it,
/// else the file. A card is identified by what a reviewer would say out loud.
fn card_name(it: &Item) -> String {
    hunk_symbol(it).unwrap_or_else(|| it.path.clone())
}

/// The symbol a hunk is about: the one it defines, else the definition holding it.
fn hunk_symbol(it: &Item) -> Option<String> {
    it.symbols
        .first()
        .map(|s| s.name.clone())
        .or_else(|| it.enclosing.clone())
}

/// `gD`: open the canvas on the selected hunk. A hunk with no edges opens
/// nothing — an empty canvas says less than staying put does.
pub(super) fn open_deps(app: &mut App) {
    if cards_for(app, app.sel).is_empty() {
        return;
    }
    app.canvas = Some(Canvas {
        anchor: app.sel,
        sel: Some(0),
        zoomed: false,
        scroll: 0,
    });
    app.focus = Pane::Deps;
}

/// `4` again, or the zoom key, on the canvas: the selected card fills the
/// frame, or stops filling it. Which card, and whether a side card gets its
/// half, is the layout's call each frame — see `canvas_layout`. A zoomed card
/// shows the file around its hunk the way the code pane does: the hunk
/// tinted, the rest plain.
pub(super) fn zoom_card(app: &mut App) {
    if let Some(c) = app.canvas.as_mut() {
        c.zoomed = !c.zoomed;
    }
}

pub(super) fn close_deps(app: &mut App) {
    app.canvas = None;
    if app.focus == Pane::Deps {
        app.focus = Pane::Code;
    }
}

/// Enter on a card: go there, and close the canvas. The jump is pushed the
/// same way `gd` pushes it, so `C-o` returns — and returns to the hunk, not to
/// the canvas, which has served its purpose once a destination is chosen.
/// Enter on the anchor just closes: it is where the reader already was.
pub(super) fn jump_to_card(app: &mut App) {
    let Some(c) = app.canvas.as_ref() else { return };
    let cards = cards_for(app, c.anchor);
    let idx = match c.sel {
        Some(i) => match cards.get(i) {
            Some(card) => card.idx,
            None => return,
        },
        None => c.anchor,
    };
    stack_push(&mut app.jumps, (app.sel, app.cursor));
    close_deps(app);
    select(app, idx);
    app.focus = Pane::Code;
}

/// Move the card selection, clamped. `delta` is signed so one function serves
/// `j` and `k`. The anchor sits one step above the first card, so `k` from
/// there reaches it, `j` comes back, and `gg` lands on it.
pub(super) fn move_card(app: &mut App, delta: isize) {
    let Some(c) = app.canvas.as_ref() else { return };
    let n = cards_for(app, c.anchor).len();
    if n == 0 {
        return;
    }
    // -1 is the anchor
    let at = c.sel.map_or(-1, |i| i as isize);
    let next = at.saturating_add(delta).clamp(-1, n as isize - 1);
    if let Some(c) = app.canvas.as_mut() {
        c.sel = usize::try_from(next).ok();
        c.scroll = 0;
    }
}

/// The paging keys on the canvas: scroll the selected card (or the anchor)
/// through its file, from the top of its hunk to the file's last line.
pub(super) fn scroll_card(app: &mut App, by: isize) {
    let Some(c) = app.canvas.as_ref() else { return };
    let idx = match c.sel {
        None => c.anchor,
        Some(i) => match cards_for(app, c.anchor).get(i) {
            Some(card) => card.idx,
            None => return,
        },
    };
    let it = &app.items[idx];
    let len = app.sources.get(&it.path).map_or(0, |(_, new)| new.len());
    let room = len.saturating_sub(it.new_range[0]);
    if let Some(c) = app.canvas.as_mut() {
        c.scroll = (c.scroll as isize + by).clamp(0, room as isize) as usize;
    }
}

pub(super) fn jump_to_edge(app: &mut App) {
    let Some(Some(idx)) = edge_at_cursor(app) else {
        return;
    };
    let from = (app.sel, app.cursor);
    if !reveal(app, idx) {
        return;
    }
    stack_push(&mut app.jumps, from);
    select(app, idx);
    app.focus = Pane::Code;
}

/// `C-o` (vim) / `Alt-Left` (vscode): pop the position stack and return
/// there — skipping (not just the out-of-range entries `stack_pop_valid`
/// already drops, but also) any entry a live filter currently hides, the
/// same "not part of this review" treatment `jump_to_edge` gives it. A
/// no-op once the stack is exhausted.
pub(super) fn jump_back(app: &mut App) {
    let (idx, cursor) = loop {
        match stack_pop_valid(&mut app.jumps, app.items.len()) {
            // one `gd` into another wave switched to it; coming back switches back
            Some((idx, cursor)) if reveal(app, idx) => break (idx, cursor),
            Some(_) => continue, // hidden by a live filter — try the next one
            None => return,
        }
    };
    select(app, idx);
    if let Some((_, nl)) = app.sources.get(&app.items[idx].path) {
        if !nl.is_empty() {
            app.cursor = clamp_cursor(cursor, nl);
        }
    }
    app.focus = Pane::Code;
}

/// What the footer offers on each pane: the few actions it is most likely to
/// want next. Each is labelled by whatever key the active keymap binds to it,
/// so a preset or a remap is what the footer says.
type FooterKeys = &'static [(&'static [Action], &'static str)];
const LIST_KEYS: FooterKeys = &[
    (&[Action::Next, Action::Prev], "move"),
    (&[Action::ToggleReviewed], "reviewed"),
    (&[Action::OpenDeps], "dependencies"),
    (&[Action::Fold(Fold::Toggle)], "fold"),
    (&[Action::CommandOpen], "command"),
    (&[Action::FocusNext], "pane"),
];
const CODE_KEYS: FooterKeys = &[
    (&[Action::Next, Action::Prev], "line"),
    (&[Action::Hover], "symbol"),
    (&[Action::SearchOpen], "search"),
    (&[Action::MarkNext, Action::MarkPrev], "uses"),
    (&[Action::Comment], "comment"),
    (&[Action::OpenEditor], "edit"),
    (&[Action::FocusNext], "pane"),
];
const WHY_KEYS: FooterKeys = &[
    (&[Action::Next, Action::Prev], "move"),
    (&[Action::JumpToEdge], "go to dep"),
    (&[Action::Hover], "preview dep"),
    (&[Action::JumpBack], "back"),
    (&[Action::FocusNext], "pane"),
];
const DEPS_KEYS: FooterKeys = &[
    (&[Action::Next, Action::Prev], "card"),
    (&[Action::JumpToEdge], "jump there"),
    (&[Action::Focus(Pane::Deps)], "zoom card"),
    (&[Action::Quit], "close"),
];

/// The first chord the keymap binds to `a`, as the help popup spells it.
pub(super) fn bound_key(keys: &Keymap, a: Action) -> Option<String> {
    keys.binds
        .iter()
        .find(|(_, _, b)| *b == a)
        .map(|&(p, k, _)| chord_label(p, k))
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// The bottom row: the focused pane's name, then its most relevant keys, as
/// many as fit, and always the key for all of them.
pub(super) fn footer_spans(app: &App, width: usize) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let mut out = vec![];
    // the review no longer matching the tree outranks everything else on the
    // line: it is the one thing here that is not about the review itself
    if let Some(stale) = &app.stale {
        out.push(Span::styled(
            format!(" {} ·", stale.line()),
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        ));
    } else if let Some(n) = &app.notice {
        out.push(Span::styled(
            format!(" {n} ·"),
            Style::default().fg(theme.accent),
        ));
    }
    let (name, keys) = match app.focus {
        Pane::List => ("list", LIST_KEYS),
        Pane::Code => ("code", CODE_KEYS),
        Pane::Why => ("why", WHY_KEYS),
        Pane::Deps => ("deps", DEPS_KEYS),
    };
    out.push(Span::styled(
        format!(" {name} "),
        Style::default()
            .fg(theme.border_focus)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD),
    ));
    out.push(Span::raw(" "));
    let entry = |label: String, desc: &str| {
        vec![
            Span::styled(
                format!(" {label} "),
                Style::default()
                    .fg(theme.fg)
                    .bg(theme.band_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {desc}  "), Style::default().fg(theme.dim)),
        ]
    };
    let mut entries = vec![];
    // say how to get out of a zoom, since the other panes are not on screen
    if app.zoom {
        entries.extend(bound_key(&app.keys, Action::Zoom).map(|k| entry(k, "restore")));
    }
    for (actions, desc) in keys {
        let labels: Option<Vec<String>> =
            actions.iter().map(|&a| bound_key(&app.keys, a)).collect();
        entries.extend(labels.map(|l| entry(l.join("/"), desc)));
    }
    let help = bound_key(&app.keys, Action::Help)
        .map(|k| entry(k, "all keys"))
        .unwrap_or_default();
    let room = width.saturating_sub(spans_width(&help));
    for e in entries {
        if spans_width(&out) + spans_width(&e) > room {
            break;
        }
        out.extend(e);
    }
    out.extend(help);
    out
}

/// Record the pane heights and the code pane's width on `app`. Called by the
/// renderer and by the event loop before a key is handled, so the two can never
/// disagree about how tall a page is.
pub(super) fn set_geometry(app: &mut App, area: Rect) {
    if area.width < MIN_COLS || area.height < MIN_ROWS {
        return;
    }
    let body = body_of(area);
    let rows = why_content(app).len();
    let findings = finding_lines(&app.items[app.sel], &app.theme, usize::MAX).len();
    let panes = pane_rects(body, app.focus, app.zoom, rows.max(findings));
    record_geometry(app, &panes, body);
}

/// Copy a computed layout onto `app`. A hidden pane records the height it would
/// have if the next keypress brought it back, so paging never runs against a
/// zero — and `body` is not a guess there: it is exactly the rect that pane
/// gets when it is zoomed to.
fn record_geometry(app: &mut App, panes: &Panes, body: Rect) {
    let code = panes.code.unwrap_or(body);
    // every pane spends one row on its header strip and nothing on a border
    app.code_height = code.height.saturating_sub(1);
    app.code_width = (code.width as usize)
        .saturating_sub(GUTTER_W)
        .min(u16::MAX as usize) as u16;
    app.why_height = panes.why.unwrap_or(body).height.saturating_sub(1);
}

/// The narrowest a card may be, and how far each later card steps outward from
/// the centre. The step is what makes the two sides read as a fan rather than
/// as two columns.
pub(super) const CARD_W_MIN: u16 = 42;
const CARD_STEP: u16 = 3;
/// The shortest a card may be: a border plus two rows of code.
pub(super) const CARD_H_MIN: u16 = 4;
/// The shortest the anchor may be. A one-line hunk gets a border and one blank
/// row rather than a three-row sliver; past that its own height decides.
const ANCHOR_H_MIN: u16 = CARD_H_MIN;
/// A card shows its definition's extent, clamped — past this nobody reads it
/// in a glance, which is the whole point of the canvas.
pub(super) const CARD_ROWS: usize = 12;

/// Where every piece of the canvas goes.
///
/// The anchor box sits at top centre. Cards this hunk NEEDS fan down-left,
/// cards that NEED IT fan down-right, and neither side crosses the centre.
/// When one side is empty it yields its width and the other centres, because a
/// half-empty split reads worse than no split at all.
pub(super) struct CanvasLayout {
    pub(super) area: Rect,
    pub(super) anchor: Rect,
    /// one rect per card, parallel to the `Card` list it was built from; an
    /// empty rect is a card the zoom hides
    pub(super) cards: Vec<Rect>,
    /// false when the frame is too narrow to fan, so the cards stack instead
    pub(super) fanned: bool,
    /// whether a zoom took effect this frame
    pub(super) zoomed: bool,
}

/// What fills the frame: nothing, the anchor, or one card by its index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Zoom {
    Off,
    Anchor,
    Card(usize),
}

pub(super) fn canvas_layout(
    body: Rect,
    cards: &[Card],
    anchor_idx: usize,
    extent: &dyn Fn(usize) -> u16,
    zoom: Zoom,
) -> CanvasLayout {
    let mut l = fan_layout(body, cards, anchor_idx, extent);
    // A zoomed side card keeps its half and takes all of it, top to bottom;
    // the anchor moves across to the other half so the two still read
    // together. Below the split threshold there is no half to take, so a
    // side card's zoom does nothing — the stacked fallback stays.
    let target = match zoom {
        Zoom::Off => return l,
        Zoom::Anchor => None,
        Zoom::Card(i) if l.fanned && i < cards.len() => Some(i),
        Zoom::Card(_) => return l,
    };
    let inner_w = l.area.width.saturating_sub(2);
    let full_h = l.area.height.saturating_sub(2);
    let half = l.area.width.saturating_sub(4) / 2;
    let centre = l.area.x + l.area.width / 2;
    let Some(i) = target else {
        l.anchor = Rect {
            x: l.area.x + 1,
            y: l.area.y + 1,
            width: inner_w,
            height: full_h,
        };
        l.cards = vec![Rect::default(); cards.len()];
        l.zoomed = true;
        return l;
    };
    let needs = cards[i].needs;
    // the left half runs from the inner edge to the centre; the right half
    // from past the centre to the inner edge, one column short of `half`
    // when the width is even
    let left = (l.area.x + 1, half.min(inner_w));
    let right_x = centre + 2;
    let right = (
        right_x,
        half.min((l.area.x + l.area.width).saturating_sub(right_x + 1)),
    );
    let (own, other) = if needs { (left, right) } else { (right, left) };
    (l.anchor.x, l.anchor.width) = other;
    for (j, (c, r)) in cards.iter().zip(l.cards.iter_mut()).enumerate() {
        if j == i {
            *r = Rect {
                x: own.0,
                y: l.area.y + 1,
                width: own.1,
                height: full_h,
            };
        } else if c.needs == needs {
            *r = Rect::default();
        }
    }
    l.zoomed = true;
    l
}

/// The plain fan: the anchor across the top, the cards below it on their sides.
fn fan_layout(
    body: Rect,
    cards: &[Card],
    anchor_idx: usize,
    extent: &dyn Fn(usize) -> u16,
) -> CanvasLayout {
    // inset from the frame so the canvas reads as floating over the panes
    let area = Rect {
        x: body.x + 1,
        y: body.y + 1,
        width: body.width.saturating_sub(2),
        height: body.height.saturating_sub(2),
    };
    let inner_w = area.width.saturating_sub(2);
    // The anchor is the hunk being read: it takes the full width it is given
    // and as much height as the hunk needs, rather than a fixed five rows that
    // showed the first three lines of everything.
    let anchor_h = (extent(anchor_idx) + 2).clamp(ANCHOR_H_MIN, max_anchor_h(area.height));
    let anchor = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: inner_w,
        height: anchor_h,
    };
    // Below the split threshold each half would be under 48 columns, too
    // narrow for code: stack instead, same fallback a zoomed pane gets.
    let fanned = area.width >= SPLIT_COLS;
    let top = anchor.y + anchor.height + 1; // the rule under the anchor
    let bottom = area.y + area.height.saturating_sub(1);
    let avail_h = bottom.saturating_sub(top);
    let mut rects = Vec::with_capacity(cards.len());
    if !fanned {
        let h = stacked_height(avail_h, cards.len() as u16);
        for (i, c) in cards.iter().enumerate() {
            rects.push(Rect {
                x: area.x + 1,
                y: top + (i as u16) * h,
                width: inner_w,
                height: h.min(extent(c.idx).min(CARD_ROWS as u16) + 2),
            });
        }
        return CanvasLayout {
            area,
            anchor,
            cards: rects,
            fanned,
            zoomed: false,
        };
    }
    // Each direction owns its half and keeps it. An empty side used to yield
    // its width so the other could centre; that made a one-directional hunk
    // look like a different view rather than the same one with nothing on the
    // left, and left the reader unsure which side they were looking at.
    let centre = area.x + area.width / 2;
    let n_left = cards.iter().filter(|c| c.needs).count() as u16;
    let n_right = cards.len() as u16 - n_left;
    let half = area.width.saturating_sub(4) / 2;
    let card_w = |n: u16| -> u16 {
        let steps = n.saturating_sub(1) * CARD_STEP;
        half.saturating_sub(steps).max(CARD_W_MIN)
    };
    let (lw, rw) = (card_w(n_left), card_w(n_right));
    let (lh, rh) = (
        stacked_height(avail_h, n_left),
        stacked_height(avail_h, n_right),
    );
    let (mut li, mut ri) = (0u16, 0u16);
    for c in cards {
        let (i, x, w, h) = if c.needs {
            let i = li;
            li += 1;
            let off = lw + 2 + i * CARD_STEP;
            (i, centre.saturating_sub(off).max(area.x + 1), lw, lh)
        } else {
            let i = ri;
            ri += 1;
            (
                i,
                (centre + 2 + i * CARD_STEP).min(area.x + area.width - rw - 1),
                rw,
                rh,
            )
        };
        rects.push(Rect {
            x,
            y: top + i * h,
            width: w.min(inner_w),
            // a card never grows past the hunk it shows — empty rows under two
            // lines of code read as a rendering fault — nor past CARD_ROWS,
            // which is as much as anyone takes in at a glance
            height: h.min(extent(c.idx).min(CARD_ROWS as u16) + 2),
        });
    }
    CanvasLayout {
        area,
        anchor,
        cards: rects,
        fanned,
        zoomed: false,
    }
}

/// How tall each of `n` stacked cards may be in `avail` rows.
///
/// They share the space rather than taking a fixed slice of it: one card on a
/// side gets the whole column, four get a quarter each.
fn stacked_height(avail: u16, n: u16) -> u16 {
    if n == 0 {
        return CARD_H_MIN;
    }
    (avail / n).max(CARD_H_MIN)
}

/// The anchor may take at most half the canvas, however long the hunk is —
/// past that there is no room left for the cards it exists to relate to.
pub(super) fn max_anchor_h(area_h: u16) -> u16 {
    (area_h.saturating_sub(2) / 2).max(ANCHOR_H_MIN)
}

/// A rect of at most `w` x `h`, centred in `body`.
pub(super) fn centred(body: Rect, w: u16, h: u16) -> Rect {
    let width = w.min(body.width.saturating_sub(2));
    let height = h.min(body.height.saturating_sub(2));
    Rect {
        x: body.x + (body.width.saturating_sub(width)) / 2,
        y: body.y + (body.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Render the dependency canvas over the panes.
fn draw_canvas(f: &mut Frame, app: &App, body: Rect) {
    let Some(c) = app.canvas.as_ref() else { return };
    let cards = cards_for(app, c.anchor);
    if cards.is_empty() {
        return;
    }
    // how many rows a hunk actually occupies. Uncapped: the anchor is meant to
    // cover its whole hunk, and `canvas_layout` caps the *cards* at CARD_ROWS
    let extent = |idx: usize| -> u16 {
        let it = &app.items[idx];
        let n = it.new_range[1].saturating_sub(it.new_range[0]) + 1;
        n.min(u16::MAX as usize) as u16
    };
    let zoom = match (c.zoomed, c.sel) {
        (false, _) => Zoom::Off,
        (true, None) => Zoom::Anchor,
        (true, Some(i)) => Zoom::Card(i),
    };
    let l = canvas_layout(body, &cards, c.anchor, &extent, zoom);
    let theme = &app.theme;
    // cards that will not fit are counted, not silently dropped; the ones a
    // zoom hides on purpose have no rect and are not
    let hidden = l
        .cards
        .iter()
        .filter(|r| r.height > 0 && r.y + r.height >= l.area.y + l.area.height)
        .count();
    f.render_widget(Clear, l.area);
    let anchor_it = &app.items[c.anchor];
    let zoomed = if l.zoomed {
        " · zoomed (same key restores)"
    } else {
        ""
    };
    let unshown = if hidden > 0 {
        format!(" · {hidden} not shown")
    } else {
        String::new()
    };
    let title = format!(
        " 4 deps — {} · {} needs, {} needed by{zoomed}{unshown} ",
        card_name(anchor_it),
        cards.iter().filter(|c| c.needs).count(),
        cards.iter().filter(|c| !c.needs).count(),
    );
    f.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focus))
            .title(Span::styled(title, Style::default().fg(theme.border_focus))),
        l.area,
    );

    // the anchor: the hunk every card is relative to
    card_widget(
        f,
        app,
        l.anchor,
        c.anchor,
        &card_name(anchor_it),
        c.sel.is_none(),
    );

    if l.fanned && !l.zoomed {
        // the rule under the anchor, labelling which way each side runs
        let rule_y = l.anchor.y + l.anchor.height;
        let rule = Rect {
            x: l.area.x + 1,
            y: rule_y,
            width: l.area.width.saturating_sub(2),
            height: 1,
        };
        let w = rule.width as usize;
        let any_needs = cards.iter().any(|c| c.needs);
        let any_needed = cards.iter().any(|c| !c.needs);
        // a side with nothing on it is not advertised — an arrow pointing at
        // empty space reads as a missing card rather than as an absent one
        let left = if any_needs { "◀── needs" } else { "" };
        let right = if any_needed {
            "needed by ──▶"
        } else {
            ""
        };
        let mut line = String::from(left);
        let pad = w.saturating_sub(left.chars().count() + right.chars().count());
        line.push_str(&"─".repeat(pad));
        line.push_str(right);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                line,
                Style::default().fg(theme.dim),
            ))),
            rule,
        );
    }

    for (i, (card, rect)) in cards.iter().zip(l.cards.iter()).enumerate() {
        if rect.height == 0 || rect.y + rect.height >= l.area.y + l.area.height {
            continue; // hidden by the zoom, or would overrun the frame's own border
        }
        card_widget(f, app, *rect, card.idx, &card.label, c.sel == Some(i));
    }
}

/// One card: a header band, then a few rows of the target hunk rendered by the
/// same `code_view` the code pane uses, so syntax and diff tint are identical.
/// The band carries the hunk's top finding level the way its list row does,
/// in the strong shade when selected; the last row is left blank so stacked
/// cards stay apart. The selected card starts `Canvas::scroll` rows further
/// down.
fn card_widget(f: &mut Frame, app: &App, rect: Rect, idx: usize, label: &str, selected: bool) {
    if rect.width < 8 || rect.height < 3 {
        return;
    }
    let theme = &app.theme;
    let it = &app.items[idx];
    let bg = match (top_level(it), selected) {
        (Some(l), s) => theme.level_bg(l, s),
        (None, true) => theme.select_bg,
        (None, false) => theme.band_bg,
    };
    let rows = rect.height.saturating_sub(2) as usize;
    // start at the hunk, not at the top of its file: `code_view`'s `start` is
    // an offset into the whole file, and passing 0 showed every card the
    // module docstring instead of the code the card is about
    let scrolled = if selected {
        app.canvas.as_ref().map_or(0, |c| c.scroll)
    } else {
        0
    };
    let start = it.new_range[0].saturating_sub(1) + scrolled;
    let (lines, _, _) = code_view(
        it,
        &app.sources,
        &app.highlights,
        rect.width as usize,
        0,
        None,
        &[],
        None,
        theme,
        start,
        rows,
        &LineMarks::default(),
        // the band already carries the card's level, and a card is sized to
        // its hunk's source lines
        &[],
    );
    let head = Line::from(vec![
        Span::styled(
            format!(" {label}"),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}", it.rationale), Style::default().fg(theme.fg)),
    ]);
    f.render_widget(
        Paragraph::new(head).style(Style::default().bg(bg)),
        Rect { height: 1, ..rect },
    );
    f.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect {
            y: rect.y + 1,
            height: rows as u16,
            ..rect
        },
    );
}

/// Where each pane sits this frame. Pure in the terminal size, the focus and
/// the zoom flag, so the event loop can ask the same question the renderer
/// does instead of reading what the last frame happened to leave behind.
pub(super) struct Panes {
    pub(super) list: Option<Rect>,
    pub(super) code: Option<Rect>,
    pub(super) why: Option<Rect>,
}

/// `why_rows` is how many rows the why pane's taller column holds.
pub(super) fn pane_rects(body: Rect, focus: Pane, zoom: bool, why_rows: usize) -> Panes {
    if zoom {
        let (list, code, why) = match focus {
            Pane::List => (Some(body), None, None),
            // the canvas floats over whatever is underneath, so while it has
            // focus the panes keep the layout they had
            Pane::Code | Pane::Deps => (None, Some(body), None),
            Pane::Why => (None, None, Some(body)),
        };
        return Panes { list, code, why };
    }
    // content height plus its header, floored at a quarter of the column so
    // the two columns side by side have room to say something, capped so a
    // long list of findings cannot crowd out the code
    let floor = (body.height / 4).max(WHY_MIN_H);
    let why_h = (why_rows.min(u16::MAX as usize) as u16)
        .saturating_add(1)
        .clamp(floor, (body.height * 2 / 5).max(floor));
    // Below `SPLIT_COLS` side by side starves both columns, so the panes stack
    // full width instead: the list gets a third, enough to see where the
    // selection sits in the order, and the code what the why pane leaves.
    if body.width < SPLIT_COLS {
        let [list, code, why] = Layout::vertical([
            Constraint::Length((body.height / 3).max(LIST_MIN_H)),
            Constraint::Min(3),
            Constraint::Length(why_h),
        ])
        .areas(body);
        return Panes {
            list: Some(list),
            code: Some(code),
            why: Some(why),
        };
    }
    // no borders: a one-column gutter is what keeps the list off the code
    let cols = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
        .spacing(1)
        .split(body);
    let rhs = Layout::vertical([Constraint::Min(3), Constraint::Length(why_h)]).split(cols[1]);
    Panes {
        list: Some(cols[0]),
        code: Some(rhs[0]),
        why: Some(rhs[1]),
    }
}

/// The why pane's least height: its header and a few rows of facts.
const WHY_MIN_H: u16 = 5;
/// The stacked list's least height: its header and a few rows around the
/// selection.
const LIST_MIN_H: u16 = 4;

/// The frame's rows: the top bar, a blank row under it, the panes, the footer.
fn frame_rows(area: Rect) -> [Rect; 4] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// The body rect — the panes, between the top bar and the footer.
fn body_of(area: Rect) -> Rect {
    frame_rows(area)[2]
}

/// Below this the three panes side by side each get too little width to be
/// read, so they stack instead. Measured, not guessed: at 80 columns a 38%
/// list pane is 30 wide and loses the line number off `gate.py:L117`.
pub(super) const SPLIT_COLS: u16 = 96;

/// Below this nothing can be laid out honestly, so say so rather than draw a
/// frame of truncated stubs.
const MIN_COLS: u16 = 56;
const MIN_ROWS: u16 = 12;

/// What a terminal too small for any layout gets: the requirement, what it
/// currently is, and nothing else.
fn draw_too_small(f: &mut Frame, area: Rect, theme: &Theme) {
    let lines = vec![
        Line::from(Span::styled(
            format!("ordo needs {MIN_COLS}×{MIN_ROWS}"),
            Style::default().fg(theme.fg),
        )),
        Line::from(Span::styled(
            format!("this terminal is {}×{}", area.width, area.height),
            Style::default().fg(theme.dim),
        )),
    ];
    let top = area.height.saturating_sub(lines.len() as u16) / 2;
    let rect = Rect {
        x: area.x,
        y: area.y + top,
        width: area.width,
        height: lines.len() as u16,
    };
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        rect,
    );
}

/// A pane's header strip, where a border and its title used to be: the pane's
/// digit and name as a chip, then what it is showing, on the band tint.
///
/// Borders went because they were the most-painted colour on screen while
/// saying only "this is a pane"; the band says that in one row.
/// The focused pane's chip is inverted in the focus colour — the one loud piece
/// of chrome, because it says where the keys will land.
fn pane_header(
    n: u8,
    name: &str,
    rest: Vec<Span<'static>>,
    focused: bool,
    theme: &Theme,
) -> Paragraph<'static> {
    let chip = if focused {
        Style::default()
            .fg(theme.border_focus)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default().fg(theme.dim)
    };
    let mut spans = vec![Span::styled(format!(" {n} {name} "), chip)];
    spans.extend(rest);
    Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.band_bg))
}

/// A pane's rect, split into its header row and its content.
fn split_header(r: Rect) -> (Rect, Rect) {
    let head = Rect {
        height: 1.min(r.height),
        ..r
    };
    let body = Rect {
        y: r.y + head.height,
        height: r.height - head.height,
        ..r
    };
    (head, body)
}

/// `s` cut to `n` characters, an ellipsis marking the cut.
pub(super) fn fit(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    if n == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(n - 1).collect();
    out.push('…');
    out
}

/// `fit`, keeping the end: a path's file name says more than its first folder.
fn fit_tail(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        return s.to_string();
    }
    if n == 0 {
        return String::new();
    }
    let tail: String = s.chars().skip(len - (n - 1)).collect();
    format!("…{tail}")
}

/// The top line: what is being reviewed on the left; on the right how far the
/// review has got — hunks reviewed as a bar whose filled part is the reviewed
/// tint, edges when there are any, and a chip per finding level present — and
/// the key preset.
pub(super) fn top_bar(app: &App, rev: &str, width: usize) -> Line<'static> {
    let t = &app.theme;
    let band = Style::default().fg(t.fg).bg(t.band_bg);
    let dim = Style::default().fg(t.dim);
    let mut left = vec![
        Span::styled(
            " ordo ",
            Style::default()
                .fg(t.fg)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(format!(" {rev} "), band.add_modifier(Modifier::BOLD)),
    ];
    let (done, total, edges_done, edges_total) = coverage(app);
    let mut right = vec![];
    if let Some(w) = app.only_wave {
        right.push(Span::styled(format!("wave {w}  "), dim));
    }
    if app.view.len() < app.items.len() {
        right.push(Span::styled("filtered  ", dim));
    }
    let progress = format!(" {done}/{total} reviewed ");
    let fill = (progress.len() * done)
        .div_ceil(total.max(1))
        .min(progress.len());
    right.push(Span::styled(
        progress[..fill].to_string(),
        band.bg(t.reviewed_bg),
    ));
    right.push(Span::styled(progress[fill..].to_string(), band));
    // edge coverage is the number that tracks understanding; it is omitted
    // when the review has no def→use links to cover
    if edges_total > 0 {
        right.push(Span::raw("  "));
        right.push(Span::styled(
            format!(" {edges_done}/{edges_total} edges "),
            band,
        ));
    }
    for level in [Level::Verdict, Level::Warn, Level::Note] {
        let n = app
            .view
            .iter()
            .flat_map(|&i| &app.items[i].findings)
            .filter(|f| f.level == level)
            .count();
        if n > 0 {
            right.push(Span::raw(" "));
            right.push(Span::styled(
                format!(" {n} {} ", level.as_str()),
                Style::default()
                    .fg(t.fg)
                    .bg(t.level_bg(level, true))
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    right.push(Span::styled(format!("  {} ", app.keys.name), dim));
    let used = spans_width(&left) + spans_width(&right);
    if let Some(subject) = &app.subject {
        // only when a useful part of it fits: a dozen characters of a subject
        // read as noise
        let room = width.saturating_sub(used + 4);
        if room >= 12 {
            left.push(Span::styled(format!("  {}", fit(subject, room)), dim));
        }
    }
    let pad = width.saturating_sub(spans_width(&left) + spans_width(&right));
    left.push(Span::raw(" ".repeat(pad)));
    left.extend(right);
    Line::from(left)
}

pub(super) fn draw(f: &mut Frame, app: &mut App, rev: &str) {
    let area = f.area();
    if area.width < MIN_COLS || area.height < MIN_ROWS {
        draw_too_small(f, area, &app.theme);
        return;
    }
    // the review's state across the top; one row at the bottom for the keys,
    // which used to ride in the code pane's border title and were truncated
    // mid-word there
    let [top, _, body, footer_area] = frame_rows(area);
    // a side card's zoom takes a half the canvas only has once it fans, so
    // below that an explicit toggle would sit invisible and surprise the
    // reviewer when the terminal widens; the canvas is inset by two, so it
    // fans two columns later than the panes split
    if body.width < SPLIT_COLS + 2 {
        if let Some(c) = app.canvas.as_mut().filter(|c| c.sel.is_some()) {
            c.zoomed = false;
        }
    }
    // built before the layout, because the why pane is sized to it: it used to
    // take a flat 30% and sat nearly empty on a one-line rationale
    let why_content = why_content(app);
    let findings = finding_lines(&app.items[app.sel], &app.theme, usize::MAX);
    let why_len = why_content.len().max(findings.len());
    let panes = pane_rects(body, app.focus, app.zoom, why_len);
    // Record what the panes came out as, rather than deriving it a second
    // time. `set_geometry` answers the same question for the event loop from
    // the same `pane_rects`, but has to rebuild the why rows to do it; here
    // they are already built.
    record_geometry(app, &panes, body);

    f.render_widget(Paragraph::new(top_bar(app, rev, top.width as usize)), top);
    draw_list(f, app, panes.list);
    // right — code (top) + why (bottom); either may be absent while zoomed.
    // A hidden pane falls back to `body`, which is not a guess: that is exactly
    // the rect it gets when the next keypress zooms to it.
    draw_code(f, app, panes.code, body);
    // content, not geometry: the why pane's length only exists once its rows
    // are built; the pane size it clamps against was set by `record_geometry`
    app.why_len = why_len;
    app.why_scroll = app.why_scroll.min(last_line(app.why_len));
    app.why_sel = app.why_sel.min(last_line(app.why_len) as usize);
    if let Some(r) = panes.why {
        draw_why(f, app, r, &why_content, !findings.is_empty());
    }
    f.render_widget(
        Paragraph::new(Line::from(footer_spans(app, footer_area.width as usize))),
        footer_area,
    );

    draw_canvas(f, app, body);
    draw_config(f, app, body);
    draw_popup(f, app, body);
    draw_command_bar(f, app);
}

/// The why pane's rows for the selected hunk.
pub(super) fn why_content(app: &App) -> Vec<WhyRow> {
    why_rows(
        &app.items[app.sel],
        &app.view,
        &app.theme,
        &WhyContext {
            note: note_for(app, app.sel),
            comments: &comments_in(app, app.sel),
            out_of_order: &out_of_order_labels(app, app.sel),
            delta: delta_line(app, app.sel),
            cascade: cascade_line(app, app.sel).as_deref(),
            churn: churn_line(app, app.sel).as_deref(),
            scope: scope_line(app, app.sel).as_deref(),
            other_waves: &other_waves(app, app.sel),
            wave_asked: app.items[app.sel].wave.and_then(|w| app.waves.asked(w)),
        },
    )
}

/// The dep targets of item `i` that only `:only-wave` hides, labelled with
/// their wave and whether they are reviewed.
fn other_waves(app: &App, i: usize) -> Vec<(usize, String)> {
    if app.only_wave.is_none() {
        return vec![];
    }
    let glob = app.path_filter.as_ref().map(|(_, g)| g);
    let reachable = compute_view(&app.items, app.comments_only, app.show_all, glob, None);
    app.items[i]
        .edges
        .iter()
        .filter_map(|e| e.target)
        .filter(|t| !app.view.contains(t) && reachable.contains(t))
        .map(|t| {
            let wave = match app.items[t].wave {
                Some(w) => format!("wave {w}"),
                None => "before the waves".to_string(),
            };
            let mark = if app.reviewed[t] { ", reviewed" } else { "" };
            (t, format!("{wave}{mark}"))
        })
        .collect()
}

/// How wide the list's symbol column is: room for most names, cut past that.
const SYMBOL_W: usize = 10;

// left — reading order
fn draw_list(f: &mut Frame, app: &App, list_area: Option<Rect>) {
    let Some(area) = list_area else { return };
    let theme = &app.theme;
    let (head, rows_area) = split_header(area);
    let mode = match app.mode {
        ViewMode::Ledger => " by symbol",
        ViewMode::Hunks => " by hunk",
    };
    f.render_widget(
        pane_header(
            1,
            "list",
            vec![Span::styled(mode, Style::default().fg(theme.dim))],
            app.focus == Pane::List,
            theme,
        ),
        head,
    );
    let w = rows_area.width as usize;
    let display = display_rows(
        &app.view,
        &app.items,
        &app.groups,
        app.show_groups,
        &app.collapsed,
    );
    let sel_row = display_row_of(&display, app.sel);
    let symbol = |i: usize| {
        app.groups
            .get(&app.items[i].bucket)
            .map_or("", String::as_str)
    };
    let rows: Vec<ListItem> = display
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let selected = r == sel_row;
            ListItem::new(match row {
                DisplayRow::File(path, n) => file_band(path, *n, w, theme),
                DisplayRow::Gap => Line::default(),
                DisplayRow::Item(i, opens) => {
                    let sym = if *opens { symbol(*i) } else { "" };
                    hunk_row(
                        app,
                        &[*i],
                        sym,
                        &app.items[*i].rationale,
                        false,
                        selected,
                        w,
                    )
                }
                DisplayRow::Folded(hidden) => {
                    let n = hidden.len();
                    let text = format!("{n} hunk{}, folded", if n == 1 { "" } else { "s" });
                    hunk_row(app, hidden, symbol(hidden[0]), &text, true, selected, w)
                }
            })
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(sel_row));
    f.render_stateful_widget(List::new(rows), rows_area, &mut state);
}

/// A file's band in the list: its path, and how many hunks run under it.
fn file_band(path: &str, n: usize, w: usize, theme: &Theme) -> Line<'static> {
    let band = Style::default().bg(theme.band_bg);
    let count = format!("{n} hunk{} ", if n == 1 { "" } else { "s" });
    let path = fit_tail(path, w.saturating_sub(count.chars().count() + 2));
    let pad = w.saturating_sub(path.chars().count() + 1 + count.chars().count());
    Line::from(vec![
        Span::styled(
            format!(" {path}"),
            band.fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(pad), band),
        Span::styled(count, band.fg(theme.dim)),
    ])
}

/// A list row's background: its state, in the strong shade when selected.
/// Reviewed outranks the findings, which the line-number cell keeps instead;
/// a state with no shade of its own is selected in the plain selection tint.
fn row_bg(theme: &Theme, level: Option<Level>, reviewed: bool, selected: bool) -> Color {
    match (reviewed, level) {
        (false, Some(l)) => theme.level_bg(l, selected),
        _ if selected => theme.select_bg,
        (true, _) => theme.reviewed_bg,
        (false, None) => Color::Reset,
    }
}

/// One list row standing for `hunks` — a hunk, or a fold of several: the
/// group's symbol (on a group's first row only), `text` cut to fit, and the
/// first hunk's new-side line number at the right edge.
///
/// No glyph says what state the row is in; its background does (`row_bg`),
/// and a reviewed or noise row dims its text. A reviewed row that had findings
/// keeps its top level on the line-number cell, so a reviewed verdict still
/// says it was one. `quiet` dims the text whatever the state, for a fold.
pub(super) fn hunk_row(
    app: &App,
    hunks: &[usize],
    symbol: &str,
    text: &str,
    quiet: bool,
    selected: bool,
    w: usize,
) -> Line<'static> {
    let t = &app.theme;
    let level = hunks
        .iter()
        .filter_map(|&i| top_level(&app.items[i]))
        .max_by_key(|l| *l as u8);
    let reviewed = hunks.iter().all(|&i| app.reviewed[i]);
    let quiet = quiet || reviewed || hunks.iter().all(|&i| app.items[i].noise);
    let mut base = Style::default().bg(row_bg(t, level, reviewed, selected));
    if selected {
        base = base.add_modifier(Modifier::BOLD);
    }
    let num = format!(" {:>4} ", app.items[hunks[0]].new_range[0]);
    let num_style = match level.filter(|_| reviewed) {
        Some(l) => base.fg(t.fg).bg(t.level_bg(l, true)),
        None => base.fg(t.dim),
    };
    let room = w.saturating_sub(SYMBOL_W + 2 + num.chars().count());
    let text = fit(text, room);
    let pad = room.saturating_sub(text.chars().count());
    Line::from(vec![
        Span::styled(" ", base),
        Span::styled(
            format!("{:<SYMBOL_W$}", fit(symbol, SYMBOL_W)),
            base.fg(if selected { t.fg } else { t.dim }),
        ),
        Span::styled(" ", base),
        Span::styled(text, base.fg(if quiet { t.dim } else { t.fg })),
        Span::styled(" ".repeat(pad), base),
        Span::styled(num, num_style),
    ])
}

fn draw_code(f: &mut Frame, app: &mut App, code_area: Option<Rect>, body: Rect) {
    let code_rect = code_area.unwrap_or(body);
    let code_w = code_rect.width as usize;
    // clamp to the selected file's longest line so hscroll can't run away
    // past any content it could ever bring into view; cached per path since
    // it only changes on a load/`:e`, not every frame
    let path = app.items[app.sel].path.as_str();
    let sources = &app.sources;
    let max_col = *app.max_col.entry(path.to_string()).or_insert_with(|| {
        sources
            .get(path)
            .map(|(_, nl)| nl.iter().map(|l| l.chars().count()).max().unwrap_or(0))
            .unwrap_or(0)
    });
    app.hscroll = app.hscroll.min(max_col.min(u16::MAX as usize) as u16);
    let it = &app.items[app.sel];
    let (search_matches, cur_match): (&[(usize, usize, usize)], Option<usize>) = match &app.search {
        Some(s) => (&s.matches, Some(s.index)),
        None => (&[], None),
    };
    // only the rows the pane can show are built; `code_total` is what the view
    // would have been, which is what the scroll clamps below still work against
    let (code, right_clip, code_total) = code_view(
        it,
        &app.sources,
        &app.highlights,
        code_w,
        app.hscroll as usize,
        Some(app.cursor),
        search_matches,
        cur_match,
        &app.theme,
        app.scroll as usize,
        app.code_height as usize,
        &line_marks(app),
        &it.findings,
    );
    let header = code_header(app, right_clip);
    app.code_len = code_total;
    app.scroll = app.scroll.min(last_line(app.code_len));
    // `code` is already the slice starting at `app.scroll`, so the paragraph
    // renders it from the top rather than scrolling within it
    if let Some(r) = code_area {
        let (head, content) = split_header(r);
        f.render_widget(
            pane_header(2, "code", header, app.focus == Pane::Code, &app.theme),
            head,
        );
        f.render_widget(Paragraph::new(Text::from(code)), content);
    }
}

// The `/` prompt and the active-search status both ride in the code pane's
// header rather than a separate widget — one line is enough for either, and it
// keeps the layout unchanged while typing or searching.
fn code_header(app: &App, right_clip: bool) -> Vec<Span<'static>> {
    let t = &app.theme;
    let dim = Style::default().fg(t.dim);
    if let Some(p) = &app.prompt {
        return vec![Span::styled(
            format!(" search: {}▏", p.text),
            Style::default().fg(t.fg),
        )];
    }
    let it = &app.items[app.sel];
    // `‹`/`›` mark content clipped off the left/right of the horizontal
    // window — truncation must never be silent, so this is always shown
    // rather than only surfaced by scrolling into it.
    let clip = match (app.hscroll > 0, right_clip) {
        (true, true) => " ‹›",
        (true, false) => " ‹",
        (false, true) => " ›",
        (false, false) => "",
    };
    let [a, b] = it.new_range;
    let lines = if b > a {
        format!("L{a}-{b}")
    } else {
        format!("L{a}")
    };
    let mut out = vec![
        Span::styled(
            format!(" {}", it.path),
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(clip, dim),
    ];
    if let Some(sym) = hunk_symbol(it) {
        out.push(Span::styled(format!("  {sym}"), dim));
    }
    out.push(Span::styled(format!("  {lines}"), dim));
    if let Some(s) = &app.search {
        let glyph = match s.kind {
            SearchKind::Text => '/',
            SearchKind::Symbol => '*',
        };
        let status = if s.matches.is_empty() {
            format!("no matches for {glyph}{}", s.pattern)
        } else {
            format!("[{}/{}] {glyph}{}", s.index + 1, s.matches.len(), s.pattern)
        };
        out.push(Span::styled(
            format!("  {status}"),
            Style::default().fg(t.accent),
        ));
    }
    out
}

// The facts on the left, each under its label, and the findings beside them;
// with no findings the facts take the width. The current line takes the
// selection tint, not REVERSED: this pane is prose, and swapping fg/bg on a
// whole wrapped paragraph reads as a block of colour rather than as "you are
// here". The code pane's cursor stays reversed — one cell, where the swap is
// exactly right.
fn draw_why(f: &mut Frame, app: &App, area: Rect, content: &[WhyRow], has_findings: bool) {
    let theme = &app.theme;
    let focused = app.focus == Pane::Why;
    let (head, body) = split_header(area);
    let sym = hunk_symbol(&app.items[app.sel])
        .map(|s| {
            vec![Span::styled(
                format!(" {s}"),
                Style::default().fg(theme.dim),
            )]
        })
        .unwrap_or_default();
    f.render_widget(pane_header(3, "why", sym, focused, theme), head);
    let (facts_area, findings_area) = if !has_findings {
        (body, None)
    } else {
        let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(1)
            .areas(body);
        (l, Some(r))
    };
    let label_w = content
        .iter()
        .map(|r| r.label.chars().count())
        .max()
        .unwrap_or(0);
    // a wrapped fact continues under its own text, not under its label
    let gutter = label_w + 3;
    let room = (facts_area.width as usize).saturating_sub(gutter);
    let facts: Vec<Line> = content
        .iter()
        .enumerate()
        .flat_map(|(i, r)| {
            let (label, text) = if focused && i == app.why_sel {
                (
                    Style::default().fg(theme.dim).bg(theme.select_bg),
                    r.style.bg(theme.select_bg),
                )
            } else {
                (Style::default().fg(theme.dim), r.style)
            };
            hang(&r.text, room, room)
                .into_iter()
                .enumerate()
                .map(move |(n, part)| {
                    let head = if n == 0 { r.label } else { "" };
                    Line::from(vec![
                        Span::styled(format!(" {head:<label_w$}  "), label),
                        Span::styled(part, text),
                    ])
                })
        })
        .collect();
    f.render_widget(
        Paragraph::new(Text::from(facts)).scroll((app.why_scroll, 0)),
        facts_area,
    );
    if let Some(r) = findings_area {
        let findings = finding_lines(&app.items[app.sel], theme, r.width as usize);
        f.render_widget(
            Paragraph::new(Text::from(findings)).scroll((app.why_scroll, 0)),
            r,
        );
    }
}

fn draw_popup(f: &mut Frame, app: &mut App, body: Rect) {
    if let Some(popup) = app.popup.as_mut() {
        popup.scroll = popup.scroll.min(last_line(popup.lines.len()));
        popup.hscroll = popup.hscroll.min(last_line(popup_width(&popup.lines)));
    }
    let Some(popup) = &app.popup else {
        return;
    };
    let rect = popup_rect(body, popup.lines.len());
    f.render_widget(Clear, rect);
    // borrow each span's content instead of cloning the popup body every
    // frame — Paragraph only needs `Into<Text>`, not an owned copy
    let text: Vec<Line> = popup
        .lines
        .iter()
        .map(|l| Line {
            style: l.style,
            alignment: l.alignment,
            spans: l
                .spans
                .iter()
                .map(|s| Span {
                    style: s.style,
                    content: std::borrow::Cow::Borrowed(s.content.as_ref()),
                })
                .collect(),
        })
        .collect();
    let clipped =
        popup_width(&popup.lines) > rect.width.saturating_sub(2) as usize || popup.hscroll > 0;
    // a popup taller than its frame said so nowhere: the help is roughly
    // twice the height it is shown at, and nothing indicated the rest
    let shown = rect.height.saturating_sub(2) as usize;
    let more = if popup.lines.len() > shown {
        format!(
            " [{}-{}/{}]",
            popup.scroll as usize + 1,
            (popup.scroll as usize + shown).min(popup.lines.len()),
            popup.lines.len()
        )
    } else {
        String::new()
    };
    let block = Block::bordered()
        .title(format!(
            " {}{}{more} ",
            popup.title,
            if clipped { " ‹›" } else { "" }
        ))
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(app.theme.mark));
    let p = Paragraph::new(Text::from(text))
        .block(block)
        .scroll((popup.scroll, popup.hscroll));
    f.render_widget(p, rect);
}

fn draw_command_bar(f: &mut Frame, app: &App) {
    let Some(bar) = &app.command else {
        return;
    };
    let area = f.area();
    let bar_rect = command_bar_rect(area);
    f.render_widget(Clear, bar_rect);
    let line = format!(":{}▏", bar.text);
    let p = Paragraph::new(Text::from(vec![Line::from(line)])).block(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(" command ")
            .border_style(Style::default().fg(app.theme.border_focus)),
    );
    f.render_widget(p, bar_rect);
    if bar.candidates.is_empty() {
        return;
    }
    let menu_rect = command_menu_rect(bar_rect, area, bar.candidates.len());
    f.render_widget(Clear, menu_rect);
    // completing the command itself: each row carries the command's help
    // sentence, since there is no central help to look it up in. Completing
    // an argument: just the candidates.
    let naming = !bar.text.contains(char::is_whitespace);
    let width = menu_rect.width.saturating_sub(2) as usize;
    let entries: Vec<ListItem> = bar
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let style = if bar.selected == Some(i) {
                Style::default().bg(app.theme.select_bg)
            } else {
                Style::default()
            };
            let (head, help) = command_menu_row(c, naming, &bar.candidates, width);
            let mut spans = vec![Span::styled(head, style)];
            if let Some(h) = help {
                spans.push(Span::styled(h, style.fg(app.theme.dim)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    f.render_widget(List::new(entries).block(Block::bordered()), menu_rect);
}

/// One row of the completion menu. While the command *name* is being
/// completed the row is `name <args>` padded to a common column, then the
/// command's help sentence, cut to what fits; an alias or an argument
/// candidate has no sentence and is shown as is.
pub(super) fn command_menu_row(
    candidate: &str,
    naming: bool,
    all: &[String],
    width: usize,
) -> (String, Option<String>) {
    let cmd = naming
        .then(|| COMMANDS.iter().find(|c| c.name == candidate))
        .flatten();
    let Some(cmd) = cmd else {
        return (candidate.to_string(), None);
    };
    let label = |c: &Cmd| {
        if c.args.is_empty() {
            c.name.to_string()
        } else {
            format!("{} {}", c.name, c.args)
        }
    };
    let col = all
        .iter()
        .filter_map(|n| COMMANDS.iter().find(|c| c.name == n))
        .map(|c| label(c).chars().count())
        .max()
        .unwrap_or(0);
    let head = format!("{:<col$}", label(cmd));
    let room = width.saturating_sub(head.chars().count() + 4);
    if room < 8 {
        return (head, None);
    }
    let mut help: String = cmd.help.chars().take(room).collect();
    if help.chars().count() < cmd.help.chars().count() {
        help.pop();
        help.push('…');
    }
    (head, Some(format!("  — {help}")))
}

// centered floating box over `area`, sized to the popup's content
fn popup_rect(area: Rect, n_lines: usize) -> Rect {
    let w = (area.width.saturating_sub(4)).clamp(20, 90);
    let h = ((n_lines as u16) + 2).clamp(3, area.height.saturating_sub(2).max(3));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

// the command bar itself: one content row, centered, ~60% of the terminal's
// width, a couple of rows down from the top
fn command_bar_rect(area: Rect) -> Rect {
    let w = ((area.width as u32 * 3 / 5) as u16).clamp(20, area.width.saturating_sub(4).max(20));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + 1,
        width: w,
        height: 3,
    }
}

// the completion menu: left-aligned to `bar`, directly below it, sized to
// its candidate rows but never past the bottom of the terminal
fn command_menu_rect(bar: Rect, area: Rect, n_candidates: usize) -> Rect {
    let below = area.y + area.height;
    let avail = below.saturating_sub(bar.y + bar.height).max(3);
    let h = ((n_candidates as u16) + 2).clamp(3, avail);
    Rect {
        x: bar.x,
        y: bar.y + bar.height,
        width: bar.width,
        height: h,
    }
}

// ---- what the why pane says about a hunk beyond its rationale

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

/// The cascade as one why-pane line, or `None` when rejecting this hunk would
/// strand nothing.
pub(super) fn cascade_line(app: &App, i: usize) -> Option<String> {
    let hit = cascade(app, i);
    if hit.is_empty() {
        return None;
    }
    let where_ = hit
        .iter()
        .take(3)
        .map(|&j| format!("{}:L{}", app.items[j].path, app.items[j].new_range[0]))
        .collect::<Vec<_>>()
        .join(", ");
    let more = if hit.len() > 3 {
        format!(" and {} more", hit.len() - 3)
    } else {
        String::new()
    };
    Some(format!(
        "rejecting this strands {} hunk{} ({where_}{more})",
        hit.len(),
        if hit.len() == 1 { "" } else { "s" }
    ))
}

/// The one-line delta for item `i`, or `None` when it reads exactly as it did
/// last time — and when there was no last time, since "everything is new" on a
/// first run is noise rather than information.
/// `FILE_CHURN_WINDOW` as prose — "6.months" is a git argument, not English.
fn churn_window_phrase() -> String {
    FILE_CHURN_WINDOW.replace('.', " ")
}

/// The churn row for hunk `i`: the file's recent churn, or the hunk's own once
/// `H` has asked for it.
///
/// Rendered here rather than in `why_rows` so that function stays a pure
/// function of an `Item`, the same way `delta_line` and `cascade_line` do it.
pub(super) fn churn_line(app: &App, i: usize) -> Option<String> {
    let it = app.items.get(i)?;
    let key = (it.path.clone(), it.new_range[0], it.new_range[1]);
    let Some(churn) = app.churn_cache.get(&key) else {
        // not asked for this hunk: fall back to the file, which load already
        // counted. The specific answer replaces this one when `H` asks for it.
        return match app.file_churn.get(&it.path) {
            None | Some(0) => None,
            Some(1) => Some(format!(
                "this file changed once in the last {}",
                churn_window_phrase()
            )),
            Some(n) => Some(format!(
                "this file changed {n} times in the last {}",
                churn_window_phrase()
            )),
        };
    };
    let Some(c) = churn else {
        return Some("churn: unavailable (no commit to review from)".to_string());
    };
    let count = match (c.commits, c.capped) {
        (0, _) => "these lines have not changed before".to_string(),
        (1, false) => "these lines changed once before".to_string(),
        (n, false) => format!("these lines changed {n} times before"),
        (n, true) => format!("these lines changed at least {n} times before"),
    };
    Some(match &c.last {
        Some((author, date)) => format!("{count} — last {date} by {author}"),
        None => count,
    })
}

pub(super) fn delta_line(app: &App, i: usize) -> Option<&'static str> {
    if app.deltas.iter().all(|d| *d == Delta::New) {
        return None; // no previous run to compare against
    }
    match app.deltas.get(i)? {
        Delta::New => Some("new since you last looked"),
        Delta::Changed => Some("changed since you last looked"),
        Delta::Moved => {
            Some("unchanged, but it reads in a different place now — its dependencies moved")
        }
        Delta::Same => None,
    }
}

/// The locations of `i`'s unreviewed dependencies, for the why pane — empty
/// unless `i` is itself marked reviewed, since the warning is about the *order*
/// things were approved in, not about work still to do.
pub(super) fn out_of_order_labels(app: &App, i: usize) -> Vec<String> {
    if !app.reviewed[i] {
        return vec![];
    }
    unreviewed_deps(app, i)
        .into_iter()
        .map(|t| format!("{}:L{}", app.items[t].path, app.items[t].new_range[0]))
        .collect()
}

/// Dependencies of item `i` that are part of this review but not yet reviewed
/// — the hunks defining what `i` uses. Marking `i` reviewed while any of these
/// are outstanding means a call was approved before its callee.
fn unreviewed_deps(app: &App, i: usize) -> Vec<usize> {
    app.items[i]
        .edges
        .iter()
        .filter(|e| e.dependency)
        .filter_map(|e| e.target)
        .filter(|&t| !app.reviewed[t])
        .collect()
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

/// What the code pane marks on the selected item's file: its comments, and
/// the selection being made.
fn line_marks(app: &App) -> LineMarks {
    let path = &app.items[app.sel].path;
    LineMarks {
        commented: app
            .comments
            .iter()
            .filter(|c| c.path == *path)
            .map(|c| (c.start, c.end))
            .collect(),
        selected: app.selection.map(|_| comment_lines(app)),
    }
}

/// Item `i`'s line comments, as the why pane lists them.
fn comments_in(app: &App, i: usize) -> Vec<String> {
    let it = &app.items[i];
    let [a, b] = it.new_range;
    app.comments
        .iter()
        .filter(|c| c.path == it.path && c.start <= b.max(a) && a <= c.end)
        .map(|c| {
            let stale = if c.stale {
                " (lines changed since)"
            } else {
                ""
            };
            format!(
                "L{}{} {}{stale}",
                c.start,
                if c.end > c.start {
                    format!("-{}", c.end)
                } else {
                    String::new()
                },
                c.text
            )
        })
        .collect()
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
