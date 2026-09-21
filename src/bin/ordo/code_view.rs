// ------------------------------------------------------------------- code view
use crate::highlight::highlight_spec;
use crate::highlight::Highlights;
use crate::highlight::LineSpans;
use crate::history::history_lines;
use crate::last_line;
use crate::prose;
use crate::App;
use crate::Cursor;
use crate::Item;
use crate::ParsedFile;
use crate::Popup;
use crate::Sources;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::collections::HashMap;
use std::collections::HashSet;
use tree_sitter::Node;
use tree_sitter::Parser;
use tree_sitter::Point;

// The reviewer's whole palette, in one place.
//
// Two kinds of theme live here. A **terminal** theme (`dark`, `light`) names
// its foregrounds with ANSI colours, so it inherits whatever palette the
// terminal is already configured with — the right default, since it matches the
// rest of the user's setup for free. A **truecolor** theme (catppuccin,
// tokyonight, …) names every colour itself, for a reviewer who wants ordo to
// look like their editor rather than like their shell.
//
// No theme paints a window background: leaving it to the terminal keeps
// transparency and blur setups intact. What a theme's tints *do* assume is a
// terminal background of roughly matching lightness — hence `--theme` /
// `$ORDO_TUI_THEME` / `[theme] name` being an explicit choice rather than a
// detection (OSC 11 background queries aren't reliably supported).

/// `0x89b4fa` → `Color::Rgb(0x89, 0xb4, 0xfa)`.
pub(super) const fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// What a highlight capture *means*. A theme colours these twelve roles rather
/// than the twenty-six capture names `HL` maps onto them, so adding a grammar's
/// capture never means touching every theme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Role {
    Comment,
    Keyword,
    Str,
    Number,
    Function,
    Type,
    Property,
    Operator,
    Variable,
    Builtin,
    Param,
    Attribute,
}

#[derive(Clone, Copy)]
pub(super) struct Syntax {
    pub(super) comment: Color,
    pub(super) keyword: Color,
    pub(super) string: Color,
    pub(super) number: Color,
    pub(super) function: Color,
    pub(super) type_: Color,
    pub(super) property: Color,
    pub(super) operator: Color,
    pub(super) variable: Color,
    pub(super) builtin: Color,
    pub(super) param: Color,
    pub(super) attribute: Color,
}

impl Syntax {
    pub(super) fn of(&self, role: Role) -> Color {
        match role {
            Role::Comment => self.comment,
            Role::Keyword => self.keyword,
            Role::Str => self.string,
            Role::Number => self.number,
            Role::Function => self.function,
            Role::Type => self.type_,
            Role::Property => self.property,
            Role::Operator => self.operator,
            Role::Variable => self.variable,
            Role::Builtin => self.builtin,
            Role::Param => self.param,
            Role::Attribute => self.attribute,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Theme {
    pub(super) name: &'static str,
    // ---- chrome
    /// default text; `Reset` on a terminal theme, so the terminal's own
    /// foreground shows through
    pub(super) fg: Color,
    /// gutters, context line numbers, noise rows — present but recessive
    pub(super) dim: Color,
    pub(super) border: Color,
    /// the focused pane's border, the one piece of chrome that must be obvious
    pub(super) border_focus: Color,
    /// line numbers in the reading order
    pub(super) accent: Color,
    /// a hunk's `[category]`
    pub(super) category: Color,
    /// the `⚠` advisory mark
    pub(super) mark: Color,
    /// a reviewed row's `✓`
    pub(super) reviewed: Color,
    /// an advisory's verdict line, and the command bar's error text
    pub(super) warn: Color,
    // ---- diff
    pub(super) add_fg: Color,
    pub(super) del_fg: Color,
    pub(super) add_bg: Color,
    pub(super) del_bg: Color,
    /// the *changed part* of a line whose counterpart was identified — the
    /// line keeps the quiet add/del tint, and only what actually changed gets
    /// these (Neovim's DiffText over DiffChange)
    pub(super) add_strong_bg: Color,
    pub(super) del_strong_bg: Color,
    /// reading-order list's selected-row tint — a neutral slate, subtle next
    /// to add_bg/del_bg rather than the full fg/bg swap REVERSED gives
    pub(super) select_bg: Color,
    /// search-match backgrounds — current match brighter than the rest, so
    /// it reads as "here" among however many others are also highlighted
    pub(super) match_bg: Color,
    pub(super) match_cur_bg: Color,
    // ---- code
    pub(super) syn: Syntax,
}

impl Theme {
    /// The terminal's own palette for text, with truecolor diff tints. The
    /// default, and the only themes that inherit the user's terminal colours.
    pub(super) fn terminal(name: &'static str, light: bool) -> Theme {
        let syn = Syntax {
            comment: Color::DarkGray,
            keyword: Color::Magenta,
            string: Color::Green,
            number: Color::Cyan,
            function: Color::Blue,
            type_: Color::Yellow,
            property: Color::LightBlue,
            operator: Color::Gray,
            variable: Color::Reset,
            builtin: Color::Red,
            param: Color::LightRed,
            attribute: Color::Cyan,
        };
        let chrome = Theme {
            name,
            fg: Color::Reset,
            dim: Color::DarkGray,
            border: Color::Reset,
            border_focus: Color::Cyan,
            accent: Color::Blue,
            category: Color::Magenta,
            mark: Color::Yellow,
            reviewed: Color::Green,
            warn: Color::Red,
            add_fg: Color::Green,
            del_fg: Color::Red,
            // filled in per lightness below
            add_bg: Color::Reset,
            del_bg: Color::Reset,
            add_strong_bg: Color::Reset,
            del_strong_bg: Color::Reset,
            select_bg: Color::Reset,
            match_bg: Color::Reset,
            match_cur_bg: Color::Reset,
            syn,
        };
        if light {
            // pale tints of the same hues, at light-terminal weight — not the
            // dark values inverted, which would read as loud on a light page
            Theme {
                add_bg: hex(0xd6f0da),
                del_bg: hex(0xf8d6d6),
                add_strong_bg: hex(0xa0dcaf),
                del_strong_bg: hex(0xf5aeae),
                select_bg: hex(0xdee2ec),
                match_bg: hex(0xffecaa),
                match_cur_bg: hex(0xffca44),
                ..chrome
            }
        } else {
            Theme {
                add_bg: hex(0x142819),
                del_bg: hex(0x32181c),
                add_strong_bg: hex(0x22542e),
                del_strong_bg: hex(0x68282e),
                select_bg: hex(0x2d323e),
                match_bg: hex(0x463c0a),
                match_cur_bg: hex(0x8c6e0f),
                ..chrome
            }
        }
    }
}

/// A truecolor theme, built from the eleven colours these palettes all publish.
/// Every field of `Theme` is derived here, so a palette is eleven lines rather
/// than twenty-five, and two themes can't disagree about which colour plays
/// which role.
struct Palette {
    name: &'static str,
    fg: u32,
    dim: u32,
    /// the palette's own "surface"/"current line" tone — the selection tint
    surface: u32,
    red: u32,
    green: u32,
    yellow: u32,
    blue: u32,
    magenta: u32,
    cyan: u32,
    orange: u32,
    /// how far to lift a tint off the background: dark palettes need a floor,
    /// light ones need to stay pale
    light: bool,
}

impl Palette {
    /// Mix `a` toward `b` by `w`/256 — how a diff tint is derived from a
    /// palette colour rather than guessed at per theme.
    const fn mix(a: u32, b: u32, w: u32) -> Color {
        const fn ch(a: u32, b: u32, w: u32, sh: u32) -> u8 {
            let (x, y) = ((a >> sh) & 0xff, (b >> sh) & 0xff);
            ((x * (256 - w) + y * w) / 256) as u8
        }
        Color::Rgb(ch(a, b, w, 16), ch(a, b, w, 8), ch(a, b, w, 0))
    }

    fn theme(&self) -> Theme {
        // a tint is the accent mixed into the page: toward black on a dark
        // palette, toward white on a light one
        let ground = if self.light { 0xffffff } else { 0x000000 };
        // how far the tint sits from the page: quiet enough to read a whole
        // line over, strong enough that the refined span stands out inside it
        let quiet = if self.light { 200 } else { 210 };
        let strong = 130;
        Theme {
            name: self.name,
            fg: hex(self.fg),
            dim: hex(self.dim),
            border: hex(self.surface),
            border_focus: hex(self.blue),
            accent: hex(self.blue),
            category: hex(self.magenta),
            mark: hex(self.yellow),
            reviewed: hex(self.green),
            warn: hex(self.red),
            add_fg: hex(self.green),
            del_fg: hex(self.red),
            add_bg: Palette::mix(self.green, ground, quiet),
            del_bg: Palette::mix(self.red, ground, quiet),
            add_strong_bg: Palette::mix(self.green, ground, strong),
            del_strong_bg: Palette::mix(self.red, ground, strong),
            select_bg: hex(self.surface),
            match_bg: Palette::mix(self.yellow, ground, quiet),
            match_cur_bg: Palette::mix(self.yellow, ground, strong),
            syn: Syntax {
                comment: hex(self.dim),
                keyword: hex(self.magenta),
                string: hex(self.green),
                number: hex(self.orange),
                function: hex(self.blue),
                type_: hex(self.yellow),
                property: hex(self.cyan),
                operator: hex(self.dim),
                variable: hex(self.fg),
                builtin: hex(self.red),
                param: hex(self.orange),
                attribute: hex(self.cyan),
            },
        }
    }
}

/// The built-in truecolor palettes, as each project publishes them.
const PALETTES: &[Palette] = &[
    Palette {
        name: "catppuccin-mocha",
        fg: 0xcdd6f4,
        dim: 0x6c7086,
        surface: 0x313244,
        red: 0xf38ba8,
        green: 0xa6e3a1,
        yellow: 0xf9e2af,
        blue: 0x89b4fa,
        magenta: 0xcba6f7,
        cyan: 0x94e2d5,
        orange: 0xfab387,
        light: false,
    },
    Palette {
        name: "catppuccin-macchiato",
        fg: 0xcad3f5,
        dim: 0x6e738d,
        surface: 0x363a4f,
        red: 0xed8796,
        green: 0xa6da95,
        yellow: 0xeed49f,
        blue: 0x8aadf4,
        magenta: 0xc6a0f6,
        cyan: 0x8bd5ca,
        orange: 0xf5a97f,
        light: false,
    },
    Palette {
        name: "catppuccin-frappe",
        fg: 0xc6d0f5,
        dim: 0x737994,
        surface: 0x414559,
        red: 0xe78284,
        green: 0xa6d189,
        yellow: 0xe5c890,
        blue: 0x8caaee,
        magenta: 0xca9ee6,
        cyan: 0x81c8be,
        orange: 0xef9f76,
        light: false,
    },
    Palette {
        name: "catppuccin-latte",
        fg: 0x4c4f69,
        dim: 0x8c8fa1,
        surface: 0xccd0da,
        red: 0xd20f39,
        green: 0x40a02b,
        yellow: 0xdf8e1d,
        blue: 0x1e66f5,
        magenta: 0x8839ef,
        cyan: 0x179299,
        orange: 0xfe640b,
        light: true,
    },
    Palette {
        name: "tokyonight-night",
        fg: 0xc0caf5,
        dim: 0x565f89,
        surface: 0x292e42,
        red: 0xf7768e,
        green: 0x9ece6a,
        yellow: 0xe0af68,
        blue: 0x7aa2f7,
        magenta: 0xbb9af7,
        cyan: 0x7dcfff,
        orange: 0xff9e64,
        light: false,
    },
    Palette {
        name: "tokyonight-storm",
        fg: 0xc0caf5,
        dim: 0x565f89,
        surface: 0x2f334d,
        red: 0xf7768e,
        green: 0x9ece6a,
        yellow: 0xe0af68,
        blue: 0x7aa2f7,
        magenta: 0xbb9af7,
        cyan: 0x7dcfff,
        orange: 0xff9e64,
        light: false,
    },
    Palette {
        name: "tokyonight-moon",
        fg: 0xc8d3f5,
        dim: 0x636da6,
        surface: 0x2f334d,
        red: 0xff757f,
        green: 0xc3e88d,
        yellow: 0xffc777,
        blue: 0x82aaff,
        magenta: 0xc099ff,
        cyan: 0x86e1fc,
        orange: 0xff966c,
        light: false,
    },
    Palette {
        name: "tokyonight-day",
        fg: 0x3760bf,
        dim: 0x848cb5,
        surface: 0xc4c8da,
        red: 0xf52a65,
        green: 0x587539,
        yellow: 0x8c6c3e,
        blue: 0x2e7de9,
        magenta: 0x9854f1,
        cyan: 0x007197,
        orange: 0xb15c00,
        light: true,
    },
    Palette {
        name: "gruvbox-dark",
        fg: 0xebdbb2,
        dim: 0x928374,
        surface: 0x3c3836,
        red: 0xfb4934,
        green: 0xb8bb26,
        yellow: 0xfabd2f,
        blue: 0x83a598,
        magenta: 0xd3869b,
        cyan: 0x8ec07c,
        orange: 0xfe8019,
        light: false,
    },
    Palette {
        name: "gruvbox-light",
        fg: 0x3c3836,
        dim: 0x7c6f64,
        surface: 0xebdbb2,
        red: 0x9d0006,
        green: 0x79740e,
        yellow: 0xb57614,
        blue: 0x076678,
        magenta: 0x8f3f71,
        cyan: 0x427b58,
        orange: 0xaf3a03,
        light: true,
    },
    Palette {
        name: "nord",
        fg: 0xd8dee9,
        dim: 0x4c566a,
        surface: 0x3b4252,
        red: 0xbf616a,
        green: 0xa3be8c,
        yellow: 0xebcb8b,
        blue: 0x81a1c1,
        magenta: 0xb48ead,
        cyan: 0x88c0d0,
        orange: 0xd08770,
        light: false,
    },
    Palette {
        name: "dracula",
        fg: 0xf8f8f2,
        dim: 0x6272a4,
        surface: 0x44475a,
        red: 0xff5555,
        green: 0x50fa7b,
        yellow: 0xf1fa8c,
        blue: 0xbd93f9,
        magenta: 0xff79c6,
        cyan: 0x8be9fd,
        orange: 0xffb86c,
        light: false,
    },
    Palette {
        name: "solarized-dark",
        fg: 0x93a1a1,
        dim: 0x586e75,
        surface: 0x073642,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        magenta: 0xd33682,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
        light: false,
    },
    Palette {
        name: "solarized-light",
        fg: 0x586e75,
        dim: 0x93a1a1,
        surface: 0xeee8d5,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        magenta: 0xd33682,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
        light: true,
    },
];

/// Every theme name, terminal ones first — the order `--theme` reports, and
/// the order `:config` cycles in.
pub(super) fn theme_names() -> Vec<String> {
    ["dark".to_string(), "light".to_string()]
        .into_iter()
        .chain(PALETTES.iter().map(|p| p.name.to_string()))
        .collect()
}

pub(super) fn theme(name: &str) -> Option<Theme> {
    match name {
        "dark" => Some(Theme::terminal("dark", false)),
        "light" => Some(Theme::terminal("light", true)),
        n => PALETTES.iter().find(|p| p.name == n).map(Palette::theme),
    }
}

const BAR: &str = "▎";

// Re-style the char range [start, end) of the whole line's rendered spans
// (prefix included), splitting spans at the boundaries as needed. `style_fn`
// maps a span's existing style to its overlaid one, so the caller decides
// whether to tint a background, reverse it, etc.
fn overlay_range(
    spans: Vec<Span<'static>>,
    start: usize,
    end: usize,
    style_fn: impl Fn(Style) -> Style,
) -> Vec<Span<'static>> {
    if start >= end {
        return spans;
    }
    let mut consumed = 0;
    let mut out = Vec::with_capacity(spans.len() + 2);
    for sp in spans {
        let text = sp.content.to_string();
        let len = text.chars().count();
        let (seg_start, seg_end) = (consumed, consumed + len);
        if end <= seg_start || start >= seg_end {
            out.push(sp);
        } else {
            let local_start = start.saturating_sub(seg_start).min(len);
            let local_end = end.saturating_sub(seg_start).min(len);
            let mut chars = text.chars();
            let before: String = chars.by_ref().take(local_start).collect();
            let mid: String = chars.by_ref().take(local_end - local_start).collect();
            let after: String = chars.collect();
            if !before.is_empty() {
                out.push(Span::styled(before, sp.style));
            }
            if !mid.is_empty() {
                out.push(Span::styled(mid, style_fn(sp.style)));
            }
            if !after.is_empty() {
                out.push(Span::styled(after, sp.style));
            }
        }
        consumed += len;
    }
    out
}

// Crop `spans` to the char range [start, start + width) — the horizontal-
// scroll counterpart of `overlay_range`'s boundary splitting: same walk over
// char-counted spans, but dropping what falls outside the window instead of
// restyling what falls inside it.
pub(super) fn slice_range(
    spans: Vec<Span<'static>>,
    start: usize,
    width: usize,
) -> Vec<Span<'static>> {
    let end = start + width;
    if start >= end {
        return vec![];
    }
    let mut consumed = 0;
    let mut out = Vec::with_capacity(spans.len());
    for sp in spans {
        let text = sp.content.to_string();
        let len = text.chars().count();
        let (seg_start, seg_end) = (consumed, consumed + len);
        consumed += len;
        if end <= seg_start || start >= seg_end {
            continue;
        }
        let local_start = start.saturating_sub(seg_start).min(len);
        let local_end = end.saturating_sub(seg_start).min(len);
        if local_end > local_start {
            let mid: String = text
                .chars()
                .skip(local_start)
                .take(local_end - local_start)
                .collect();
            out.push(Span::styled(mid, sp.style));
        }
    }
    out
}

// Reverse-style the char at `target` (a char index into the whole line's
// rendered spans, prefix included) — the cursor cell. Past the last rendered
// char (an empty line, or a column beyond it) it appends one blank reversed
// cell so the cursor is still visible.
fn overlay_cursor(spans: Vec<Span<'static>>, target: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if target >= total {
        let mut out = spans;
        out.push(Span::styled(
            " ",
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        return out;
    }
    overlay_range(spans, target, target + 1, |s| {
        s.add_modifier(Modifier::REVERSED)
    })
}

// prefix width: 1-char sign bar + 4-digit line number + 1 space
pub(super) const GUTTER_W: usize = 1 + 5;
/// gutter glyph on a row `uses_at` names
pub(super) const MARK: &str = "▸";

// The whole new file with the changed hunk highlighted in place: removed lines
// on a red-tinted row (shown at the change point), added lines on a green tint,
// the rest plain context. Code is syntax-highlighted via tree-sitter.
#[allow(clippy::too_many_arguments)]
pub(super) fn code_view(
    it: &Item,
    sources: &Sources,
    highlights: &Highlights,
    width: usize,
    hscroll: usize,
    cursor: Option<Cursor>,
    matches: &[(usize, usize, usize)],
    cur_match: Option<usize>,
    theme: &Theme,
    start: usize,
    rows: usize,
) -> (Vec<Line<'static>>, bool, usize) {
    let mut out = vec![];
    let Some((ol, nl)) = sources.get(&it.path) else {
        return (out, false, 0);
    };
    let hl = highlights.get(&it.path);
    let [o0, o1] = it.old_range;
    let [n0, n1] = it.new_range;
    let removed: Vec<&String> = if o0 >= 1 && o0 <= o1 && o1 <= ol.len() {
        ol[o0 - 1..o1].iter().collect()
    } else {
        vec![]
    };
    // the rows `uses_at` names, marked in the gutter's last column so the
    // reviewer reads positions in the code rather than as prose in the why pane
    let use_rows: HashSet<usize> = it
        .uses_at
        .iter()
        .flat_map(|u| u.rows.iter().copied())
        .collect();
    let avail = width.saturating_sub(GUTTER_W);
    // Every row this view would hold, counted rather than built: the pane shows
    // `rows` of them, so building the whole file to throw all but a screenful
    // away costs ~20 allocations per line of a file that can run to thousands.
    // The removed block lands at `n0` (or after the last line when the deletion
    // sits at EOF); `n0 == 0` is a change before line 1, where it is not shown.
    let total = nl.len() + if n0 >= 1 { removed.len() } else { 0 };
    let start = start.min(last_line(total) as usize);
    let end = start.saturating_add(rows);
    // Clipping is a property of the whole view, not of the rows on screen: the
    // `›` marker would otherwise blink on and off as the reviewer scrolls past
    // a long line. Counted over every line — no allocation, and `any` stops at
    // the first one wide enough.
    let over = |l: &str| l.chars().count() > hscroll + avail;
    let right_clip = nl.iter().any(|l| over(l)) || (n0 >= 1 && removed.iter().any(|r| over(r)));
    let painter = RowPainter {
        theme,
        width,
        hscroll,
        avail,
    };
    // the removed block's rows: counted past the window, painted inside it
    let emit_removed = |out: &mut Vec<Line<'static>>, row: &mut usize| {
        for (k, r) in removed.iter().enumerate() {
            let here = *row;
            *row += 1;
            if here < start || here >= end {
                continue;
            }
            let refined = it.refined.removed.get(k).and_then(|s| s.as_ref());
            out.push(painter.removed(r, refined));
        }
    };
    let mut row = 0usize;
    for (i, line) in nl.iter().enumerate() {
        let ln = i + 1;
        if ln == n0 {
            emit_removed(&mut out, &mut row);
        }
        let here = row;
        row += 1;
        if here < start {
            continue;
        }
        if here >= end {
            break;
        }
        let added = n0 <= ln && ln <= n1;
        out.push(painter.source(&SourceRow {
            i,
            ln,
            line,
            added,
            use_row: use_rows.contains(&ln),
            hl: hl.and_then(|h| h.get(i)),
            refined: if added {
                it.refined.added.get(ln - n0).and_then(|s| s.as_ref())
            } else {
                None
            },
            matches,
            cur_match,
            cursor,
        }));
    }
    if n0 > nl.len() {
        emit_removed(&mut out, &mut row); // deletion at/after EOF
    }
    (out, right_clip, total)
}

/// What every row of the code pane is painted with: the theme, the
/// horizontal window, and the pane's width.
struct RowPainter<'a> {
    theme: &'a Theme,
    width: usize,
    hscroll: usize,
    /// columns left for code once the gutter has its share
    avail: usize,
}

/// One line of the new side, with what the painter needs to know about it.
struct SourceRow<'a> {
    /// 0-based index into the new side
    i: usize,
    /// 1-based line number
    ln: usize,
    line: &'a str,
    /// inside the hunk's new range
    added: bool,
    /// a row `uses_at` names
    use_row: bool,
    hl: Option<&'a LineSpans>,
    /// the columns that differ from the paired old line, when added
    refined: Option<&'a Vec<(usize, usize)>>,
    matches: &'a [(usize, usize, usize)],
    cur_match: Option<usize>,
    cursor: Option<Cursor>,
}

impl RowPainter<'_> {
    fn num(&self) -> Style {
        Style::default().fg(self.theme.dim)
    }

    // slice `content` (the code portion only — never the gutter built ahead
    // of it) to the horizontally visible window, with how many columns of it
    // are shown
    fn window(&self, content: Vec<Span<'static>>, len: usize) -> (Vec<Span<'static>>, usize) {
        let shown = len.saturating_sub(self.hscroll).min(self.avail);
        (slice_range(content, self.hscroll, self.avail), shown)
    }

    // fill the rest of the row so the background tint spans the full width
    fn pad(&self, spans: &mut Vec<Span<'static>>, used: usize, bg: Color) {
        if self.width > used {
            spans.push(Span::styled(
                " ".repeat(self.width - used),
                Style::default().bg(bg),
            ));
        }
    }

    // a line paired with its counterpart (see `ordo::refine`) keeps the quiet
    // tint and gets the strong one only where it actually differs; an unpaired
    // line has no counterpart to compare against and tints whole
    fn emphasize(
        &self,
        mut spans: Vec<Span<'static>>,
        refined: Option<&Vec<(usize, usize)>>,
        bg: Color,
    ) -> Vec<Span<'static>> {
        for &(cs, ce) in refined.map(|v| v.as_slice()).unwrap_or(&[]) {
            if ce <= self.hscroll || cs >= self.hscroll + self.avail {
                continue;
            }
            let (ls, le) = (
                cs.saturating_sub(self.hscroll),
                (ce - self.hscroll).min(self.avail),
            );
            spans = overlay_range(spans, GUTTER_W + ls, GUTTER_W + le, |st| st.bg(bg));
        }
        spans
    }

    fn removed(&self, text: &str, refined: Option<&Vec<(usize, usize)>>) -> Line<'static> {
        let theme = self.theme;
        let content = vec![Span::styled(
            text.to_string(),
            Style::default().fg(theme.del_fg).bg(theme.del_bg),
        )];
        let (visible, shown) = self.window(content, text.chars().count());
        let mut spans = vec![
            Span::styled(BAR, Style::default().fg(theme.del_fg)),
            Span::styled("     ".to_string(), self.num().bg(theme.del_bg)),
        ];
        spans.extend(visible);
        self.pad(&mut spans, GUTTER_W + shown, theme.del_bg);
        Line::from(self.emphasize(spans, refined, theme.del_strong_bg))
    }

    fn source(&self, r: &SourceRow) -> Line<'static> {
        let theme = self.theme;
        let bg = if r.added { theme.add_bg } else { Color::Reset };
        let num = self.num();
        let mut spans = vec![
            Span::styled(
                if r.added { BAR } else { " " },
                Style::default().fg(theme.add_fg),
            ),
            Span::styled(format!("{:>4}", r.ln), num.bg(bg)),
            if r.use_row {
                Span::styled(MARK, Style::default().fg(theme.accent).bg(bg))
            } else {
                Span::styled(" ".to_string(), num.bg(bg))
            },
        ];
        // syntax-colored code segments (fall back to the raw line if unhighlighted)
        let content: Vec<Span<'static>> = match r.hl {
            Some(segs) if !segs.is_empty() => segs
                .iter()
                .map(|(text, color)| Span::styled(text.clone(), Style::default().fg(*color).bg(bg)))
                .collect(),
            _ => vec![Span::styled(
                r.line.to_string(),
                Style::default().fg(theme.fg).bg(bg),
            )],
        };
        let (visible, shown) = self.window(content, r.line.chars().count());
        spans.extend(visible);
        if r.added {
            self.pad(&mut spans, GUTTER_W + shown, bg);
            spans = self.emphasize(spans, r.refined, theme.add_strong_bg);
        }
        for (mi, &(ml, s, e)) in r.matches.iter().enumerate() {
            // only a match that intersects the visible horizontal window can
            // be shown at all — one further off-screen is reached by jumping
            // to it (`n`/`N`, `*`/`#`), which scrolls the window to include it
            if ml != r.i || e <= self.hscroll || s >= self.hscroll + self.avail {
                continue;
            }
            let (ls, le) = (
                s.saturating_sub(self.hscroll),
                (e - self.hscroll).min(self.avail),
            );
            let mbg = if Some(mi) == r.cur_match {
                theme.match_cur_bg
            } else {
                theme.match_bg
            };
            spans = overlay_range(spans, GUTTER_W + ls, GUTTER_W + le, |st| st.bg(mbg));
        }
        let at_cursor = r.cursor.filter(|c| {
            c.line == r.i && c.col >= self.hscroll && c.col < self.hscroll + self.avail
        });
        if let Some(c) = at_cursor {
            spans = overlay_cursor(spans, GUTTER_W + (c.col - self.hscroll));
        }
        Line::from(spans)
    }
}

// ---------------------------------------------------------------------- hover

pub(super) fn node_text(n: Node, src: &str) -> String {
    src.get(n.start_byte()..n.end_byte())
        .unwrap_or("")
        .to_string()
}

// byte offset of a char column within one line — tree-sitter Points are byte-indexed
pub(super) fn char_byte(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

// node kinds counted as a "definition" worth showing, by file extension —
// deliberately a small, curated set rather than every grammar's declaration
// kinds, so a hover only ever lands on something with a clear signature/body.
pub(super) fn def_kinds(path: &str) -> &'static [&'static str] {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" | "xsh" | "xonsh" | "xonshrc" => &["function_definition", "class_definition"],
        "rs" => &[
            "function_item",
            "struct_item",
            "enum_item",
            "trait_item",
            "impl_item",
            "const_item",
            "static_item",
            "type_item",
        ],
        "js" | "jsx" | "mjs" | "cjs" => &[
            "function_declaration",
            "class_declaration",
            "method_definition",
        ],
        "ts" | "tsx" | "mts" | "cts" => &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
        ],
        "go" => &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
        ],
        "c" | "h" => &["function_definition", "struct_specifier", "enum_specifier"],
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => {
            &["function_definition", "class_specifier", "struct_specifier"]
        }
        "java" => &[
            "method_declaration",
            "class_declaration",
            "interface_declaration",
        ],
        _ => &[],
    }
}

// A definition's name — the `name` field where the grammar has one; C/C++
// function definitions don't (the identifier is buried in `declarator`), so
// fall back to hunting one down there, skipping the parameter list.
fn def_name(n: Node, src: &str) -> Option<String> {
    if let Some(name) = n.child_by_field_name("name") {
        return Some(node_text(name, src));
    }
    find_identifier(n.child_by_field_name("declarator")?, src)
}

fn find_identifier(n: Node, src: &str) -> Option<String> {
    if n.kind() == "identifier" || n.kind() == "field_identifier" {
        return Some(node_text(n, src));
    }
    let mut cursor = n.walk();
    let children: Vec<Node> = n.children(&mut cursor).collect();
    children
        .into_iter()
        .filter(|c| !c.kind().contains("parameter"))
        .find_map(|c| find_identifier(c, src))
}

// depth-first search of the whole tree for a definition-kind node named `name`
pub(super) fn find_definition<'a>(
    root: Node<'a>,
    kinds: &[&str],
    name: &str,
    src: &str,
) -> Option<Node<'a>> {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if kinds.contains(&n.kind()) && def_name(n, src).as_deref() == Some(name) {
            return Some(n);
        }
        let mut cursor = n.walk();
        stack.extend(n.children(&mut cursor));
    }
    None
}

// the def's header — its own text up to (not including) its body, so a
// multi-line signature still reads as just the signature
pub(super) fn signature(n: Node, src: &str) -> String {
    let end = n
        .child_by_field_name("body")
        .map(|b| b.start_byte())
        .unwrap_or(n.end_byte());
    src.get(n.start_byte()..end)
        .unwrap_or("")
        .trim_end()
        .to_string()
}

fn python_docstring(n: Node, src: &str) -> Option<String> {
    let body = n.child_by_field_name("body")?;
    let first = body.named_child(0)?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let expr = first.named_child(0)?;
    (expr.kind() == "string").then(|| node_text(expr, src))
}

// the run of `//` / `/** */` comment nodes immediately preceding the
// definition, stopping at the first blank line or non-comment sibling
fn leading_comment(n: Node, src: &str) -> Option<String> {
    let mut lines = vec![];
    let mut cur = n.prev_sibling();
    let mut expect_row = n.start_position().row;
    while let Some(c) = cur {
        if !c.kind().contains("comment") || c.end_position().row + 1 < expect_row {
            break;
        }
        lines.push(node_text(c, src).trim_end().to_string());
        expect_row = c.start_position().row;
        cur = c.prev_sibling();
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

pub(super) fn doc_for(path: &str, n: Node, src: &str) -> Option<String> {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" => python_docstring(n, src),
        _ => leading_comment(n, src),
    }
}

// Ensure `path`'s new-content is parsed and cached, returning the cached tree.
// Shared by `hover` and the symbol-occurrence search so both cache the same tree.
pub(super) fn parse_cached<'a>(
    trees: &'a mut HashMap<String, ParsedFile>,
    path: &str,
    nl: &[String],
    lang: tree_sitter::Language,
) -> Option<&'a ParsedFile> {
    if !trees.contains_key(path) {
        let src = nl.join("\n");
        let mut parser = Parser::new();
        parser.set_language(&lang).ok()?;
        let tree = parser.parse(&src, None)?;
        trees.insert(path.to_string(), ParsedFile { tree, src });
    }
    trees.get(path)
}

// The identifier node at `cursor`, widening from whatever node is directly
// under it until an `identifier`-kind ancestor is found.
pub(super) fn identifier_at<'a>(
    parsed: &'a ParsedFile,
    nl: &[String],
    cursor: Cursor,
) -> Option<Node<'a>> {
    let line = nl.get(cursor.line)?;
    let col = char_byte(line, cursor.col);
    let point = Point {
        row: cursor.line,
        column: col,
    };
    if let Some(mut node) = parsed
        .tree
        .root_node()
        .descendant_for_point_range(point, point)
    {
        loop {
            if node.kind().contains("identifier") {
                return Some(node);
            }
            match node.parent() {
                // stop widening at the line: an identifier further up the tree
                // begins somewhere else entirely
                Some(p) if p.start_position().row == cursor.line => node = p,
                _ => break,
            }
        }
    }
    first_identifier_on_line(parsed, cursor.line, col)
}

/// The definition this identifier names a *parameter* of, if it does. Looks up
/// from the identifier to the enclosing definition and checks that definition's
/// own parameter list — so a parameter reads the same whether the cursor is on
/// its declaration or on a use of it further down the body. A parameter has no
/// definition to find, and reporting "not defined in this file" sends the
/// reviewer looking through other files for something that was never there.
fn parameter_owner(node: Node, kinds: &[&str], src: &str) -> Option<String> {
    let name = node_text(node, src);
    let mut cur = node;
    loop {
        cur = cur.parent()?;
        if !kinds.contains(&cur.kind()) {
            continue;
        }
        let params = cur.child_by_field_name("parameters")?;
        if !subtree_names(params, src).contains(&name) {
            return None; // the enclosing definition binds it some other way
        }
        let owner = cur
            .child_by_field_name("name")
            .or_else(|| cur.child_by_field_name("declarator"))?;
        return Some(node_text(owner, src));
    }
}

/// Every identifier text inside a subtree.
fn subtree_names(node: Node, src: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind().contains("identifier") {
            out.push(node_text(n, src));
        }
        stack.extend(n.named_children(&mut cur));
    }
    out
}

/// The leftmost identifier node beginning at or after `col` on `row`.
fn first_identifier_on_line<'a>(
    parsed: &'a ParsedFile,
    row: usize,
    col: usize,
) -> Option<Node<'a>> {
    let mut cur = parsed.tree.walk();
    let mut stack = vec![parsed.tree.root_node()];
    let mut best: Option<Node<'a>> = None;
    while let Some(n) = stack.pop() {
        let (s, e) = (n.start_position(), n.end_position());
        if s.row > row || e.row < row {
            continue;
        }
        if n.kind().contains("identifier") && s.row == row && s.column >= col {
            if best.is_none_or(|b| s.column < b.start_position().column) {
                best = Some(n);
            }
            continue;
        }
        stack.extend(n.named_children(&mut cur));
    }
    best
}

/// `K` / `F12` — resolve the symbol under the cursor via tree-sitter (not by
/// scanning characters) and show its definition in a popup. Parses lazily,
/// caching the tree per path so repeated presses are cheap.
pub(super) fn hover(app: &mut App) {
    let path = app.items[app.sel].path.clone();
    let Some((_, nl)) = app.sources.get(&path) else {
        return;
    };
    if nl.is_empty() {
        return;
    }
    let Some((lang, _)) = highlight_spec(&path) else {
        app.popup = Some(Popup::new(
            "hover",
            vec![prose("no grammar available for this file type")],
        ));
        return;
    };
    let Some(parsed) = parse_cached(&mut app.trees, &path, nl, lang) else {
        return;
    };
    let Some(node) = identifier_at(parsed, nl, app.cursor) else {
        app.popup = Some(Popup::new("hover", vec![prose("no symbol here")]));
        return;
    };
    let name = node_text(node, &parsed.src);
    let root = parsed.tree.root_node();
    let kinds = def_kinds(&path);
    // `def_id` carries owned data (kind, row, the exact source the tree was
    // built from) out of this branch so the `app.trees` borrow behind `parsed`
    // ends here — `history_lines` below needs `&mut app`.
    let (mut lines, def_id) = if kinds.is_empty() {
        (
            vec!["definition lookup not supported for this file type".to_string()],
            None,
        )
    } else if let Some(def) = find_definition(root, kinds, &name, &parsed.src) {
        let mut lines = vec![format!("kind: {}", def.kind()), String::new()];
        lines.extend(signature(def, &parsed.src).lines().map(str::to_string));
        if let Some(doc) = doc_for(&path, def, &parsed.src) {
            lines.push(String::new());
            lines.extend(doc.lines().map(str::to_string));
        }
        let id = (
            def.kind().to_string(),
            def.start_position().row,
            parsed.src.clone(),
        );
        (lines, Some(id))
    } else if let Some(owner) = parameter_owner(node, kinds, &parsed.src) {
        // a parameter has no definition to find, and saying so as "not defined
        // in this file" points the reviewer at other files for no reason
        (vec![format!("parameter of {owner}")], None)
    } else {
        (vec!["not defined in this file".to_string()], None)
    };
    // history only makes sense for a symbol whose definition resolved in the
    // current file — an unresolved lookup has nothing to look up history for
    if let Some((kind, row, content)) = def_id {
        lines.push(String::new());
        lines.extend(history_lines(app, &path, &name, &kind, row, &content));
    }
    let lines = lines.into_iter().map(prose).collect();
    app.popup = Some(Popup::new(name, lines));
}

/// Declares the theme roles a config file may set, each the name of a `Theme`
/// field, generating the write side (`apply_theme_colors`) and read side
/// (`theme_role_color`) from one list — so a role can't drift between the
/// two, which is how "match-bg" once read back the wrong field.
macro_rules! theme_roles {
    ($($role:literal => $($seg:ident).+),* $(,)?) => {
        pub(super) const THEME_ROLES: &[&str] = &[$($role),*];

        /// Overlay a config's colour overrides onto a theme. Unknown roles are
        /// rejected at parse time, so everything reaching here names a field.
        pub(super) fn apply_theme_colors(mut t: Theme, colors: &[(String, Color)]) -> Theme {
            for (role, c) in colors {
                match role.as_str() {
                    $($role => t.$($seg).+ = *c,)*
                    _ => {}
                }
            }
            t
        }

        /// A theme role's current colour, by the name a config file uses. The
        /// read side of `apply_theme_colors`, so `--init-config` prints what
        /// the program would actually read back.
        pub(super) fn theme_role_color(t: &Theme, role: &str) -> Color {
            match role {
                $($role => t.$($seg).+,)*
                _ => Color::Reset,
            }
        }
    };
}

theme_roles! {
    "fg" => fg,
    "dim" => dim,
    "border" => border,
    "border-focus" => border_focus,
    "accent" => accent,
    "category" => category,
    "mark" => mark,
    "reviewed" => reviewed,
    "warn" => warn,
    "add-fg" => add_fg,
    "del-fg" => del_fg,
    "add-bg" => add_bg,
    "del-bg" => del_bg,
    "add-strong-bg" => add_strong_bg,
    "del-strong-bg" => del_strong_bg,
    "select-bg" => select_bg,
    "match-bg" => match_bg,
    "match-current-bg" => match_cur_bg,
    "syntax-comment" => syn.comment,
    "syntax-keyword" => syn.keyword,
    "syntax-string" => syn.string,
    "syntax-number" => syn.number,
    "syntax-function" => syn.function,
    "syntax-type" => syn.type_,
    "syntax-property" => syn.property,
    "syntax-operator" => syn.operator,
    "syntax-variable" => syn.variable,
    "syntax-builtin" => syn.builtin,
    "syntax-parameter" => syn.param,
    "syntax-attribute" => syn.attribute,
}
