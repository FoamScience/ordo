// -------------------------------------------------------------------- draw
use crate::clamp_cursor;
use crate::code_view::code_view;
use crate::code_view::slice_range;
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
use crate::highlight::cat_name;
use crate::highlight::LineSpans;
use crate::history::FILE_CHURN_WINDOW;
use crate::keys::Pane;
use crate::last_line;
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
use ratatui::widgets::Wrap;
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
            .fg(theme.category)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(theme.dim)
    }
}

/// The why pane's content, in render order — reason, details, notes, dep
/// (edge) lines, then advisories. A pure function of `Item` (plus `view`, so
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
    /// dep targets `:only-wave` hides, each with its wave and reviewed state:
    /// still followable, since `gd` switches to their wave
    other_waves: &'a [(usize, String)],
    /// what this hunk's wave was asked (`Waves::asked`)
    wave_asked: Option<&'a str>,
}

pub(super) fn why_rows(it: &Item, view: &[usize], theme: &Theme, ctx: &WhyContext) -> Vec<WhyRow> {
    let mut rows = annotation_rows(ctx, theme);
    rows.extend(rationale_rows(it, theme));
    // Churn sits with coverage: both answer "how much weight does this hunk
    // deserve" rather than "what does it do".
    if let Some(c) = ctx.churn {
        rows.push(text_row(c.to_string(), theme.mark));
    }
    if let Some(w) = it.wave {
        let row = match ctx.wave_asked {
            Some(asked) => format!("wave {w} · asked: {asked}"),
            None => format!("wave {w}"),
        };
        rows.push(text_row(row, theme.accent));
    }
    rows.extend(signal_rows(it, theme));
    for e in &it.edges {
        let elsewhere = e
            .target
            .and_then(|t| ctx.other_waves.iter().find(|(i, _)| *i == t));
        let target = e.target.filter(|t| view.contains(t) || elsewhere.is_some());
        let text = match elsewhere {
            Some((_, wave)) => format!("dep {} · {wave}", e.label),
            None => format!("dep {}", e.label),
        };
        rows.push(WhyRow {
            text,
            style: edge_style(target, theme),
            kind: WhyKind::Edge(target),
        });
    }
    rows.extend(finding_rows(it, theme));
    rows
}

fn text_row(text: String, color: Color) -> WhyRow {
    WhyRow {
        text,
        style: Style::default().fg(color),
        kind: WhyKind::Text,
    }
}

// what the reviewer and the last run said about this hunk, ahead of what the
// engine derived
fn annotation_rows(ctx: &WhyContext, theme: &Theme) -> Vec<WhyRow> {
    let mut rows = vec![];
    if let Some(c) = ctx.cascade {
        rows.push(text_row(format!("· {c}"), theme.accent));
    }
    if let Some(d) = ctx.delta {
        rows.push(text_row(format!("· {d}"), theme.accent));
    }
    // approving a call before its callee is the one review-order mistake the
    // graph can actually prove
    if !ctx.out_of_order.is_empty() {
        rows.push(text_row(
            format!(
                "⚠ marked reviewed, but depends on unreviewed {}",
                ctx.out_of_order.join(", ")
            ),
            theme.warn,
        ));
    }
    // a review note leads: it is the reviewer's own words about this symbol,
    // and it outranks anything the engine derived
    if let Some(n) = ctx.note {
        rows.push(text_row(format!("note: {n}"), theme.warn));
    }
    for c in ctx.comments {
        rows.push(text_row(c.clone(), theme.warn));
    }
    rows
}

// The engine joins independent clauses with "; " to fit one line, which is
// right for `hunks[].rationale` in the JSON and wrong here: a busy hunk
// arrived as one wrapped paragraph ("adds a, b, c, and 5 more; adds CFG (L30,
// L37, …), HOST (…), READY (…)") that has to be read word by word. One clause
// per row restores what the join flattened, and the pane is sized to its
// wrapped height, so the rows are free.
fn rationale_rows(it: &Item, theme: &Theme) -> Vec<WhyRow> {
    // The engine's terminal fallback rationale: it found nothing to say about
    // the hunk, so a "reason: change" line says nothing either — leave it out.
    if it.rationale == "change" {
        return vec![];
    }
    it.rationale
        .split("; ")
        .enumerate()
        .map(|(i, frag)| {
            let text = if i == 0 {
                format!("reason: {frag}")
            } else {
                // aligned under the first clause, not under the label
                format!("        {frag}")
            };
            text_row(text, theme.border_focus)
        })
        .collect()
}

// coverage, the detail layer and the engine's notes
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
        rows.push(text_row(
            text,
            if cold == 0 {
                theme.reviewed
            } else {
                theme.warn
            },
        ));
    }
    for d in &it.details {
        rows.push(text_row(format!("- {d}"), theme.accent));
    }
    if !it.notes.is_empty() {
        rows.push(text_row(
            format!("notes: {}", it.notes.join("; ")),
            theme.mark,
        ));
    }
    rows
}

// every finding reads the same way, whoever found it: a named head line,
// then the message beneath it
fn finding_rows(it: &Item, theme: &Theme) -> Vec<WhyRow> {
    let mut rows = vec![];
    for f in &it.findings {
        let (head, color) = match f.level {
            ordo::model::Level::Note => (f.name.clone(), theme.category),
            _ => (format!("⚠ {}", f.name), theme.warn),
        };
        rows.push(WhyRow {
            text: head,
            style: Style::default().fg(color).add_modifier(Modifier::BOLD),
            kind: WhyKind::Text,
        });
        for ml in f.message.lines() {
            rows.push(text_row(format!("  {ml}"), theme.fg));
        }
    }
    rows
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
    it.symbols
        .first()
        .map(|s| s.name.clone())
        .or_else(|| it.enclosing.clone())
        .unwrap_or_else(|| it.path.clone())
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

/// The focused pane gets a cyan border.
/// A pane's frame. Rounded corners and a dim border for context, the theme's
/// focus colour for the pane that has it — the border is how the reviewer knows
/// where the keys will land, so it is the one piece of chrome allowed to be loud.
/// The bottom row: which pane each digit selects, whether the frame is zoomed,
/// and the movement keys the code pane's title used to carry and truncate.
/// The focused pane's own digit is emphasised, so focus is legible without
/// hunting for the highlighted border.
fn footer_spans(app: &App, zoomed: bool) -> Vec<Span<'static>> {
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
    let mut panes: Vec<(u8, Pane, &str)> = vec![
        (1, Pane::List, "list"),
        (2, Pane::Code, "code"),
        (3, Pane::Why, "why"),
    ];
    if app.canvas.is_some() {
        // a floating view takes the next free digit while it exists
        panes.push((4, Pane::Deps, "deps"));
    }
    for (n, pane, label) in panes {
        let on = app.focus == pane;
        let style = if on {
            Style::default()
                .fg(theme.border_focus)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        out.push(Span::styled(format!(" {n} {label}"), style));
    }
    let hint = if zoomed {
        // say how to get out of it, since the other two panes are not on screen
        if app.zoom {
            "  ·  zoomed (same digit restores)"
        } else {
            "  ·  narrow: one pane at a time"
        }
    } else {
        "  ·  same digit zooms"
    };
    out.push(Span::styled(
        hint.to_string(),
        Style::default().fg(theme.dim),
    ));
    out.push(Span::styled(
        format!("  ·  {}  ·  ? help ", app.keys.hint),
        Style::default().fg(theme.dim),
    ));
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
    if body.width < SPLIT_COLS {
        // the split is unavailable at this width, so an explicit toggle would
        // sit invisible and surprise the reviewer when the terminal widens
        app.zoom = false;
    }
    let why_widths: Vec<usize> = why_content(app)
        .iter()
        .map(|r| r.text.chars().count())
        .collect();
    let panes = pane_rects(body, app.focus, app.zoom, &why_widths);
    record_geometry(app, &panes, body);
}

/// Copy a computed layout onto `app`. A hidden pane records the height it would
/// have if the next keypress brought it back, so paging never runs against a
/// zero — and `body` is not a guess there: it is exactly the rect that pane
/// gets when it is zoomed to.
fn record_geometry(app: &mut App, panes: &Panes, body: Rect) {
    let code = panes.code.unwrap_or(body);
    app.code_height = code.height.saturating_sub(2);
    app.code_width = (code.width as usize)
        .saturating_sub(2)
        .saturating_sub(GUTTER_W)
        .min(u16::MAX as usize) as u16;
    app.why_height = panes.why.unwrap_or(body).height.saturating_sub(2);
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
        true,
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
        card_widget(
            f,
            app,
            *rect,
            card.idx,
            &card.label,
            false,
            c.sel == Some(i),
        );
    }
}

/// One framed card: a few rows of the target hunk, rendered by the same
/// `code_view` the code pane uses, so syntax and diff tint are identical. The
/// selected card starts `Canvas::scroll` rows further down.
fn card_widget(
    f: &mut Frame,
    app: &App,
    rect: Rect,
    idx: usize,
    label: &str,
    anchor: bool,
    selected: bool,
) {
    if rect.width < 8 || rect.height < 3 {
        return;
    }
    let theme = &app.theme;
    let border = if selected {
        theme.border_focus
    } else if anchor {
        theme.accent
    } else {
        theme.border
    };
    let rows = rect.height.saturating_sub(2) as usize;
    let it = &app.items[idx];
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
        rect.width.saturating_sub(2) as usize,
        0,
        None,
        &[],
        None,
        theme,
        start,
        rows,
        &LineMarks::default(),
    );
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            format!(" {label} "),
            Style::default().fg(if selected {
                theme.border_focus
            } else {
                theme.dim
            }),
        ));
    f.render_widget(Paragraph::new(Text::from(lines)).block(block), rect);
}

/// Where each pane sits this frame. Pure in the terminal size, the focus and
/// the zoom flag, so the event loop can ask the same question the renderer
/// does instead of reading what the last frame happened to leave behind.
pub(super) struct Panes {
    pub(super) zoomed: bool,
    pub(super) list: Option<Rect>,
    pub(super) code: Option<Rect>,
    pub(super) why: Option<Rect>,
}

/// `why_widths` is each rationale row's character count; the why pane wraps, so
/// its height is the wrapped row count, not the logical one.
pub(super) fn pane_rects(body: Rect, focus: Pane, zoom: bool, why_widths: &[usize]) -> Panes {
    // Below `SPLIT_COLS` three panes each get too little to be read; one good
    // pane beats three starved ones, so a narrow terminal zooms on its own.
    let zoomed = zoom || body.width < SPLIT_COLS;
    if zoomed {
        let (list, code, why) = match focus {
            Pane::List => (Some(body), None, None),
            // the canvas floats over whatever is underneath, so while it has
            // focus the panes keep the layout they had
            Pane::Code | Pane::Deps => (None, Some(body), None),
            Pane::Why => (None, None, Some(body)),
        };
        return Panes {
            zoomed,
            list,
            code,
            why,
        };
    }
    let cols =
        Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).split(body);
    // content height plus borders, floored so an empty pane still reads as a
    // pane, capped so a long rationale cannot crowd out the code
    let why_w = (cols[1].width.saturating_sub(2)).max(1) as usize;
    let wrapped: usize = why_widths.iter().map(|w| w.div_ceil(why_w).max(1)).sum();
    let why_h = (wrapped as u16)
        .saturating_add(2)
        .clamp(3, (body.height * 2 / 5).max(3));
    let rhs = Layout::vertical([Constraint::Min(3), Constraint::Length(why_h)]).split(cols[1]);
    Panes {
        zoomed,
        list: Some(cols[0]),
        code: Some(rhs[0]),
        why: Some(rhs[1]),
    }
}

/// The body rect — everything above the footer row.
fn body_of(area: Rect) -> Rect {
    Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area)[0]
}

/// Below this the three panes each get too little width to be read, so the
/// focused one takes the frame instead. Measured, not guessed: at 80 columns a
/// 38% list pane is 30 wide and loses the line number off `gate.py:L117`.
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

/// A pane's frame. Only the focused one is drawn.
///
/// Measured at 140x42 on the default theme, the border colour was 37% of every
/// non-space cell painted — the single most-used colour on screen, and drawn
/// at a contrast against the background low enough to read as texture rather
/// than structure (tasks-9sj.30). Three full rectangles repeat what the title,
/// the footer and the pane numbers already say.
///
/// So an unfocused pane keeps the frame's *space* and loses its glyphs: the
/// panes stay separated by the gap the border occupied, the layout does not
/// shift by a cell when focus moves, and the one box still on screen means
/// "you are here" instead of "this is a pane".
fn pane_block(title: String, focused: bool, theme: &Theme) -> Block<'static> {
    let block = Block::bordered().title(Span::styled(
        title,
        Style::default().fg(if focused {
            theme.border_focus
        } else {
            theme.dim
        }),
    ));
    if focused {
        block
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focus))
    } else {
        block.border_set(ratatui::symbols::border::EMPTY)
    }
}

pub(super) fn draw(f: &mut Frame, app: &mut App, rev: &str) {
    let area = f.area();
    if area.width < MIN_COLS || area.height < MIN_ROWS {
        draw_too_small(f, area, &app.theme);
        return;
    }
    // one row reserved at the bottom for the key hint, which used to ride in
    // the code pane's border title and was truncated mid-word there
    let root = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    let (body, footer_area) = (root[0], root[1]);
    if body.width < SPLIT_COLS {
        // the split is unavailable at this width, so an explicit toggle would
        // sit invisible and surprise the reviewer when the terminal widens
        app.zoom = false;
    }
    // the same for a side card's zoom: the canvas is inset by two, so it fans
    // two columns later than the panes split
    if body.width < SPLIT_COLS + 2 {
        if let Some(c) = app.canvas.as_mut().filter(|c| c.sel.is_some()) {
            c.zoomed = false;
        }
    }
    // built before the layout, because the why pane is sized to it: it used to
    // take a flat 30% and sat nearly empty on a one-line rationale
    let why_content = why_content(app);
    let why_widths: Vec<usize> = why_content.iter().map(|r| r.text.chars().count()).collect();
    let panes = pane_rects(body, app.focus, app.zoom, &why_widths);
    // Record what the panes came out as, rather than deriving it a second
    // time. `set_geometry` answers the same question for the event loop from
    // the same `pane_rects`, but has to rebuild the why rows to do it; here
    // they are already built.
    record_geometry(app, &panes, body);

    draw_list(f, app, rev, panes.list, body);
    // right — code (top) + why (bottom); either may be absent while zoomed.
    // A hidden pane falls back to `body`, which is not a guess: that is exactly
    // the rect it gets when the next keypress zooms to it.
    draw_code(f, app, panes.code, body);
    // content, not geometry: the why pane's length only exists once its rows
    // are built; the pane size it clamps against was set by `record_geometry`
    app.why_len = why_content.len();
    app.why_scroll = app.why_scroll.min(last_line(app.why_len));
    app.why_sel = app.why_sel.min(last_line(app.why_len) as usize);
    if let Some(r) = panes.why {
        draw_why(f, app, r, &why_content);
    }
    f.render_widget(
        Paragraph::new(Line::from(footer_spans(app, panes.zoomed))),
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
            let mark = if app.reviewed[t] { " ✓" } else { "" };
            (t, format!("{wave}{mark}"))
        })
        .collect()
}

// left — reading order
fn draw_list(f: &mut Frame, app: &App, rev: &str, list_area: Option<Rect>, body: Rect) {
    let symbol = "▶ ";
    let text_w = (list_area.map_or(body.width, |a| a.width) as usize)
        .saturating_sub(2 + symbol.chars().count());
    let display = display_rows(
        &app.view,
        &app.items,
        &app.groups,
        app.show_groups,
        &app.collapsed,
    );
    let rows: Vec<ListItem> = display
        .iter()
        .map(|row| match row {
            DisplayRow::Header(reason) => {
                // bold, not blue: a group header is a structural label, and
                // wearing the focus colour is what stopped that colour meaning
                // focus (tasks-9sj.31). Weight separates it from its rows
                // without spending the accent.
                let spans = vec![Span::styled(
                    reason.to_string(),
                    Style::default()
                        .fg(app.theme.fg)
                        .add_modifier(Modifier::BOLD),
                )];
                ListItem::new(Line::from(slice_range(spans, 0, text_w)))
            }
            DisplayRow::Item(i) => {
                ListItem::new(Line::from(slice_range(list_row(app, *i), 0, text_w)))
            }
        })
        .collect();
    let (done, total, edges_done, edges_total) = coverage(app);
    let mut state = ListState::default();
    let sel_row = display_row_of(
        &app.view,
        &app.items,
        app.show_groups,
        &app.collapsed,
        view_pos(&app.view, app.sel),
    );
    state.select(Some(sel_row));
    let filtered = app.view.len() < app.items.len();
    let list = List::new(rows)
        .block(pane_block(
            format!(
                " 1 {rev} — {done}/{total} reviewed{}{} · {} ",
                // edge coverage is the number that tracks understanding; it is
                // omitted when the review has no def→use links to cover
                if edges_total > 0 {
                    format!(" · {edges_done}/{edges_total} edges")
                } else {
                    String::new()
                },
                if filtered { " (filtered)" } else { "" },
                app.keys.name
            ),
            app.focus == Pane::List,
            &app.theme,
        ))
        .highlight_style(Style::default().bg(app.theme.select_bg))
        .highlight_symbol(symbol);
    if let Some(r) = list_area {
        f.render_stateful_widget(list, r, &mut state);
    }
}

// one hunk's row: head only — the full rationale lives in the "why" pane.
// Path, line number and category are separate spans so each reads at a
// glance; a noise or reviewed row overrides all three, because *that* is what
// the row is saying.
fn list_row(app: &App, i: usize) -> Vec<Span<'static>> {
    let it = &app.items[i];
    let style = if it.noise {
        Style::default().fg(app.theme.dim)
    } else if app.reviewed[i] {
        Style::default().fg(app.theme.reviewed)
    } else {
        Style::default().fg(app.theme.fg)
    };
    let tinted = it.noise || app.reviewed[i];
    let dim = |c: Color| if tinted { style } else { style.fg(c) };
    vec![
        // two cells of indent put the file under the definition it belongs
        // to; headers sit flush, so the list reads as a tree
        Span::styled("  ".to_string(), style),
        Span::styled(it.mark.clone(), dim(app.theme.mark)),
        Span::styled(it.path.clone(), style),
        // a line number says *where*, not *look here*: the accent is reserved
        // for focus and selection (tasks-9sj.31)
        Span::styled(format!(":L{}", it.new_range[0]), dim(app.theme.dim)),
        Span::styled(format!(" [{}]", cat_name(it.cat)), dim(app.theme.category)),
    ]
}

fn draw_code(f: &mut Frame, app: &mut App, code_area: Option<Rect>, body: Rect) {
    let code_rect = code_area.unwrap_or(body);
    let code_w = (code_rect.width as usize).saturating_sub(2);
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
    );
    let title = code_title(app, right_clip);
    app.code_len = code_total;
    app.scroll = app.scroll.min(last_line(app.code_len));
    // `code` is already the slice starting at `app.scroll`, so the paragraph
    // renders it from the top rather than scrolling within it
    if let Some(r) = code_area {
        let code_view = Paragraph::new(Text::from(code))
            .block(pane_block(title, app.focus == Pane::Code, &app.theme))
            .scroll((0, 0));
        f.render_widget(code_view, r);
    }
}

// The `/` prompt and the active-search status both reuse the code pane's
// border title rather than a separate widget — one line is enough for either,
// and it keeps the layout unchanged while typing or searching.
fn code_title(app: &App, right_clip: bool) -> String {
    // `‹`/`›` mark content clipped off the left/right of the horizontal
    // window — truncation must never be silent, so this is always shown
    // rather than only surfaced by scrolling into it.
    let clip = match (app.hscroll > 0, right_clip) {
        (true, true) => " ‹›",
        (true, false) => " ‹",
        (false, true) => " ›",
        (false, false) => "",
    };
    let path = &app.items[app.sel].path;
    if let Some(p) = &app.prompt {
        return format!(" search: {}▏ ", p.text);
    }
    let Some(s) = &app.search else {
        return format!(" 2 {path}{clip} ");
    };
    let glyph = match s.kind {
        SearchKind::Text => '/',
        SearchKind::Symbol => '*',
    };
    let status = if s.matches.is_empty() {
        format!("no matches for {glyph}{}", s.pattern)
    } else {
        format!("[{}/{}] {glyph}{}", s.index + 1, s.matches.len(), s.pattern)
    };
    format!(" 2 {path}{clip}  {status} ")
}

// the current line takes the list's selection tint, not REVERSED: this pane
// is prose, and swapping fg/bg on a whole wrapped paragraph reads as a block
// of colour rather than as "you are here". The code pane's cursor stays
// reversed — one cell, where the swap is exactly right.
fn draw_why(f: &mut Frame, app: &App, area: Rect, content: &[WhyRow]) {
    let why: Vec<Line> = content
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let style = if app.focus == Pane::Why && i == app.why_sel {
                r.style.bg(app.theme.select_bg)
            } else {
                r.style
            };
            Line::from(Span::styled(r.text.clone(), style))
        })
        .collect();
    let info = Paragraph::new(Text::from(why))
        .block(pane_block(
            " 3 why ".to_string(),
            app.focus == Pane::Why,
            &app.theme,
        ))
        .scroll((app.why_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(info, area);
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
                "● L{}{} {}{stale}",
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

/// The note anchored to item `i`'s symbol, if any.
pub(super) fn note_for(app: &App, i: usize) -> Option<&str> {
    let key = note_key(&app.items[i])?;
    app.notes.get(&key).map(String::as_str)
}
