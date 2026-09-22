// ----------------------------------------------------------------------- keys
use crate::code_view::hex;
use crate::code_view::THEME_ROLES;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::style::Color;

/// The three panes, in focus-cycle order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Pane {
    List,
    Code,
    Why,
    /// the dependency canvas — only reachable while it is open, so the
    /// `Tab` cycle below leaves it out
    Deps,
}

impl Pane {
    pub(super) fn next(self) -> Pane {
        match self {
            Pane::List => Pane::Code,
            Pane::Code => Pane::Why,
            // the canvas is entered by `gD` or `4`, never by cycling into it
            Pane::Why | Pane::Deps => Pane::List,
        }
    }
    pub(super) fn prev(self) -> Pane {
        match self {
            Pane::List => Pane::Why,
            Pane::Code => Pane::List,
            Pane::Why | Pane::Deps => Pane::Code,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Action {
    Quit,
    Next,
    Prev,
    First,
    Last,
    ToggleReviewed,
    PageDown,
    PageUp,
    HalfDown,
    HalfUp,
    FocusNext,
    /// fill the frame with the focused pane, or restore the split
    Zoom,
    /// open the dependency canvas on the selected hunk
    OpenDeps,
    FocusPrev,
    Focus(Pane),
    // code-pane cursor motions
    CursorLeft,
    CursorRight,
    WordNext,
    WordPrev,
    WordEnd,
    /// column line-end motion (vim 0/$, vscode Home/End); pane-dependent, see
    /// `apply` — falls back to `First`/`Last`'s meaning outside the code pane
    LineStart,
    LineEnd,
    ParaPrev,
    ParaNext,
    MarkPrev,
    MarkNext,
    /// `H` — how often these lines have changed before, and who last touched
    /// them. Explicit because it shells out to `git log -L` (see `hunk_churn`)
    Churn,
    /// symbol under the cursor, in a floating popup
    Hover,
    /// `/` — open the text-search prompt
    SearchOpen,
    /// `*` / `#` — jump to the next/previous tree-sitter occurrence of the
    /// symbol under the cursor
    SymbolNext,
    SymbolPrev,
    /// `n` / `N` — cycle the currently active search (text or symbol)
    SearchNext,
    SearchPrev,
    /// open the selected hunk's file in `$VISUAL`/`$EDITOR`, positioned at its
    /// line — handled specially in `run` (it needs to suspend the terminal)
    OpenEditor,
    /// `?` / `F1` — open the generated keybinding-help popup
    Help,
    /// `:` (vim), `C-Shift-P` (vscode) — open the command bar empty
    CommandOpen,
    /// `C-P` (vscode's own "Go to File") — open the command bar pre-filled
    /// with `goto `
    CommandGoto,
    /// `Enter`/`gd` (vim), `C-Enter` (vscode) — jump to the why pane's current
    /// dep line's target hunk; a no-op outside the why pane, or when the
    /// current line isn't a dep line, or its target isn't part of this review
    JumpToEdge,
    /// `C-o` (vim) / `Alt-Left` (vscode) — pop the position stack `JumpToEdge`
    /// pushed, returning to where the jump was made from
    JumpBack,
    /// `za`/`zo`/`zc`/`zR`/`zM` (vim), `C-k C-l`/`C-k C-0`/`C-k C-j` (vscode) —
    /// fold the reading-order list by group
    Fold(Fold),
    /// `zh`/`zl` (vim), shift-left/shift-right (vscode) — scroll the code
    /// pane's horizontal window without moving the cursor
    ScrollLeft,
    ScrollRight,
}

/// What the reading-order list is a list *of*. The ledger is the default: a
/// hunk is an artifact of `diff`, a symbol is what a reviewer reasons about.
/// `:mode` switches. Both render through the same header/item machinery — the
/// mode only decides which key `Item::bucket` carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum ViewMode {
    #[default]
    Ledger,
    Hunks,
}

/// Which way a fold key moves the group under the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Fold {
    Toggle,
    Open,
    Close,
    OpenAll,
    CloseAll,
}

pub(super) type Key = (KeyCode, KeyModifiers);
/// An optional prefix key (for chords like `C-w C-w`), the key, and its action.
type Bind = (Option<Key>, Key, Action);

pub(super) fn ch(c: char) -> Key {
    (KeyCode::Char(c), KeyModifiers::NONE)
}
pub(super) fn ctrl(c: char) -> Key {
    (KeyCode::Char(c), KeyModifiers::CONTROL)
}
pub(super) fn plain(c: KeyCode) -> Key {
    (c, KeyModifiers::NONE)
}
fn ctrlk(c: KeyCode) -> Key {
    (c, KeyModifiers::CONTROL)
}

/// An uppercase char already carries its case, so SHIFT is dropped there to keep
/// the tables independent of how a terminal reports it.
pub(super) fn norm(code: KeyCode, mods: KeyModifiers) -> Key {
    match code {
        KeyCode::Char(_) => (code, mods.difference(KeyModifiers::SHIFT)),
        _ => (code, mods),
    }
}

#[derive(Clone)]
pub(super) struct Keymap {
    pub(super) name: &'static str,
    pub(super) hint: &'static str,
    pub(super) binds: Vec<Bind>,
}

pub(super) enum Resolve {
    Act(Action),
    /// The key opens a chord; hold it and wait for the next one.
    Pending,
    Miss,
}

impl Keymap {
    pub(super) fn resolve(&self, pending: Option<Key>, key: Key) -> Resolve {
        let find = |pre: Option<Key>| {
            self.binds
                .iter()
                .find(|(p, k, _)| *p == pre && *k == key)
                .map(|(_, _, a)| Resolve::Act(*a))
        };
        if let Some(p) = pending {
            return find(Some(p)).unwrap_or(Resolve::Miss);
        }
        if self.binds.iter().any(|(p, _, _)| *p == Some(key)) {
            return Resolve::Pending;
        }
        find(None).unwrap_or(Resolve::Miss)
    }
}

/// Every keymap `keymap` answers to. The list is the authority — a test asserts
/// it and the function agree, so `:config` can offer presets without a second
/// copy of the names.
pub(super) const KEYMAP_NAMES: &[&str] = &["vim", "vscode"];

pub(super) fn keymap(name: &str) -> Option<Keymap> {
    match name {
        "vim" => Some(Keymap {
            name: "vim",
            hint: "j/k move · h/l/w/b/e/0/$ cursor · zh/zl hscroll · K symbol/dep · / search · */# sym-occ · n/N cycle · gd/Enter jump-dep · C-o back · ge edit · : cmd · ? help · q quit",
            binds: vec![
                (None, ch('q'), Action::Quit),
                (None, plain(KeyCode::Esc), Action::Quit),
                (None, ch('?'), Action::Help),
                (None, ch(':'), Action::CommandOpen),
                (None, ch('j'), Action::Next),
                (None, plain(KeyCode::Down), Action::Next),
                (None, ch('k'), Action::Prev),
                (None, plain(KeyCode::Up), Action::Prev),
                (Some(ch('g')), ch('g'), Action::First),
                (None, ch('G'), Action::Last),
                (None, ch('x'), Action::ToggleReviewed),
                (None, ch(' '), Action::PageDown),
                (None, ch('f'), Action::PageDown),
                (None, ctrl('f'), Action::PageDown),
                (None, plain(KeyCode::PageDown), Action::PageDown),
                (None, ctrl('b'), Action::PageUp),
                (None, plain(KeyCode::PageUp), Action::PageUp),
                (None, ctrl('d'), Action::HalfDown),
                (None, ctrl('u'), Action::HalfUp),
                (Some(ctrl('w')), ctrl('w'), Action::FocusNext),
                (Some(ctrl('w')), ch('w'), Action::FocusNext),
                (Some(ctrl('w')), ch('W'), Action::FocusPrev),
                (Some(ctrl('w')), ch('p'), Action::FocusPrev),
                // plain digits, because Ctrl-1/2/3 below are unsendable by
                // xterm, gnome-terminal, Terminal.app and most tmux setups —
                // only kitty-protocol terminals emit them. Pressing the digit
                // of the pane already focused zooms it.
                (Some(ch('g')), ch('D'), Action::OpenDeps),
                (None, ch('4'), Action::Focus(Pane::Deps)),
                (None, ch('1'), Action::Focus(Pane::List)),
                (None, ch('2'), Action::Focus(Pane::Code)),
                (None, ch('3'), Action::Focus(Pane::Why)),
                (Some(ctrl('w')), ch('z'), Action::Zoom),
                (Some(ctrl('w')), ch('_'), Action::Zoom),
                (Some(ctrl('w')), ch('h'), Action::Focus(Pane::List)),
                (Some(ctrl('w')), ch('l'), Action::Focus(Pane::Code)),
                (Some(ctrl('w')), ch('k'), Action::Focus(Pane::Code)),
                (Some(ctrl('w')), ch('j'), Action::Focus(Pane::Why)),
                // vim takes arrows wherever it takes hjkl, and a C-w chord is
                // no exception
                (Some(ctrl('w')), plain(KeyCode::Left), Action::Focus(Pane::List)),
                (Some(ctrl('w')), plain(KeyCode::Right), Action::Focus(Pane::Code)),
                (Some(ctrl('w')), plain(KeyCode::Up), Action::Focus(Pane::Code)),
                (Some(ctrl('w')), plain(KeyCode::Down), Action::Focus(Pane::Why)),
                // code-pane cursor motions — `b` (word-back) displaces the old
                // bare-b page-up shortcut; C-b/PageUp still page.
                (None, ch('h'), Action::CursorLeft),
                (None, ch('l'), Action::CursorRight),
                (None, ch('w'), Action::WordNext),
                (None, ch('b'), Action::WordPrev),
                (None, ch('e'), Action::WordEnd),
                (None, ch('0'), Action::LineStart),
                (None, ch('$'), Action::LineEnd),
                (None, ch('{'), Action::ParaPrev),
                (None, ch('}'), Action::ParaNext),
                (None, ch('['), Action::MarkPrev),
                (None, ch(']'), Action::MarkNext),
                (None, ch('H'), Action::Churn),
                // `z` prefix (vim's own convention for view-scrolling
                // commands, e.g. zh/zl to scroll a `nowrap` window sideways)
                (Some(ch('z')), ch('h'), Action::ScrollLeft),
                (Some(ch('z')), ch('l'), Action::ScrollRight),
                (Some(ch('z')), ch('a'), Action::Fold(Fold::Toggle)),
                (Some(ch('z')), ch('o'), Action::Fold(Fold::Open)),
                (Some(ch('z')), ch('c'), Action::Fold(Fold::Close)),
                (Some(ch('z')), ch('R'), Action::Fold(Fold::OpenAll)),
                (Some(ch('z')), ch('M'), Action::Fold(Fold::CloseAll)),
                (None, ch('K'), Action::Hover),
                (None, ch('/'), Action::SearchOpen),
                (None, ch('*'), Action::SymbolNext),
                (None, ch('#'), Action::SymbolPrev),
                (None, ch('n'), Action::SearchNext),
                (None, ch('N'), Action::SearchPrev),
                // `ge` (not bare `e`, which is already word-end in the code
                // pane) — mnemonic "go edit", chorded off the same `g` prefix
                // as `gg`
                (Some(ch('g')), ch('e'), Action::OpenEditor),
                // `gd` ("go to definition"), off the same `g` prefix as
                // `gg`/`ge`; `Enter` is free in vim (toggling reviewed is `x`
                // here, not `Enter`/`Space` as in vscode) so it's bound too
                (Some(ch('g')), ch('d'), Action::JumpToEdge),
                (None, plain(KeyCode::Enter), Action::JumpToEdge),
                // vim's own jumplist key, and free here — `ge`/OpenEditor
                // took the `g` prefix's `e`, not `C-o`
                (None, ctrl('o'), Action::JumpBack),
            ],
        }),
        "vscode" => Some(Keymap {
            name: "vscode",
            hint: "↑/↓ move · ←/→/C-←/C-→ cursor · S-←/S-→ hscroll · F12 symbol/dep · C-f search · F3 next · C-Enter jump-dep · A-← back · C-o edit · C-Shift-P cmd · F1 help · C-q quit",
            binds: vec![
                (None, ctrl('q'), Action::Quit),
                (None, plain(KeyCode::Esc), Action::Quit),
                // F1 is vscode's own "show command palette / help" key;
                // Ctrl+Shift+P (its other command-palette binding) is a
                // three-key chord many terminals don't report cleanly, so F1
                // alone is the more reliable analog for "show me the keys"
                (None, plain(KeyCode::F(1)), Action::Help),
                // vscode's own command-palette key; Ctrl+P ("Go to File") is
                // free here (list navigation is Up/Down, not C-p/C-n) and
                // maps to the closest analog this reviewer has: `:goto`
                (None, ctrl('P'), Action::CommandOpen),
                (None, ctrl('p'), Action::CommandGoto),
                (None, plain(KeyCode::Down), Action::Next),
                (None, plain(KeyCode::Up), Action::Prev),
                (None, ch(' '), Action::ToggleReviewed),
                (None, plain(KeyCode::Enter), Action::ToggleReviewed),
                (None, plain(KeyCode::PageDown), Action::PageDown),
                (None, plain(KeyCode::PageUp), Action::PageUp),
                (None, plain(KeyCode::F(6)), Action::FocusNext),
                (
                    None,
                    (KeyCode::F(6), KeyModifiers::SHIFT),
                    Action::FocusPrev,
                ),
                (None, ctrl('1'), Action::Focus(Pane::List)),
                (None, ctrl('2'), Action::Focus(Pane::Code)),
                (None, ctrl('3'), Action::Focus(Pane::Why)),
                // code-pane cursor motions — Home/End move column-wise here
                // (list navigation keeps `LineStart`/`LineEnd`'s list fallback).
                (None, plain(KeyCode::Left), Action::CursorLeft),
                (None, plain(KeyCode::Right), Action::CursorRight),
                (None, ctrlk(KeyCode::Right), Action::WordNext),
                (None, ctrlk(KeyCode::Left), Action::WordPrev),
                (None, plain(KeyCode::Home), Action::LineStart),
                (None, plain(KeyCode::End), Action::LineEnd),
                // shift-left/right is otherwise unused here (no text
                // selection in this reviewer), so it's free for the
                // horizontal-scroll pair vscode has no dedicated key for
                (None, (KeyCode::Left, KeyModifiers::SHIFT), Action::ScrollLeft),
                (None, (KeyCode::Right, KeyModifiers::SHIFT), Action::ScrollRight),
                // vscode's own folding chords. `Ctrl+Shift+[` / `]` (its
                // fold/unfold pair) can't be told apart from Esc by a terminal,
                // so the `Ctrl+K` chords — which vscode also ships — are used.
                (Some(ctrl('k')), ctrl('l'), Action::Fold(Fold::Toggle)),
                (Some(ctrl('k')), ctrl('0'), Action::Fold(Fold::CloseAll)),
                (Some(ctrl('k')), ctrl('j'), Action::Fold(Fold::OpenAll)),
                (None, plain(KeyCode::F(12)), Action::Hover),
                (None, ctrl('f'), Action::SearchOpen),
                (None, plain(KeyCode::F(3)), Action::SearchNext),
                (
                    None,
                    (KeyCode::F(3), KeyModifiers::SHIFT),
                    Action::SearchPrev,
                ),
                // F12 is already `Hover` (def signature); Ctrl/Shift+Ctrl+F12
                // is the closest free analog to vscode's own references keys
                // (Shift+F12 "Go to References") for cycling occurrences.
                (None, ctrlk(KeyCode::F(12)), Action::SymbolNext),
                (
                    None,
                    (KeyCode::F(12), KeyModifiers::CONTROL | KeyModifiers::SHIFT),
                    Action::SymbolPrev,
                ),
                // vscode's own default binding for "File: Open File" —
                // reused here for "open this location in the editor"
                (None, ctrl('o'), Action::OpenEditor),
                // `Enter` is already `ToggleReviewed` here (vscode's own
                // "activate" key); vscode's actual "Go to Definition" (F12)
                // is likewise already `Hover`, so `C-Enter` (free, and reads
                // as "Enter, but do more") is the jump-to-dep key instead
                (None, (KeyCode::Enter, KeyModifiers::CONTROL), Action::JumpToEdge),
                // vscode's own default "Go Back" binding
                (None, (KeyCode::Left, KeyModifiers::ALT), Action::JumpBack),
            ],
        }),
        _ => None,
    }
}

// ------------------------------------------------------------------ key help

/// Grouping for the generated `?`/`F1` help popup. `General` isn't among the
/// project owner's suggested categories (navigation/panes/search/review/
/// editor/help) but earns its own row for `Quit`, which fits none of them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Category {
    General,
    Navigation,
    Panes,
    Search,
    Review,
    Editor,
    Help,
}

impl Category {
    /// the order the help popup and the docs list the categories in
    pub(super) const ORDER: [Category; 7] = [
        Category::General,
        Category::Navigation,
        Category::Panes,
        Category::Search,
        Category::Review,
        Category::Editor,
        Category::Help,
    ];
}

pub(super) fn category_label(c: Category) -> &'static str {
    match c {
        Category::General => "general",
        Category::Navigation => "navigation",
        Category::Panes => "panes",
        Category::Search => "search",
        Category::Review => "review",
        Category::Editor => "editor",
        Category::Help => "help",
    }
}

/// Category + one-line description for an action — the only hand-written
/// prose in the help system. Everything else (which keys, in what preset)
/// comes straight from `Keymap.binds`, so this text can drift in *wording*
/// but never in *which key does what*.
pub(super) fn action_help(a: Action) -> (Category, &'static str) {
    match a {
        Action::Quit => (Category::General, "quit (or dismiss an open popup or search)"),
        Action::Next => (Category::Navigation, "next item / move cursor down"),
        Action::Prev => (Category::Navigation, "previous item / move cursor up"),
        Action::First => (Category::Navigation, "jump to the first item / top"),
        Action::Last => (Category::Navigation, "jump to the last item / bottom"),
        Action::ToggleReviewed => (Category::Review, "toggle reviewed on the selected hunk"),
        Action::PageDown => (Category::Navigation, "page down"),
        Action::PageUp => (Category::Navigation, "page up"),
        Action::HalfDown => (Category::Navigation, "half-page down"),
        Action::HalfUp => (Category::Navigation, "half-page up"),
        Action::Zoom => (Category::Panes, "fill the frame with the focused pane"),
        Action::OpenDeps => (
            Category::Panes,
            "open the dependency canvas: what this hunk needs, and what needs it",
        ),
        Action::FocusNext => (Category::Panes, "focus the next pane"),
        Action::FocusPrev => (Category::Panes, "focus the previous pane"),
        Action::Focus(Pane::List) => (
            Category::Panes,
            "focus the reading-order pane (again to zoom)",
        ),
        Action::Focus(Pane::Code) => (Category::Panes, "focus the code pane (again to zoom)"),
        Action::Focus(Pane::Why) => (Category::Panes, "focus the why pane"),
        Action::Focus(Pane::Deps) => (Category::Panes, "focus the dependency canvas, when open"),
        Action::CursorLeft => (Category::Navigation, "move the code cursor left"),
        Action::CursorRight => (Category::Navigation, "move the code cursor right"),
        Action::WordNext => (Category::Navigation, "move the code cursor to the next word"),
        Action::WordPrev => (Category::Navigation, "move the code cursor to the previous word"),
        Action::WordEnd => (Category::Navigation, "move the code cursor to the end of the word"),
        Action::LineStart => (Category::Navigation, "move to line start / pane top"),
        Action::LineEnd => (Category::Navigation, "move to line end / pane bottom"),
        Action::ParaPrev => (Category::Navigation, "jump to the previous blank line"),
        Action::ParaNext => (Category::Navigation, "jump to the next blank line"),
        Action::MarkPrev => (Category::Navigation, "jump to the previous use of a name added here"),
        Action::MarkNext => (Category::Navigation, "jump to the next use of a name added here"),
        Action::Churn => (
            Category::Review,
            "how often these lines changed before, and who touched them last",
        ),
        Action::Fold(Fold::Toggle) => (Category::Review, "fold/unfold the selected hunk's group"),
        Action::Fold(Fold::Open) => (Category::Review, "unfold the selected hunk's group"),
        Action::Fold(Fold::Close) => (Category::Review, "fold the selected hunk's group"),
        Action::Fold(Fold::OpenAll) => (Category::Review, "unfold every group"),
        Action::Fold(Fold::CloseAll) => (Category::Review, "fold every group"),
        Action::ScrollLeft => (
            Category::Navigation,
            "scroll the code pane (or an open popup) left",
        ),
        Action::ScrollRight => (
            Category::Navigation,
            "scroll the code pane (or an open popup) right",
        ),
        Action::Hover => (
            Category::Editor,
            "code pane: show the symbol under the cursor and its history · why pane: preview the current dep line's target",
        ),
        Action::SearchOpen => (Category::Search, "open the text-search prompt"),
        Action::SymbolNext => (Category::Search, "next occurrence of the symbol under the cursor"),
        Action::SymbolPrev => (Category::Search, "previous occurrence of the symbol under the cursor"),
        Action::SearchNext => (Category::Search, "cycle to the next match"),
        Action::SearchPrev => (Category::Search, "cycle to the previous match"),
        Action::OpenEditor => (Category::Editor, "open the selected hunk's file in $VISUAL/$EDITOR"),
        Action::JumpToEdge => (
            Category::Navigation,
            "why pane: jump to the current dep line's target hunk",
        ),
        Action::JumpBack => (Category::Navigation, "jump back to the position before the last dep jump"),
        Action::Help => (Category::Help, "show this keybinding help"),
        Action::CommandOpen => (
            Category::General,
            "open the command bar (`:help` lists every command)",
        ),
        Action::CommandGoto => (Category::General, "open the command bar pre-filled with `goto `"),
    }
}

/// Every action, by the name a config file calls it. The inverse of
/// `key_label`'s job: `key_label` renders a key for humans, this names an
/// action for them. One table, used to parse a config and to check it — a name
/// missing here simply cannot be bound, and the error says which names exist.
pub(super) const ACTION_NAMES: &[(&str, Action)] = &[
    ("quit", Action::Quit),
    ("next", Action::Next),
    ("prev", Action::Prev),
    ("first", Action::First),
    ("last", Action::Last),
    ("toggle-reviewed", Action::ToggleReviewed),
    ("page-down", Action::PageDown),
    ("page-up", Action::PageUp),
    ("half-down", Action::HalfDown),
    ("half-up", Action::HalfUp),
    ("focus-next", Action::FocusNext),
    ("focus-prev", Action::FocusPrev),
    ("focus-list", Action::Focus(Pane::List)),
    ("focus-code", Action::Focus(Pane::Code)),
    ("focus-why", Action::Focus(Pane::Why)),
    ("zoom", Action::Zoom),
    ("open-deps", Action::OpenDeps),
    ("focus-deps", Action::Focus(Pane::Deps)),
    ("cursor-left", Action::CursorLeft),
    ("cursor-right", Action::CursorRight),
    ("word-next", Action::WordNext),
    ("word-prev", Action::WordPrev),
    ("word-end", Action::WordEnd),
    ("line-start", Action::LineStart),
    ("line-end", Action::LineEnd),
    ("para-prev", Action::ParaPrev),
    ("para-next", Action::ParaNext),
    ("mark-prev", Action::MarkPrev),
    ("mark-next", Action::MarkNext),
    ("churn", Action::Churn),
    ("hover", Action::Hover),
    ("search", Action::SearchOpen),
    ("symbol-next", Action::SymbolNext),
    ("symbol-prev", Action::SymbolPrev),
    ("search-next", Action::SearchNext),
    ("search-prev", Action::SearchPrev),
    ("open-editor", Action::OpenEditor),
    ("help", Action::Help),
    ("command", Action::CommandOpen),
    ("command-goto", Action::CommandGoto),
    ("jump-to-edge", Action::JumpToEdge),
    ("jump-back", Action::JumpBack),
    ("fold-toggle", Action::Fold(Fold::Toggle)),
    ("fold-open", Action::Fold(Fold::Open)),
    ("fold-close", Action::Fold(Fold::Close)),
    ("fold-open-all", Action::Fold(Fold::OpenAll)),
    ("fold-close-all", Action::Fold(Fold::CloseAll)),
    ("scroll-left", Action::ScrollLeft),
    ("scroll-right", Action::ScrollRight),
];

pub(super) fn action_by_name(name: &str) -> Option<Action> {
    ACTION_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, a)| *a)
}

/// Parse one key as a config writes it: `j`, `C-w`, `S-F3`, `Esc`, `Space`.
/// The inverse of `key_label`, and checked against it by test.
pub(super) fn parse_key(text: &str) -> Option<Key> {
    let mut mods = KeyModifiers::NONE;
    let mut rest = text.trim();
    loop {
        let (m, tail) = match rest.split_at_checked(2) {
            Some(("C-", t)) => (KeyModifiers::CONTROL, t),
            Some(("S-", t)) => (KeyModifiers::SHIFT, t),
            Some(("A-", t)) => (KeyModifiers::ALT, t),
            _ => break,
        };
        // a lone "C-" with nothing after it names no key
        if tail.is_empty() {
            return None;
        }
        mods |= m;
        rest = tail;
    }
    let code = match rest {
        "Space" => KeyCode::Char(' '),
        "Esc" => KeyCode::Esc,
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Tab" => KeyCode::Tab,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        f if f.starts_with('F') && f.len() > 1 => KeyCode::F(f[1..].parse().ok()?),
        c if c.chars().count() == 1 => KeyCode::Char(c.chars().next()?),
        _ => return None,
    };
    Some((code, mods))
}

/// A binding as a config writes it: one key, or two separated by a space for a
/// chord (`g d`, `C-w l`, `z a`). Chords are written with the space so `zh` and
/// `z h` can't be confused — the former is not a key at all.
fn parse_bind_keys(text: &str) -> Option<(Option<Key>, Key)> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    match parts.as_slice() {
        [one] => Some((None, parse_key(one)?)),
        [prefix, key] => Some((Some(parse_key(prefix)?), parse_key(key)?)),
        _ => None,
    }
}

/// The user's keymap overrides, read from
/// `${XDG_CONFIG_HOME:-~/.config}/ordo/tui.toml`:
///
/// ```toml
/// preset = "vim"        # which built-in preset to start from
///
/// [binds]
/// "C-n" = "next"        # add or replace a binding
/// "g d" = "jump-to-edge"  # a chord: prefix, space, key
/// "x" = "none"          # remove a binding
/// ```
///
/// Deliberately a small hand-read subset rather than a TOML dependency: the
/// file has two shapes of line, and a parser for exactly those cannot drift
/// from what the docs promise. Anything it cannot read is reported by line
/// number and skipped — a typo costs one binding, never the session.
pub(super) struct KeyConfig {
    pub(super) preset: Option<String>,
    pub(super) binds: Vec<(Option<Key>, Key, Option<Action>)>,
    /// `[theme] name = "…"`, and any per-role `#rrggbb` overrides on top of it
    pub(super) theme: Option<String>,
    /// `docs_last` — whether a docs-only hunk sorts after the code
    pub(super) docs_last: Option<bool>,
    pub(super) colors: Vec<(String, Color)>,
    pub(super) problems: Vec<String>,
}

/// `#rrggbb` (or `rrggbb`) as a colour. Deliberately the only accepted form: a
/// theme file names colours the way every palette publishes them.
pub(super) fn parse_hex(text: &str) -> Option<Color> {
    let t = text.trim().trim_start_matches('#');
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex(u32::from_str_radix(t, 16).ok()?))
}

/// Which section of the config the reader is in.
#[derive(PartialEq, Eq)]
enum Section {
    Top,
    Binds,
    Theme,
}

pub(super) fn parse_key_config(text: &str) -> KeyConfig {
    let mut cfg = KeyConfig {
        preset: None,
        binds: vec![],
        theme: None,
        docs_last: None,
        colors: vec![],
        problems: vec![],
    };
    let mut section = Section::Top;
    for (n, raw) in text.lines().enumerate() {
        let line = strip_comment(raw);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(head) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = match head.trim() {
                "binds" => Section::Binds,
                "theme" => Section::Theme,
                other => {
                    cfg.problems
                        .push(format!("line {}: unknown section `[{other}]`", n + 1));
                    Section::Top
                }
            };
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            cfg.problems
                .push(format!("line {}: expected `key = value`", n + 1));
            continue;
        };
        let unquote = |s: &str| s.trim().trim_matches('"').trim_matches('\'').to_string();
        let (k, v) = (unquote(k), unquote(v));
        let problem = match section {
            Section::Top => cfg.top_setting(k, v),
            Section::Theme => cfg.theme_setting(k, v),
            Section::Binds => cfg.bind_setting(k, v),
        };
        if let Some(what) = problem {
            cfg.problems.push(format!("line {}: {what}", n + 1));
        }
    }
    cfg
}

impl KeyConfig {
    /// a `key = value` above any section; the complaint when it is not one
    fn top_setting(&mut self, k: String, v: String) -> Option<String> {
        match k.as_str() {
            "preset" => self.preset = Some(v),
            "docs_last" => match v.parse() {
                Ok(b) => self.docs_last = Some(b),
                Err(_) => return Some("`docs_last` wants true or false".to_string()),
            },
            // `theme` reads naturally at the top of the file as well as
            // inside `[theme]`, and a config is read, not just written
            "theme" => self.theme = Some(v),
            _ => return Some(format!("unknown setting `{k}`")),
        }
        None
    }

    /// a `[theme]` entry: the theme's name, or one role's colour override
    fn theme_setting(&mut self, k: String, v: String) -> Option<String> {
        if k == "name" {
            self.theme = Some(v);
        } else if !THEME_ROLES.contains(&k.as_str()) {
            return Some(format!("unknown theme role `{k}`"));
        } else if v.is_empty() {
            // an empty role follows the palette — what `:config` writes
            // back when a reviewer clears an override
        } else {
            match parse_hex(&v) {
                Some(c) => self.colors.push((k, c)),
                None => return Some(format!("`{v}` is not a #rrggbb colour")),
            }
        }
        None
    }

    /// a `[binds]` entry: a chord bound to an action, or to `none`
    fn bind_setting(&mut self, k: String, v: String) -> Option<String> {
        let Some((prefix, key)) = parse_bind_keys(&k) else {
            return Some(format!("`{k}` is not a key"));
        };
        if v == "none" {
            self.binds.push((prefix, key, None));
            return None;
        }
        match action_by_name(&v) {
            Some(a) => self.binds.push((prefix, key, Some(a))),
            None => return Some(format!("unknown action `{v}`")),
        }
        None
    }
}

/// Apply overrides to a preset: a bound key replaces whatever held it, and
/// `none` removes it. Order is the config's, so a file can be read top to
/// bottom to know what it did.
pub(super) fn apply_key_config(mut km: Keymap, cfg: &KeyConfig) -> Keymap {
    for (prefix, key, action) in &cfg.binds {
        km.binds.retain(|(p, k, _)| !(p == prefix && k == key));
        if let Some(a) = action {
            km.binds.push((*prefix, *key, *a));
        }
    }
    km
}

/// A config line without its trailing comment. `#` opens a comment only
/// *outside* quotes: a colour is written `"#89b4fa"`, and cutting at the first
/// `#` regardless would eat every palette value in the file.
pub(super) fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '#') => return &line[..i],
            _ => {}
        }
    }
    line
}

/// One key, rendered legibly: `C-`/`S-`/`A-` modifier prefixes, named special
/// keys, `F<n>` for function keys, the char itself otherwise.
pub(super) fn key_label(key: Key) -> String {
    let (code, mods) = key;
    let mut prefix = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        prefix.push_str("C-");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        prefix.push_str("S-");
    }
    if mods.contains(KeyModifiers::ALT) {
        prefix.push_str("A-");
    }
    let body = match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        other => format!("{other:?}"),
    };
    format!("{prefix}{body}")
}

/// A chord as the `[binds]` section spells it: prefix and key space-separated,
/// always — the form `parse_key_config` reads back.
pub(super) fn chord_setting(prefix: Option<Key>, key: Key) -> String {
    match prefix {
        Some(p) => format!("{} {}", key_label(p), key_label(key)),
        None => key_label(key),
    }
}

/// A bind's full chord: `gg`/`ge` (no space — vim's own convention for a
/// plain-char chord) vs. `C-w C-w` (spaced — either half carries a modifier
/// or a named key, and vim always writes those chords spaced).
pub(super) fn chord_label(prefix: Option<Key>, key: Key) -> String {
    let Some(p) = prefix else {
        return key_label(key);
    };
    let simple = |k: Key| matches!(k.0, KeyCode::Char(_)) && k.1 == KeyModifiers::NONE;
    if simple(p) && simple(key) {
        format!("{}{}", key_label(p), key_label(key))
    } else {
        format!("{} {}", key_label(p), key_label(key))
    }
}

/// The `?`/`F1` help popup body: every bind in `keys`, grouped by category and
/// collapsed onto one row per action (several keys can mean the same thing,
/// e.g. `j` and `Down` both `Next`) — generated straight from `Keymap.binds`
/// rather than hand-duplicated, so it cannot describe a key the table doesn't
/// actually bind.
pub(super) fn build_help(keys: &Keymap) -> Vec<String> {
    struct Row {
        category: Category,
        desc: &'static str,
        keys: Vec<String>,
    }
    let mut rows: Vec<Row> = vec![];
    for &(prefix, key, action) in &keys.binds {
        let (category, desc) = action_help(action);
        let label = chord_label(prefix, key);
        match rows
            .iter_mut()
            .find(|r| r.category == category && r.desc == desc)
        {
            Some(r) if !r.keys.contains(&label) => r.keys.push(label),
            Some(_) => {}
            None => rows.push(Row {
                category,
                desc,
                keys: vec![label],
            }),
        }
    }
    let mut out = vec![];
    for &cat in &Category::ORDER {
        let group: Vec<&Row> = rows.iter().filter(|r| r.category == cat).collect();
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(category_label(cat).to_uppercase());
        // At most three chords per row. The help is for discovery, and one
        // action with four aliases (`Space, f, C-f, PageDown`) widened the key
        // column for every other row in its section; the full set is in the
        // generated table in docs/tui.md.
        const SHOWN: usize = 3;
        let label = |r: &Row| {
            let mut l = r.keys[..r.keys.len().min(SHOWN)].join(", ");
            if r.keys.len() > SHOWN {
                l.push('…');
            }
            l
        };
        // the column is as wide as the widest chord in this section, not a
        // fixed 16 that the longest row overflowed and fell out of line with
        let w = group
            .iter()
            .map(|r| label(r).chars().count())
            .max()
            .unwrap_or(0);
        for r in group {
            out.push(format!("  {:<w$} {}", label(r), r.desc));
        }
    }
    out
}
