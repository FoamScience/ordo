//! ordo — a terminal reviewer that is a pure client of the ordo engine.
//! It owns git (shells out for a commit's blobs), calls `ordo::run`, and renders
//! the change in comprehension order: the full file with the changed hunk
//! highlighted in context, plus rationale, advisories and def→use edges. The
//! engine stays git-free; gated behind the `tui` feature so the default build
//! never pulls a UI stack.
use crate::code_view::apply_theme_colors;
use crate::keys::build_help;
use crate::keys::keymap;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::code_view::hover;
use crate::code_view::theme;
use crate::code_view::theme_names;
use crate::code_view::Syntax;
use crate::code_view::Theme;
use crate::commands::carry_across_reload;
use crate::commands::handle_command_key;
use crate::commands::open_command_bar;
use crate::commands::CommandOutcome;
use crate::config::config_commit_edit;
use crate::config::config_path;
use crate::config::config_toggle;
use crate::config::config_write;
use crate::config::move_config;
use crate::config::write_init_config;
use crate::config::ConfigUi;
use crate::draw::close_deps;
use crate::draw::draw;
use crate::draw::jump_back;
use crate::draw::jump_to_card;
use crate::draw::jump_to_edge;
use crate::draw::move_card;
use crate::draw::open_deps;
use crate::draw::preview_edge;
use crate::draw::set_geometry;
use crate::editor::open_editor;
use crate::editor::open_quickfix_editor;
use crate::findings::parse_lcov;
use crate::findings::place_coverage;
use crate::findings::place_findings;
use crate::findings::sarif_findings;
use crate::findings::Finding;
use crate::git::command_failures;
use crate::git::declared_generated;
use crate::git::gather;
use crate::git::gather_range;
use crate::git::gather_uncommitted;
use crate::git::gather_worktree_range;
use crate::git::git;
use crate::git::note_command_failure;
use crate::git::resolve;
use crate::git::review_commit_sha;
use crate::git::Target;
use crate::highlight::highlight_file;
use crate::highlight::Highlights;
use crate::history::churn_query;
use crate::history::file_churn;
use crate::history::hunk_churn;
use crate::history::Churn;
use crate::keys::apply_key_config;
use crate::keys::norm;
use crate::keys::parse_key_config;
use crate::keys::Action;
use crate::keys::Fold;
use crate::keys::Key;
use crate::keys::KeyConfig;
use crate::keys::Keymap;
use crate::keys::Pane;
use crate::keys::Resolve;
use crate::keys::ViewMode;
use crate::marks::compare_runs;
use crate::marks::load_marks;
use crate::marks::load_notes;
use crate::marks::load_snaps;
use crate::marks::mark_key;
use crate::marks::marks_file_path;
use crate::marks::note_key;
use crate::marks::note_key_named;
use crate::marks::notes_file_path;
use crate::marks::now_unix;
use crate::marks::prune_marks;
use crate::marks::runs_file_path;
use crate::marks::save_marks;
use crate::marks::save_notes;
use crate::marks::save_snaps;
use crate::marks::Delta;
use crate::rules::load_rules_report;
use crate::rules::RuleSet;
use crate::search::accept_search;
use crate::search::cancel_search;
use crate::search::cycle_search;
use crate::search::symbol_search;
use ordo::model::{Change, Input, Output, Symbol};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;
use tree_sitter::Tree;
mod code_view;
mod commands;
mod config;
mod draw;
mod editor;
mod findings;
mod git;
mod highlight;
mod history;
mod keys;
mod marks;
mod rules;
mod search;

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const PAGE: u16 = 15;

const USAGE: &str = "\
ordo — interactive review of a commit, ordered for comprehension.

usage:
  ordo [<rev>] [<glob>...] [--keys <preset>] [--theme <name>] [--rules <file>]...
       [--sarif <file>]... [--coverage <file>]... [--all] [--only-comments]
       [--no-catalog]
  ordo --init-config [--force]
  ordo help [<topic>]
  ordo --version

<rev> is any git commit-ish (a sha, HEAD~2, a tag), a commit range (main..branch,
or main...branch to diff from the merge base), or `zz` for the uncommitted area.
base..zz (or base...zz for the merge base of base and HEAD) reviews everything
done since base including uncommitted work — base's tree versus the worktree.
Default: HEAD. Its diff is ordered by def→use with rationale, structural notes,
advisories and PR-split clusters.

On a GitButler-managed repo <rev> also takes the CLI IDs `but status` prints: a
branch (by ID or name) reviews that branch's own commits, a commit ID (or a
change-ID prefix) reviews that commit.

`gD` on a hunk opens the dependency canvas: the hunk at top centre, everything
it needs fanning left and everything that needs it fanning right, each as a card
showing that hunk's own code. Enter goes to a card (C-o returns), Esc closes.
The canvas takes the next free pane digit while it is open, so `4` addresses it.

--no-catalog runs only your own rules. The built-in construct catalog is on by
default — it is what a reviewer gets with no configuration — and this turns it
off for one run. `catalog = false` in a rules file turns it off for good, and
`disable = [\"goto\"]` silences a single entry by name, the same gesture that
silences one of your own rules.

--sarif <file> reads analyzer results in SARIF 2.1.0 — what semgrep, CodeQL,
ruff, eslint, shellcheck and `clippy --message-format` all emit — and attaches
each finding to the hunk whose lines contain it, so they arrive in the reading
order instead of as a separate list. Repeatable. A finding on a line this change
did not touch is counted in `:audit` rather than shown.

--coverage <file> reads an lcov tracefile — what `cargo llvm-cov --lcov`,
`coverage.py lcov` and most language toolchains emit — and reports, per hunk,
how many of the executable lines it changed were never run. Repeatable. Only
lcov's `DA:` records count, so a blank line, a comment or a declaration is never
held against a hunk. This is the fact behind the engine's `code changed but no
test touched` note, which is a guess made from file names.

Generated and lock files (Cargo.lock, package-lock.json, vendor/, node_modules/,
.min.js, …) are skipped, as is anything .gitattributes marks `linguist-generated`
or `-diff`; --all keeps them all. Any <glob> after <rev> confines the
review to paths matching at least one of them — `*` crosses `/`, so `src/*` is
everything under src/ and `*.rs` matches at any depth. A glob prefixed `!` is
negative and excludes a path that matches it, e.g. `'src/*' '!tests/*'`; with
only negative globs given, everything except those is kept. `\\!literal` escapes
a leading bang for a path genuinely named that way. Matching is order-independent
and deliberately unlike .gitignore: any negative match excludes a path no matter
where it appears relative to the positives, and a later positive never
re-includes something a negative excluded. Quote them so the shell doesn't
expand them first.

--theme selects the palette (also read from $ORDO_TUI_THEME, default dark).
`dark` and `light` keep the terminal's own foreground colours and only tint the
diff backgrounds; the truecolor themes — catppuccin (mocha, macchiato, frappe,
latte), tokyonight (night, storm, moon, day), gruvbox (dark, light), nord,
dracula, solarized (dark, light) — name every colour themselves. `:config` lists
them and swaps live. No theme paints a window background, so terminal
transparency survives; what a theme does assume is a background of matching
lightness. Roles are overridable in tui.toml's [theme] section.

--only-comments limits the review to comment/docstring-only hunks (ordered
among themselves, as the engine's --only-comments does) — a starting point
only; `:only-comments` in command mode toggles it live, same as `--all`
and a path filter (see below).

--keys selects a keystroke preset (also read from $ORDO_TUI_KEYS, default vim).
The three panes — reading order, code, why — take focus one at a time; motion
keys act on the focused pane, paging always drives the code pane.

Command mode (`:` in vim, Ctrl+Shift+P in vscode) turns launch-time choices
into live controls: `:only-comments`, `:all`, `:filter <glob>` (empty clears,
same negative-glob syntax as the CLI), `:config` (every setting), `:strategy
<comprehension|defs-first|file>` (re-orders in place), `:group` (toggle group
headers in the reading-order list), `:goto <path>`, `:e <rev>` (review a
different revision without restarting), `:q`, `:help` (lists these, generated
from the same table `:help`'s popup shows). Typing completes command names,
then — after a space — each command's own arguments (paths, presets, strategy
names, revisions); Tab/C-n/Down cycle the menu forward, C-p/Up back, Enter
accepts the highlighted entry or runs the line when nothing is highlighted,
Esc cancels.

  vim      j/k move · C-w C-w (or C-w h/j/k/l) pane · gg/G ends · C-d/C-u half
           space/f/C-f, C-b page · x toggle reviewed · q / Esc quit · ?
           opens a keybinding help popup (Esc/q close it without quitting)
           : opens the command bar (see above)
           in the code pane: h/l/w/b/e/0/$/{/} move the cursor, zh/zl scroll
           the pane horizontally without moving it, K shows the symbol under
           it (Esc/q close that popup without quitting)
           / opens a text-search prompt (Enter jumps to the first match at or
           after the cursor, Esc cancels, Backspace edits); * and # jump to
           the next/previous occurrence of the symbol under the cursor,
           resolved via tree-sitter identifiers rather than text; n/N cycle
           whichever search — text or symbol — is currently active, wrapping
           ge opens the selected hunk's file in $VISUAL/$EDITOR at its line
           (a chord, not bare `e`, which is already word-end in the code pane)
           in the why pane: j/k move a line cursor over reason/details/dep
           lines; on a `dep` (def→use edge) line, K previews the referenced
           hunk and gd (or Enter) jumps to it and focuses the code pane; C-o
           jumps back. A dep line whose hunk isn't part of this review (a
           glob filter, --only-comments, or an unreviewed file) previews as
           such and gd/Enter does nothing rather than guessing where to go
  vscode   up/down move · F6 / shift-F6 pane · C-1/C-2/C-3 pane · PageUp/PageDown
           page · space / enter toggle reviewed · C-q / Esc quit · F1 opens a
           keybinding help popup (Esc closes it without quitting)
           C-Shift-P opens the command bar (vscode's own command-palette key;
           C-P, its \"Go to File\", opens the bar pre-filled with `goto `)
           in the code pane: left/right/C-left/C-right/Home/End move the
           cursor, shift-left/shift-right scroll the pane horizontally
           without moving it, F12 shows the symbol under it (Esc closes that
           popup)
           C-f opens the text-search prompt, F3/shift-F3 cycle its matches;
           C-F12/shift-C-F12 jump to the next/previous occurrence of the
           symbol under the cursor (same tree-sitter search as vim's */#)
           C-o opens the selected hunk's file in $VISUAL/$EDITOR at its line
           in the why pane: up/down move a line cursor over reason/details/dep
           lines; on a `dep` (def→use edge) line, F12 previews the referenced
           hunk and C-Enter jumps to it and focuses the code pane; Alt-Left
           (vscode's own \"Go Back\") jumps back. A dep line whose hunk isn't
           part of this review previews as such and C-Enter does nothing
           rather than guessing where to go

Long lines are clipped, not wrapped, so the line-number gutter stays put; a
‹/› marker appears in the code pane's title whenever the current file has
content scrolled past the left/right edge.
";

// A path filter's compiled matcher: an optional include set (empty/absent
// means "everything") and an optional exclude set. A path is kept when it
// matches at least one include pattern (or none were given) AND matches no
// exclude pattern.
//
// Deliberately unlike `.gitignore`: matching is order-independent. `.gitignore`
// lets a later pattern re-include something an earlier one excluded (the last
// match wins); here any negative match excludes a path, full stop, regardless
// of where it appeared relative to the positives on the command line. This is
// a real, deliberate difference from what a shell-glob-literate user might
// expect from `!pattern`, so it's worth restating at every place a glob list
// turns into a `PathGlobs` — see `build_globs` below, where the patterns are
// actually parsed.
#[derive(Clone, Default)]
struct PathGlobs {
    include: Option<globset::GlobSet>,
    exclude: Option<globset::GlobSet>,
}

impl PathGlobs {
    fn is_match(&self, path: &str) -> bool {
        let included = self.include.as_ref().is_none_or(|g| g.is_match(path));
        let excluded = self.exclude.as_ref().is_some_and(|g| g.is_match(path));
        included && !excluded
    }

    fn is_empty(&self) -> bool {
        self.include.is_none() && self.exclude.is_none()
    }
}

/// What never reached the screen, and why — the numbers `:audit` reports.
/// File-level counts are filled in by `Filter::apply` and the gather functions;
/// hunk-level ones come from the engine's own `dropped` record. Anything hidden
/// that no field here explains is a bug, and `:audit` says so rather than
/// quietly rounding it away.
#[derive(Clone, Copy, Default)]
struct Ledger {
    /// paths git reported changed, before any client-side filtering
    files_seen: usize,
    /// dropped by the built-in generated/lock set
    files_generated: usize,
    /// dropped because the repo's own .gitattributes declares them generated
    files_declared: usize,
    /// dropped by a path glob (positive miss or negative match)
    files_globbed: usize,
    /// listed as changed but unreadable, so never sent to the engine
    files_unreadable: usize,
    /// import hunks — not dropped, but skippable: they are counted with the
    /// noise a filter can hide rather than as something removed
    hunks_import: usize,
    /// hunks the engine dropped under `only_comments`
    hunks_non_comment: usize,
    /// analyzer findings read from `--sarif` files
    findings_seen: usize,
    /// of those, ones whose line is in no hunk this change touched — the
    /// normal case for a file the diff barely reached, counted so the
    /// difference is never a silent one
    findings_unplaced: usize,
    /// files named in the `--coverage` tracefiles
    coverage_files: usize,
    /// of those, ones this review never looked at — usual, since a tracefile
    /// covers a project and a change touches part of it
    coverage_unmatched: usize,
}

/// Which changed files reach the engine. Generated and lock files are dropped
/// before their blobs are even read — parsing a lock file costs more than the
/// review it would add. Globs, when given, keep a path that matches at least
/// one positive pattern (or there are none) and no negative one.
#[derive(Clone)]
struct Filter {
    globs: PathGlobs,
    skip_generated: bool,
    /// set by `apply`: whether every path that matched a positive pattern was
    /// then excluded by a negative one — `note`'s only use for it, so the
    /// "nothing to review" message can say *why* rather than just *that*.
    negatives_emptied: std::cell::Cell<bool>,
    /// file-level accounting, filled in by `apply` (and by the gather that owns
    /// this filter, for unreadable paths) — read once the load is done.
    tally: std::cell::Cell<Ledger>,
}

/// Why a path never reached the engine — one variant per file-level count in
/// `Ledger`, so the tally and the decision can't drift apart.
enum FileDrop {
    Generated,
    Globbed,
}

impl Filter {
    /// The drop decision itself: `None` keeps the path.
    fn reject(&self, path: &str) -> Option<FileDrop> {
        if self.skip_generated && ordo::is_generated_path(path) {
            return Some(FileDrop::Generated);
        }
        (!self.globs.is_match(path)).then_some(FileDrop::Globbed)
    }

    /// `reject` read as a predicate — the glob tests' vocabulary.
    #[cfg(test)]
    fn keep(&self, path: &str) -> bool {
        self.reject(path).is_none()
    }

    /// Winnow a change list: globs and the built-in generated set first, then one
    /// `git check-attr` for whatever the repo declares generated itself. That
    /// declaration decides the cases a path can't — a committed `dist/`, a
    /// snapshot directory, a tool-written changelog.
    fn apply(&self, paths: Vec<String>) -> Vec<String> {
        let any_positive = paths
            .iter()
            .any(|p| self.globs.include.as_ref().is_none_or(|g| g.is_match(p)));
        let any_survives = paths.iter().any(|p| self.globs.is_match(p));
        self.negatives_emptied
            .set(any_positive && !any_survives && !paths.is_empty());
        let mut tally = self.tally.get();
        tally.files_seen += paths.len();
        let mut kept: Vec<String> = vec![];
        for p in paths {
            match self.reject(&p) {
                Some(FileDrop::Generated) => tally.files_generated += 1,
                Some(FileDrop::Globbed) => tally.files_globbed += 1,
                None => kept.push(p),
            }
        }
        if !self.skip_generated || kept.is_empty() {
            self.tally.set(tally);
            return kept;
        }
        let declared = declared_generated(&kept);
        let before = kept.len();
        kept.retain(|p| !declared.contains(p));
        tally.files_declared += before - kept.len();
        self.tally.set(tally);
        kept
    }

    /// Record a path that was listed as changed but could not be read, so it is
    /// accounted for rather than silently absent (`gather_worktree_range`).
    fn note_unreadable(&self) {
        let mut t = self.tally.get();
        t.files_unreadable += 1;
        self.tally.set(t);
    }

    // so an empty review doesn't read as "no changes" when it's the filter
    fn note(&self) -> &'static str {
        if !self.globs.is_empty() {
            return if self.negatives_emptied.get() {
                " (every matching path was excluded by a negative glob)"
            } else {
                " matching the given globs"
            };
        }
        if self.skip_generated {
            " (generated and lock files are skipped; --all keeps them)"
        } else {
            ""
        }
    }
}

// `*` deliberately crosses `/`, so `src/*` reads as everything under src/ and
// `*.rs` matches at any depth — what a shell-shaped filter is expected to mean
// here, and the shell itself never gets to expand these against the worktree.
//
// A leading `!` makes a pattern negative (exclude rather than include);
// `\!literal` escapes a leading bang for a path genuinely starting with one.
// Matching against the whole set is order-independent (see `PathGlobs`) —
// unlike `.gitignore`, a negative here can never be re-included by a later
// positive pattern.
fn build_globs(pats: &[String]) -> Result<PathGlobs, String> {
    let mut inc = globset::GlobSetBuilder::new();
    let mut exc = globset::GlobSetBuilder::new();
    let (mut any_inc, mut any_exc) = (false, false);
    for p in pats {
        // a leading `!` makes the pattern negative; `\!literal` escapes it —
        // stripping only the backslash, so the glob still starts with the
        // literal `!` character rather than losing it
        let (negative, pat): (bool, &str) = if let Some(rest) = p.strip_prefix('!') {
            (true, rest)
        } else if let Some(rest) = p.strip_prefix('\\') {
            (false, rest)
        } else {
            (false, p.as_str())
        };
        if pat.is_empty() {
            return Err(format!("bad glob '{p}': empty pattern"));
        }
        let g = globset::GlobBuilder::new(pat)
            .literal_separator(false)
            .build()
            .map_err(|e| format!("bad glob '{p}': {e}"))?;
        if negative {
            exc.add(g);
            any_exc = true;
        } else {
            inc.add(g);
            any_inc = true;
        }
    }
    let include = any_inc
        .then(|| inc.build())
        .transpose()
        .map_err(|e| e.to_string())?;
    let exclude = any_exc
        .then(|| exc.build())
        .transpose()
        .map_err(|e| e.to_string())?;
    Ok(PathGlobs { include, exclude })
}

/// rev, keymap, path filter, --only-comments, theme, --rules files
/// What the command line asked for. A named struct rather than the eight-wide
/// tuple this was, where the only thing keeping `sarif` out of `extra_rules`
/// was that both are `Vec<String>` in the right order.
struct ParsedArgs {
    rev: String,
    keys: Keymap,
    /// whether a docs-only hunk sorts after the code (tui.toml `docs_last`)
    docs_last: bool,
    filter: Filter,
    only_comments: bool,
    theme: Theme,
    extra_rules: Vec<String>,
    sarif: Vec<String>,
    coverage: Vec<String>,
    /// `--no-catalog`: run only the caller's own rules this once
    no_catalog: bool,
}

/// The user-facing documentation, embedded in the binary so `ordo help
/// <topic>` works with no network, no install layout to find, and no chance of
/// showing a page from a different version than the one running. The `docs/`
/// files are the same ones GitHub renders; a test keeps this list and that
/// directory in step.
const TOPICS: &[(&str, &str, &str)] = &[
    (
        "architecture",
        "for contributors: the seams, and where a new fact goes",
        include_str!("../../../docs/architecture.md"),
    ),
    (
        "cli",
        "the engine CLI, schema v2, and the library API",
        include_str!("../../../docs/cli.md"),
    ),
    (
        "tui",
        "this reviewer: revisions, filters, keys, command bar, themes",
        include_str!("../../../docs/tui.md"),
    ),
    (
        "reviewing",
        "the detail layer, rationale patterns, advisories, the ledger",
        include_str!("../../../docs/reviewing.md"),
    ),
    (
        "rules",
        "conventions as data, and the rulesets that ship with ordo",
        include_str!("../../../docs/rules.md"),
    ),
    (
        "languages",
        "every supported language, and the shape it is read in",
        include_str!("../../../docs/languages.md"),
    ),
    (
        "ceilings",
        "what ordo deliberately does not do",
        include_str!("../../../docs/ceilings.md"),
    ),
];

/// The topic list appended to `--help`, and printed on its own by `ordo help`.
fn topic_list() -> String {
    let mut out = String::from("topics (`ordo help <topic>`):\n");
    for (name, blurb, _) in TOPICS {
        out.push_str(&format!("  {name:<11} {blurb}\n"));
    }
    out
}

/// Sends long output through `$PAGER` when there is a terminal to page for.
/// Falls back to plain stdout whenever that isn't true or the pager won't
/// start, so `ordo help rules | grep max-` behaves like any other command.
fn page_out(text: &str) {
    use std::io::{IsTerminal, Write};
    let pager = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());
    if std::io::stdout().is_terminal() && !pager.is_empty() {
        let mut parts = pager.split_whitespace();
        let Some(program) = parts.next() else {
            return print!("{text}");
        };
        let mut cmd = Command::new(program);
        cmd.args(parts).stdin(std::process::Stdio::piped());
        // `less` without these quits on short input and eats the colours of
        // whatever the user's LESS already sets; -R -F -X is the conventional
        // "act like git" set
        if program == "less" && std::env::var("LESS").is_err() {
            cmd.env("LESS", "-RFX");
        }
        if let Ok(mut child) = cmd.spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
    print!("{text}");
}

/// `ordo help [<topic>]`. Returns the exit code: an unknown topic is a usage
/// error, not an empty page.
fn help_topic(topic: Option<&str>) -> i32 {
    let Some(topic) = topic else {
        page_out(&format!("{USAGE}\n{}", topic_list()));
        return 0;
    };
    match TOPICS.iter().find(|(name, _, _)| *name == topic) {
        Some((_, _, body)) => {
            page_out(body);
            0
        }
        None => {
            eprintln!("ordo: no help topic '{topic}'\n\n{}", topic_list());
            2
        }
    }
}

fn parse_args() -> Result<ParsedArgs, i32> {
    parse_argv(std::env::args().skip(1).collect())
}

/// The command line resolved against the config file, over a vector rather
/// than the process's own arguments so a test can hand it one. `parse_args`
/// is the one-line caller.
fn parse_argv(argv: Vec<String>) -> Result<ParsedArgs, i32> {
    let raw = flag_loop(argv)?;
    // the config's preset and theme are defaults; an explicit flag still wins
    let cfg = config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_key_config(&t));
    let (keys, preset) = resolve_keymap(cfg.as_ref(), raw.preset, raw.preset_given)?;
    let (theme, theme_name) = resolve_theme(cfg.as_ref(), raw.theme_name, raw.theme_given)?;
    if raw.want_init {
        // `--init-config` is a whole run of its own: nothing is reviewed, and
        // the exit code is the write's own (`Err(0)` on success, as `--help`
        // and `--version` already report a clean stop)
        write_init_config(&preset, &theme_name, raw.force)?;
        return Err(0);
    }
    let globs = build_globs(&raw.globs).map_err(|e| {
        eprintln!("ordo: {e}");
        2
    })?;
    let filter = Filter {
        globs,
        skip_generated: raw.skip_generated,
        negatives_emptied: std::cell::Cell::new(false),
        tally: std::cell::Cell::new(Ledger::default()),
    };
    Ok(ParsedArgs {
        rev: raw.rev.unwrap_or_else(|| "HEAD".to_string()),
        keys,
        docs_last: cfg.as_ref().and_then(|c| c.docs_last).unwrap_or(true),
        filter,
        only_comments: raw.only_comments,
        theme,
        extra_rules: raw.extra_rules,
        sarif: raw.sarif,
        coverage: raw.coverage,
        no_catalog: raw.no_catalog,
    })
}

/// The command line as written, before the config file has its say.
struct RawArgs {
    rev: Option<String>,
    globs: Vec<String>,
    skip_generated: bool,
    no_catalog: bool,
    only_comments: bool,
    /// whether the preset / theme was *chosen* (flag or env) — a config
    /// file's own setting only applies when it wasn't
    preset_given: bool,
    preset: String,
    theme_given: bool,
    theme_name: String,
    extra_rules: Vec<String>,
    sarif: Vec<String>,
    coverage: Vec<String>,
    want_init: bool,
    force: bool,
}

/// A flag whose value is the next argument.
#[derive(Clone, Copy)]
enum Pending {
    Preset,
    Theme,
    Rules,
    Sarif,
    Coverage,
}

/// The flag loop: what each argument said, with nothing resolved yet.
fn flag_loop(argv: Vec<String>) -> Result<RawArgs, i32> {
    let mut raw = RawArgs {
        rev: None,
        globs: vec![],
        skip_generated: true,
        no_catalog: false,
        only_comments: false,
        preset_given: std::env::var("ORDO_TUI_KEYS").is_ok(),
        preset: std::env::var("ORDO_TUI_KEYS").unwrap_or_else(|_| "vim".to_string()),
        theme_given: std::env::var("ORDO_TUI_THEME").is_ok(),
        theme_name: std::env::var("ORDO_TUI_THEME").unwrap_or_else(|_| "dark".to_string()),
        extra_rules: vec![],
        sarif: vec![],
        coverage: vec![],
        want_init: false,
        force: false,
    };
    // `ordo help [<topic>]` short-circuits everything else: it takes an
    // argument the flag loop below would otherwise read as a revision.
    if let Some(first) = argv.first() {
        if first == "help" || first == "--help" || first == "-h" {
            return Err(help_topic(argv.get(1).map(String::as_str)));
        }
    }
    let mut pending: Option<Pending> = None;
    for a in argv {
        if let Some(p) = pending.take() {
            raw.take_value(p, a);
            continue;
        }
        match a.as_str() {
            "--keys" => pending = Some(Pending::Preset),
            "--theme" => pending = Some(Pending::Theme),
            "--rules" => pending = Some(Pending::Rules),
            "--sarif" => pending = Some(Pending::Sarif),
            "--coverage" => pending = Some(Pending::Coverage),
            s if s.starts_with("--keys=") => {
                raw.take_value(Pending::Preset, s["--keys=".len()..].to_string())
            }
            s if s.starts_with("--theme=") => {
                raw.take_value(Pending::Theme, s["--theme=".len()..].to_string())
            }
            s if s.starts_with("--rules=") => {
                raw.take_value(Pending::Rules, s["--rules=".len()..].to_string())
            }
            s if s.starts_with("--sarif=") => {
                raw.take_value(Pending::Sarif, s["--sarif=".len()..].to_string())
            }
            s if s.starts_with("--coverage=") => {
                raw.take_value(Pending::Coverage, s["--coverage=".len()..].to_string())
            }
            "--all" => raw.skip_generated = false,
            "--no-catalog" => raw.no_catalog = true,
            "--only-comments" => raw.only_comments = true,
            "--init-config" => raw.want_init = true,
            "--force" => raw.force = true,
            "-h" | "--help" | "help" => return Err(help_topic(None)),
            "-V" | "--version" | "version" => {
                println!(
                    "ordo {} (ordo schema {})",
                    env!("CARGO_PKG_VERSION"),
                    ordo::SCHEMA_VERSION
                );
                return Err(0);
            }
            s if s.starts_with('-') => {
                eprintln!("ordo: unknown flag '{s}'\n\n{USAGE}");
                return Err(2);
            }
            s if raw.rev.is_some() => raw.globs.push(s.to_string()),
            s => raw.rev = Some(s.to_string()),
        }
    }
    match pending {
        Some(Pending::Preset) => {
            eprintln!("ordo: --keys needs a preset name\n\n{USAGE}");
            Err(2)
        }
        Some(Pending::Theme) => {
            eprintln!("ordo: --theme needs a value\n\n{USAGE}");
            Err(2)
        }
        _ => Ok(raw),
    }
}

impl RawArgs {
    fn take_value(&mut self, flag: Pending, value: String) {
        match flag {
            Pending::Preset => {
                self.preset = value;
                self.preset_given = true;
            }
            Pending::Theme => {
                self.theme_name = value;
                self.theme_given = true;
            }
            Pending::Rules => self.extra_rules.push(value),
            Pending::Sarif => self.sarif.push(value),
            Pending::Coverage => self.coverage.push(value),
        }
    }
}

/// The keymap for the chosen preset, with the config's rebinds applied, and
/// the preset's name.
fn resolve_keymap(
    cfg: Option<&KeyConfig>,
    preset: String,
    preset_given: bool,
) -> Result<(Keymap, String), i32> {
    let preset = match (cfg, preset_given) {
        (Some(c), false) => c.preset.clone().unwrap_or(preset),
        _ => preset,
    };
    let Some(keys) = keymap(&preset) else {
        eprintln!("ordo: unknown key preset '{preset}' (want: vim, vscode)");
        return Err(2);
    };
    let keys = match cfg {
        Some(c) => {
            for p in &c.problems {
                eprintln!("ordo: tui.toml: {p}");
            }
            apply_key_config(keys, c)
        }
        None => keys,
    };
    Ok((keys, preset))
}

/// The theme by name, with the config's colour overrides applied, and the
/// name. Same precedence as the keymap: an explicit `--theme` beats the
/// config's.
fn resolve_theme(
    cfg: Option<&KeyConfig>,
    theme_name: String,
    theme_given: bool,
) -> Result<(Theme, String), i32> {
    let theme_name = match (cfg, theme_given) {
        (Some(c), false) => c.theme.clone().unwrap_or(theme_name),
        _ => theme_name,
    };
    let Some(theme) = theme(&theme_name) else {
        eprintln!(
            "ordo: unknown theme '{theme_name}' (want: {})",
            theme_names().join(", ")
        );
        return Err(2);
    };
    let theme = match cfg {
        Some(c) => apply_theme_colors(theme, &c.colors),
        None => theme,
    };
    Ok((theme, theme_name))
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

// one-based "(i/total)" progress messages for the load stages that iterate
// per item — pulled out of `gather_range`/`gather_uncommitted`/`load` so the
// counter arithmetic and wording are unit-testable without a worker thread
fn read_progress(path: &str, i: usize, total: usize) -> String {
    format!("reading {path} ({}/{total})", i + 1)
}

fn highlight_progress(i: usize, total: usize) -> String {
    format!("highlighting ({}/{total})", i + 1)
}

fn main() -> std::io::Result<()> {
    let ParsedArgs {
        rev,
        keys,
        docs_last,
        filter,
        only_comments,
        theme,
        extra_rules,
        sarif,
        coverage,
        no_catalog,
    } = match parse_args() {
        Ok(v) => v,
        Err(code) => std::process::exit(code),
    };
    let Some(target) = resolve(&rev) else {
        eprintln!(
            "ordo: '{rev}' is not a git revision, a commit range or a \
             GitButler CLI ID (see `but status`)"
        );
        std::process::exit(1);
    };
    // the single commit the `K` popup's history section reviews from — a
    // range's tip, or HEAD standing in for the uncommitted area (`zz`)
    let review_sha = review_commit_sha(&target);
    let uncommitted = matches!(target, Target::Uncommitted | Target::WorktreeRange(_));
    // rules are the client's to collect: this user's, then this repository's
    let repo_root = git(&["rev-parse", "--show-toplevel"]);
    let report = load_rules_report(repo_root.trim(), &extra_rules);
    for p in &report.problems {
        eprintln!("ordo: {p}");
    }
    let rules_report = report.lines();
    // the catalog is on unless this run turned it off, either way round: the
    // flag for once, `catalog = false` in a rules file for always
    let rules = report.rule_set(!no_catalog && report.catalog);
    run(
        rev,
        keys,
        docs_last,
        target,
        filter,
        only_comments,
        review_sha,
        uncommitted,
        theme,
        rules,
        rules_report,
        sarif,
        coverage,
    )
}

/// What the worker thread reports back over the channel while `gather`,
/// `highlight_file` and `ordo::run` do their work off the draw loop.
enum LoadMsg {
    /// one line of progress, replacing whatever was shown before
    Progress(String),
    /// `build_items` came back empty — the message `main` used to print
    /// before taking the screen
    Empty(String),
    /// boxed, like `State::Ready` — keeps `LoadMsg` itself small regardless of
    /// how much a load result carries
    Done(Box<LoadResult>),
}

struct LoadResult {
    items: Vec<Item>,
    /// exactly what `ordo::run` was given, kept so `:strategy` can re-run the
    /// engine on it. Rebuilding it from `sources` was lossy — `.lines()` drops
    /// the trailing newline and turns an absent side into an empty one, so a
    /// deleted file came back as an emptied file and the group reasons changed
    /// under a re-order that was supposed to be a no-op
    changes: Vec<Change>,
    /// indices into `items` visible under the launch-time `--only-comments`
    /// state (`:only-comments`, `:all` and `:filter` narrow/widen this live
    /// from here — see `compute_view`)
    view: Vec<usize>,
    comments_only: bool,
    sources: Sources,
    highlights: Highlights,
    /// the same timing line `main` used to print before init, now printed
    /// after `ratatui::restore()` so it still lands on the shell's
    /// scrollback
    timing: String,
    /// `reviewed[i]` set from a persisted mark whose key matches item `i`
    /// (see `mark_key`) — loading is here, off the draw loop, alongside
    /// the rest of the load work
    reviewed: Vec<bool>,
    /// where a toggle gets persisted; `None` when the repo root or the
    /// cache directory couldn't be resolved, in which case marks just
    /// live for this session and are never written
    marks_path: Option<PathBuf>,
    marks: HashMap<u64, u64>,
    /// commits touching each file in `FILE_CHURN_WINDOW` — the cheap, eager
    /// half of the churn signal; `H` refines the selected hunk to its exact
    /// lines (see `hunk_churn`)
    file_churn: HashMap<String, usize>,
    /// group id -> reason, for `:group`'s header rows
    groups: HashMap<String, String>,
    /// what was dropped on the way here, for `:audit`
    ledger: Ledger,
    /// the engine's change ledger (P23.1) — what the list defaults to being a
    /// list of; distinct from `ledger`, which is the `:audit` one
    symbol_ledger: Vec<ordo::model::LedgerEntry>,
    notes: HashMap<u64, String>,
    notes_path: Option<PathBuf>,
    deltas: Vec<Delta>,
    delta_gone: usize,
}

/// group id -> the engine's `Group::reason`, for `:group`'s header rows.
fn group_reasons(out: &Output) -> HashMap<String, String> {
    out.groups
        .iter()
        .map(|g| (g.id.clone(), g.reason.clone()))
        .collect()
}

/// Everything `gather`/`highlight_file`/`ordo::run` need, run off the draw
/// loop. `ordo::run` itself can `eprintln!` when a change is degraded to
/// positional order — unreachable here, since every `Change` this file
/// builds always sets `new` (see `gather_range`/`gather_uncommitted`), which
/// is the one branch in `build_change` that never degrades.
/// Everything the worker needs to produce a review. Bundled because these
/// travel together to `load` and to every reload of it, and are otherwise eight
/// positional arguments whose order nothing checks.
struct LoadSpec {
    target: Target,
    /// whether a docs-only hunk sorts after the code (see `model::Options`)
    docs_last: bool,
    filter: Filter,
    only_comments: bool,
    rev: String,
    /// highlighting happens on the worker, off the draw loop, so it needs the
    /// theme's syntax colours rather than re-highlighting on every redraw
    syn: Syntax,
    /// the reviewer's own rules (user + repo) plus the catalog switch,
    /// collected by `main`
    rules: RuleSet,
    /// paths given with `--sarif`; their findings are placed onto the hunks
    sarif: Vec<String>,
    /// paths given with `--coverage`; lcov tracefiles
    coverage: Vec<String>,
}

fn load(spec: LoadSpec, tx: mpsc::Sender<LoadMsg>) {
    let LoadSpec {
        target,
        docs_last,
        filter,
        only_comments,
        rev,
        syn,
        rules,
        sarif,
        coverage,
    } = spec;
    let progress = |msg: String| {
        let _ = tx.send(LoadMsg::Progress(msg));
    };
    // resolved before `target` is consumed below; per-file churn counts from it
    let churn_sha = review_commit_sha(&target);
    let t = std::time::Instant::now();
    let input = match target {
        Target::Commit(sha) => gather(&sha, &filter, &progress),
        Target::Range(base, tip) => gather_range(&base, &tip, &filter, &progress),
        Target::Uncommitted => gather_uncommitted(&filter, &progress),
        Target::WorktreeRange(base) => gather_worktree_range(&base, &filter, &progress),
    };
    // `only_comments` becomes the starting state of the live `:only-comments`
    // filter (see `compute_view`) rather than an engine option — the engine
    // always sees every hunk, so toggling it back off has something to show.
    let (read_ms, files) = (t.elapsed().as_millis(), input.changes.len());
    let t = std::time::Instant::now();
    let sources = split_sources(&input);
    let highlights = highlight_all(&input.changes, &syn, &progress);
    let (hl_ms, hl_files) = (t.elapsed().as_millis(), highlights.len());
    let t = std::time::Instant::now();
    progress("ordering…".to_string());
    let mut input = input;
    input.options.rules = rules.rules;
    input.options.catalog = rules.catalog;
    input.options.disable = rules.disables;
    input.options.docs_last = docs_last;
    let changes = input.changes.clone();
    let out = ordo::run(input);
    let mut ledger = filter.tally.get();
    count_engine_drops(&out, &mut ledger);
    let mut items = build_items(&out);
    place_overlays(&mut items, &sarif, &coverage, &mut ledger);
    refine_items(&mut items, &sources);
    let groups = group_reasons(&out);
    let view = compute_view(&items, only_comments, true, None);
    if view.is_empty() {
        let _ = tx.send(LoadMsg::Empty(empty_message(&rev, only_comments, &filter)));
        return;
    }
    // cheap enough to be eager (see `file_churn`), and the reviewer gets the
    // signal without having to ask for it on every hunk
    let churn_paths: Vec<String> = {
        let mut p: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
        p.sort();
        p.dedup();
        p
    };
    let churn_t = std::time::Instant::now();
    let churn_by_file = match &churn_sha {
        Some(sha) => file_churn(sha, &churn_paths, &progress),
        None => HashMap::new(),
    };
    let (churn_ms, churn_files) = (churn_t.elapsed().as_millis(), churn_by_file.len());
    let timing = format!(
        "ordo: read {files} file{} in {read_ms}ms · highlighted {hl_files} in {hl_ms}ms · \
         counted churn for {churn_files} file{} in {churn_ms}ms · \
         ordered {} hunks into {} groups, {} cluster{} in {}ms",
        plural(files),
        plural(churn_files),
        items.len(),
        out.groups.len(),
        out.clusters.len(),
        plural(out.clusters.len()),
        t.elapsed().as_millis(),
    );
    // reviewed-mark persistence: repo-scoped, best-effort — an unresolved
    // repo root or cache dir just means this session's marks live only in
    // memory (see `marks_file_path`/`load_marks`)
    let repo_root = git(&["rev-parse", "--show-toplevel"]);
    let repo_root = repo_root.trim();
    let marks_path = (!repo_root.is_empty())
        .then(|| marks_file_path(repo_root))
        .flatten();
    let mut marks = marks_path.as_deref().map(load_marks).unwrap_or_default();
    prune_marks(&mut marks, now_unix());
    let notes_path = (!repo_root.is_empty())
        .then(|| notes_file_path(repo_root))
        .flatten();
    let mut notes = notes_path.as_deref().map(load_notes).unwrap_or_default();
    if migrate_notes(&out.ledger, &items, &mut notes) {
        if let Some(p) = notes_path.as_deref() {
            save_notes(p, &notes);
        }
    }
    let reviewed: Vec<bool> = items
        .iter()
        .map(|it| mark_key(&rev, it, &sources).is_some_and(|k| marks.contains_key(&k)))
        .collect();
    // what changed since this review was last opened, then overwrite the
    // snapshot so the next run answers the same question about this one
    let runs_path = (!repo_root.is_empty())
        .then(|| runs_file_path(repo_root))
        .flatten();
    let prev = runs_path.as_deref().map(load_snaps).unwrap_or_default();
    let order: Vec<usize> = (0..items.len()).collect();
    let (snaps, deltas) = compare_runs(&items, &order, &sources, &prev);
    let delta_gone = prev.keys().filter(|k| !snaps.contains_key(*k)).count();
    if let Some(p) = runs_path.as_deref() {
        save_snaps(p, &snaps);
    }
    let _ = tx.send(LoadMsg::Done(Box::new(LoadResult {
        items,
        changes,
        file_churn: churn_by_file,
        view,
        comments_only: only_comments,
        sources,
        highlights,
        timing,
        reviewed,
        marks_path,
        marks,
        groups,
        ledger,
        symbol_ledger: out.ledger.clone(),
        notes,
        notes_path,
        deltas,
        delta_gone,
    })));
}

/// each file's two sides as lines, by path
fn split_sources(input: &Input) -> Sources {
    let split = |s: Option<&String>| {
        s.map(|t| t.lines().map(String::from).collect())
            .unwrap_or_default()
    };
    input
        .changes
        .iter()
        .map(|c| {
            (
                c.path.clone(),
                (split(c.old.as_ref()), split(c.new.as_ref())),
            )
        })
        .collect()
}

/// syntax highlight each new file once, up front (indexed by path)
fn highlight_all(changes: &[Change], syn: &Syntax, progress: &dyn Fn(String)) -> Highlights {
    let candidates: Vec<(&str, &str)> = changes
        .iter()
        .filter_map(|c| Some((c.path.as_str(), c.new.as_deref()?)))
        .collect();
    let total = candidates.len();
    let mut highlights: Highlights = HashMap::new();
    for (i, (path, new)) in candidates.iter().enumerate() {
        progress(highlight_progress(i, total));
        if let Some(h) = highlight_file(path, new, syn) {
            highlights.insert(path.to_string(), h);
        }
    }
    highlights
}

/// the engine records what it dropped and why; fold it into the same ledger
/// the path filter has been filling in, so `:audit` reads one set of numbers
fn count_engine_drops(out: &Output, ledger: &mut Ledger) {
    for d in out.files.iter().flat_map(|f| f.dropped.iter()) {
        match d.reason {
            ordo::model::DropReason::NonComment => ledger.hunks_non_comment += 1,
        }
    }
    // an import hunk is no longer dropped: it is noise, counted with the rest
    ledger.hunks_import = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .filter(|h| h.category == ordo::model::Category::Import)
        .count();
}

/// analyzer findings and coverage onto the hunks that contain their lines,
/// so they arrive in the reading order rather than as a separate flat list.
/// Straight onto the local ledger: `filter.tally` was snapshotted before, so
/// anything written back to it now would never reach the screen.
fn place_overlays(items: &mut [Item], sarif: &[String], coverage: &[String], ledger: &mut Ledger) {
    let findings: Vec<Finding> = sarif_findings(sarif);
    ledger.findings_seen += findings.len();
    ledger.findings_unplaced += place_findings(items, &findings);
    // coverage says whether what the change wrote was ever executed — the fact
    // behind the engine's "code changed but no test touched" proxy
    for path in coverage {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let cov = parse_lcov(&text);
                ledger.coverage_files += cov.files.len();
                ledger.coverage_unmatched += place_coverage(items, &cov);
            }
            Err(e) => note_command_failure("coverage", &[path.as_str()], &e.to_string()),
        }
    }
}

/// what the user sees instead of a review when the filters left nothing
fn empty_message(rev: &str, only_comments: bool, filter: &Filter) -> String {
    let mut msg = if only_comments {
        format!(
            "ordo: nothing to review in {rev} — no comment changes{}",
            filter.note()
        )
    } else {
        format!("ordo: nothing to review in {rev}{}", filter.note())
    };
    // an empty review and a failed git call look identical from here, so
    // when one happened it is the more likely explanation and leads
    for f in command_failures() {
        msg.push_str(&format!("\nordo: {f}"));
    }
    msg
}

/// carry a note across a rename: the ledger knows the name the symbol had,
/// so the note written against the old identity finds its way to the new one.
/// True when any note moved.
fn migrate_notes(
    ledger: &[ordo::model::LedgerEntry],
    items: &[Item],
    notes: &mut HashMap<u64, String>,
) -> bool {
    let mut migrated = false;
    for e in ledger.iter().filter(|e| e.from.is_some()) {
        let Some(old) = e.from.as_deref() else {
            continue;
        };
        let Some(it) = items.iter().find(|i| {
            i.new_range[0] > 0
                && !i.symbols.is_empty()
                && i.symbols.iter().any(|s| s.name == e.name)
        }) else {
            continue;
        };
        let (Some(from_key), Some(to_key)) = (note_key_named(it, old), note_key(it)) else {
            continue;
        };
        if from_key != to_key {
            if let Some(text) = notes.remove(&from_key) {
                notes.insert(to_key, text);
                migrated = true;
            }
        }
    }
    migrated
}

// ------------------------------------------------------------------- view model

type Sources = HashMap<String, (Vec<String>, Vec<String>)>;

struct Item {
    path: String,
    old_range: [usize; 2],
    new_range: [usize; 2],
    /// the list row's parts, kept unjoined so line number and category can be
    /// coloured independently of the path
    mark: String,
    cat: ordo::model::Category,
    rationale: String,
    details: Vec<String>,
    notes: Vec<String>,
    edges: Vec<EdgeRef>,
    /// the key the list groups and folds by — a ledger symbol or a group id,
    /// depending on `App::mode`. Swapped in place by `set_mode`, so the two
    /// display functions never need to know which mode is active.
    bucket: String,
    /// this hunk's entry in `Output::ledger`, when a symbol change is anchored
    /// to it — `None` for an import, a region, or a hunk that changed nothing
    /// nameable
    ledger: Option<usize>,
    noise: bool,
    /// every changed line is a comment or docstring — drives `:only-comments`
    /// (the engine's own field of the same name, see `HunkOut::comment`)
    comment: bool,
    /// the hunk's own defined symbols (name + tree-sitter kind + scope) — the
    /// identity component of a persisted reviewed-mark key (see `mark_key`)
    symbols: Vec<Symbol>,
    /// fallback identity when `symbols` is empty (a body-edit or comment-only
    /// hunk defines nothing of its own)
    enclosing: Option<String>,
    /// the engine's `HunkOut::group` id — drives `:group`'s header rows (see
    /// `App::groups` for the id -> reason lookup)
    group: String,
    /// everything anyone noticed about this hunk: the engine's catalog
    /// advisories and rule hits, plus any analyzer result placed here
    findings: Vec<ordo::model::Finding>,
    /// where the names this hunk introduces are used — marked in the code
    /// gutter and stepped through with `]`/`[`, rather than listed as line
    /// numbers in the why pane
    uses_at: Vec<ordo::model::UseSite>,
    /// (executed, executable) counts for the lines this hunk changed, when a
    /// coverage tracefile covers them
    executed: Option<(usize, usize)>,
    /// intra-line refinement (`ordo::refine`), parallel to the hunk's lines on
    /// each side: `Some(spans)` means the line was paired with its counterpart
    /// and only those char spans changed; `None` means it renders whole. Empty
    /// when the file has no grammar or the hunk was too large to refine.
    refined: ordo::refine::Refined,
    /// this hunk's 1-based position in `out.clusters` (P12.3's independent
    /// change parts) — `None` when the whole review is one cluster, so
    /// `:quickfix` has no cluster worth naming.
    cluster: Option<usize>,
}

/// One `dep` line's rendered label plus the target hunk's resolved position in
/// `app.items` — `None` when the referenced hunk was never built into this
/// review (filtered out, `--only-comments`, or a file that wasn't sent at
/// all), so the why pane can act on it (preview, jump) without ever guessing.
struct EdgeRef {
    label: String,
    target: Option<usize>,
    /// this hunk *uses* what the target defines — so the target is something
    /// this hunk depends on, and reviewing this one first is reviewing a call
    /// before its callee
    dependency: bool,
}

/// A cursor position in the *new* content of the selected item's file — line
/// and column are both 0-based char offsets. Only meaningful for the code pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cursor {
    line: usize,
    col: usize,
}

/// A parsed file, cached per path so repeated `K` presses don't reparse. `src`
/// is the exact joined text the tree was built from, so node byte ranges slice
/// it directly.
struct ParsedFile {
    tree: Tree,
    src: String,
}

/// The floating symbol-hover popup over the code pane: kind/signature/doc,
/// then (when the definition resolves) a cross-commit history section.
/// The dependency canvas: the selected hunk at top centre, everything it needs
/// fanning out to the left and everything that needs it to the right.
///
/// Holds only what the layout cannot re-derive — which hunk it was opened on,
/// which card is selected, and how far the fan is scrolled. The cards
/// themselves are rebuilt from `Item.edges` every frame, so a reload cannot
/// leave a stale copy on screen.
struct Canvas {
    /// the item the canvas was opened on
    anchor: usize,
    /// index into the flattened card list (left side first, then right)
    sel: usize,
}

/// One card: a hunk this one depends on, or one that depends on it.
struct Card {
    /// item index the card shows
    idx: usize,
    /// `Needs` on the left, `NeededBy` on the right
    needs: bool,
    label: String,
}

struct Popup {
    title: String,
    /// styled, so a popup can carry syntax-highlighted code (the dep preview)
    /// rather than only prose
    lines: Vec<Line<'static>>,
    /// vertical scroll offset, so a popup taller than the terminal (the help
    /// popup, most likely) is reachable rather than just clipped
    scroll: u16,
    /// horizontal offset — popups do not wrap, matching the code pane, so a
    /// long line is reached by scrolling rather than reflowed
    hscroll: u16,
}

impl Popup {
    /// A popup at the top, unscrolled — the common case; a popup that opens
    /// pre-scrolled builds the struct directly.
    fn new(title: impl Into<String>, lines: Vec<Line<'static>>) -> Popup {
        Popup {
            title: title.into(),
            lines,
            scroll: 0,
            hscroll: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchKind {
    Text,
    Symbol,
}

/// An active `/`, `*`, or `#` search over the code pane's current file.
/// `matches` are (line, start_col, end_col) char ranges in document order;
/// `index` is the current one — `n`/`N` cycle through them, wrapping.
struct Search {
    kind: SearchKind,
    pattern: String,
    matches: Vec<(usize, usize, usize)>,
    index: usize,
}

/// The `/` search prompt: keystrokes build `text` instead of running normal
/// actions (see `run`'s prompt branch). `anchor` is the cursor position to
/// restore on `Esc`.
struct Prompt {
    text: String,
    anchor: Cursor,
}

/// The `:` command bar: keystrokes build `text` instead of running normal
/// actions (see `run`'s command branch and `handle_command_key`), same
/// interception scheme as `Prompt`. `candidates` is the current completion
/// menu (command names, then — once `text` has a space — that command's own
/// argument candidates); `selected` is which of them is highlighted, `None`
/// when the user hasn't cycled to one yet.
struct CommandBar {
    text: String,
    candidates: Vec<String>,
    selected: Option<usize>,
    /// `:e`'s ref pool (`rev_completions()`), fetched at most once per opened
    /// bar and reused for every later keystroke in the same session — see
    /// `recompute_candidates`. `None` until first needed; a fresh bar always
    /// starts `None` so a branch created since the last session is picked up.
    rev_cache: Option<Vec<String>>,
}

struct App {
    items: Vec<Item>,
    reviewed: Vec<bool>,
    /// indices into `items`, currently visible under `comments_only`/
    /// `show_all`/`path_filter` — never empty (a filter that would empty it
    /// is rejected, see `set_filters`). Navigation and the list pane move
    /// through this rather than `items` directly, so a hidden hunk is never
    /// selected.
    view: Vec<usize>,
    /// `:only-comments` — show only `Item::comment` hunks
    comments_only: bool,
    /// `:all` — show `Item::noise` hunks too (generated paths and
    /// formatting-only changes); defaults to `true` (everything shown),
    /// matching this reviewer's behaviour before command mode existed
    show_all: bool,
    /// `:filter <glob>` — the typed pattern (for display) and its compiled
    /// matcher; `None` when no live filter is active
    path_filter: Option<(String, PathGlobs)>,
    /// the open command bar, if any — see `CommandBar`
    command: Option<CommandBar>,
    sel: usize,
    scroll: u16,
    /// horizontal scroll offset into the code pane's code column (the sign
    /// bar and line-number gutter never scroll) — column units, char-indexed
    hscroll: u16,
    why_scroll: u16,
    /// the why pane's selected-line cursor (an index into its logical lines,
    /// same units as `why_len`) — moved by `Next`/`Prev` when the why pane has
    /// focus, mirroring the code pane's `cursor`
    why_sel: usize,
    /// rendered line counts, filled in by `draw`, so the scroll actions can clamp
    code_len: usize,
    why_len: usize,
    /// code pane's visible row count, filled in by `draw`, so cursor motions can
    /// scroll to keep the cursor in view
    code_height: u16,
    /// why pane's visible row count, filled in by `draw`, so `why_sel` motions
    /// can scroll to keep it in view (mirrors `code_height`)
    why_height: u16,
    /// code pane's visible code-column width (pane width minus the sign bar
    /// and gutter), filled in by `draw`, so cursor motions can horizontally
    /// scroll to keep the cursor in view
    code_width: u16,
    focus: Pane,
    keys: Keymap,
    pending: Option<Key>,
    /// whether a docs-only hunk sorts after the code it describes; a `:config`
    /// setting, so a re-order has to be told about it
    docs_last: bool,
    /// the engine's input for this review — see `LoadResult::changes`. The
    /// text is also held line-split in `sources`, which the panes read; this
    /// copy exists so a re-run is the same run, and both are bounded by the
    /// size of the change under review
    changes: Vec<Change>,
    sources: Sources,
    highlights: Highlights,
    cursor: Cursor,
    popup: Option<Popup>,
    trees: HashMap<String, ParsedFile>,
    prompt: Option<Prompt>,
    search: Option<Search>,
    /// the commit the history section is relative to; None when it couldn't
    /// be resolved (history is then reported unavailable rather than guessed)
    review_sha: Option<String>,
    /// true when reviewing the uncommitted area (`zz`) or a `base..zz` /
    /// `base...zz` worktree range — there the worktree file IS the new
    /// content, so `ge`/C-o's line number is always exact; for a commit or
    /// commit range it may not be (see `edit_target`)
    uncommitted: bool,
    /// history-section lines, cached per (path, name, kind, scope) so
    /// repeated `K` on the same symbol doesn't re-shell-out to git
    history_cache: HashMap<(String, String, String, Option<String>), Vec<String>>,
    /// per-hunk churn, cached per (path, new-side range) — see `Churn`. Only
    /// ever filled on demand: `git log -L` costs ~0.2s on a large file and
    /// the load path is already the slow one.
    /// `None` means asked for and unavailable, which the why pane says out
    /// loud — a key that silently does nothing reads as broken
    churn_cache: HashMap<(String, usize, usize), Option<Churn>>,
    /// whether the built-in construct catalog runs — `--no-catalog`, or
    /// `catalog = false` in a rules file
    catalog: bool,
    /// `disable` globs from every rules file, forwarded to the engine so they
    /// silence catalog entries as well as the caller's own rules
    disables: Vec<String>,
    /// bundled rulesets named by `include` — what `:config` shows as on
    includes: Vec<String>,
    /// `:config`'s open state; `None` when it is closed
    config: Option<ConfigUi>,
    /// commits per file in `FILE_CHURN_WINDOW`, computed once at load. The
    /// cheap half of the signal: always shown, and `H` refines the selected
    /// hunk to its exact lines.
    file_churn: HashMap<String, usize>,
    /// positions `JumpToEdge` jumped from, most-recent last; `JumpBack` pops
    /// one. Bounded by `JUMP_STACK_CAP`.
    jumps: Vec<(usize, Cursor)>,
    /// the reviewed rev string, as typed — part of a persisted mark's key
    /// (see `mark_key`)
    rev: String,
    /// where `ToggleReviewed` persists a mark; `None` when the repo root or
    /// cache dir couldn't be resolved, so toggles just aren't written
    marks_path: Option<PathBuf>,
    /// in-memory mirror of the mark file, kept in sync on every toggle
    marks: HashMap<u64, u64>,
    /// the five diff/selection background colours — `--theme`/`$ORDO_TUI_THEME`,
    /// carried across an `:e` reload since it's a display preference, not
    /// something indexed to the reviewed revision
    theme: Theme,
    /// `:group` — show a non-selectable group-reason header before each run
    /// of the reading-order list that shares a group id
    show_groups: bool,
    /// the focused pane fills the frame. A narrow terminal zooms on its own
    /// (see `SPLIT_COLS`); this is the explicit toggle on top of that.
    zoom: bool,
    /// the dependency canvas, when open (see `Canvas`)
    canvas: Option<Canvas>,
    /// bucket key -> the header text for it. What the keys *are* depends on
    /// `mode`: group ids in hunk mode, `L<n>` ledger keys in ledger mode.
    groups: HashMap<String, String>,
    /// the engine's `Group::reason` map, kept unchanged so `:mode hunks` can
    /// restore it without re-touching the engine
    group_reasons: HashMap<String, String>,
    /// what the reading-order list is a list of — ledger by default
    mode: ViewMode,
    /// how each item differs from the previous run of this review
    deltas: Vec<Delta>,
    /// keys present last run but gone now, for the `:delta` summary
    delta_gone: usize,
    /// symbol-anchored review notes, key -> text (see `note_key`)
    notes: HashMap<u64, String>,
    /// where notes get persisted; `None` when the cache dir can't be resolved
    notes_path: Option<PathBuf>,
    /// the engine's change ledger, kept so `:mode` can rebuild the headers.
    /// Named apart from `ledger`, which is the unrelated `:audit` ledger.
    symbol_ledger: Vec<ordo::model::LedgerEntry>,
    /// group ids whose hunks are folded away under their header (`za` and
    /// friends); empty means everything is expanded
    collapsed: HashSet<String>,
    /// what the filters and the engine dropped on the way here — `:audit`
    ledger: Ledger,
    /// what `:rules` shows: where the rules came from, what was replaced or disabled
    rules_report: Vec<String>,
    /// the reviewer's own rules, carried so a re-order or an `:e` reload keeps
    /// applying them
    rules: Vec<ordo::model::Rule>,
    /// the ordering strategy the current `items`/`view` were built with
    /// (`"comprehension"`, `"defs-first"`, `"file"`) — set by `:strategy`,
    /// otherwise the engine's own default; carried into `:quickfix`'s
    /// exported `context.strategy` so a `:cnext` session in the editor knows
    /// what order it's walking.
    strategy: String,
    /// per-path longest-line cache, so `draw`'s `hscroll` clamp doesn't rescan
    /// the whole file every frame — filled in lazily, keyed by `Item::path`
    max_col: HashMap<String, usize>,
}

/// Bound on the position stack `JumpToEdge`/`JumpBack` maintain — generous
/// (vim's own default `'jumps'` is 100) without growing unbounded across a
/// long review session.
const JUMP_STACK_CAP: usize = 50;

/// Push a position, dropping the oldest entry first if the stack is already
/// at `JUMP_STACK_CAP`.
fn stack_push(stack: &mut Vec<(usize, Cursor)>, pos: (usize, Cursor)) {
    if stack.len() >= JUMP_STACK_CAP {
        stack.remove(0);
    }
    stack.push(pos);
}

/// Pop the most recent position whose item index still resolves against
/// `n_items` — skipping (never panicking on) any entry pointing at a hunk
/// index that's since gone out of range, so the stack survives even a jump to
/// a hunk that later becomes unreachable. `None` on an empty (or exhausted)
/// stack.
fn stack_pop_valid(stack: &mut Vec<(usize, Cursor)>, n_items: usize) -> Option<(usize, Cursor)> {
    while let Some(pos) = stack.pop() {
        if pos.0 < n_items {
            return Some(pos);
        }
    }
    None
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

fn build_items(out: &Output) -> Vec<Item> {
    let (ledger_at, ledger_named) = ledger_index(out);
    let by_id: HashMap<&str, (&str, &ordo::model::HunkOut)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks
                .iter()
                .map(move |h| (h.id.as_str(), (f.path.as_str(), h)))
        })
        .collect();
    // hunk id -> its index in the final `items` vec, so an edge can carry a
    // resolved target instead of just the id string it's thrown away for.
    // Same filter as the `filter_map` below (a hunk id absent from `by_id`
    // is skipped there too), so the index lines up with the vec it produces.
    let item_index: HashMap<&str, usize> = out
        .order
        .iter()
        .filter(|o| by_id.contains_key(o.hunk.as_str()))
        .enumerate()
        .map(|(i, o)| (o.hunk.as_str(), i))
        .collect();
    // hunk id -> 1-based cluster number, only when there's more than one
    // cluster to distinguish — a single cluster covering everything isn't
    // worth tagging every item with.
    let cluster_of: HashMap<&str, usize> = if out.clusters.len() > 1 {
        out.clusters
            .iter()
            .enumerate()
            .flat_map(|(i, ids)| ids.iter().map(move |id| (id.as_str(), i + 1)))
            .collect()
    } else {
        HashMap::new()
    };
    out.order
        .iter()
        .filter_map(|o| {
            let (path, h) = by_id.get(o.hunk.as_str())?;
            // a catalog entry is worth the mark at any level — v1 marked every
            // advisory — while a rule only earns it at `warn` or above
            let flagged = h.findings.iter().any(|f| {
                f.source == ordo::model::FindingSource::Catalog
                    || f.level != ordo::model::Level::Note
            });
            let mark = if flagged {
                "⚠ "
            } else if h.noise {
                "· "
            } else {
                ""
            };
            // the entry anchored here, else the one for the definition holding
            // this hunk — only a *definition* container, since a test block or
            // a region names no symbol
            let ledger = ledger_at.get(h.id.as_str()).copied().or_else(|| {
                let name = h
                    .enclosing
                    .as_deref()
                    .filter(|_| h.enclosing_kind.is_none())?;
                let bare = name.rsplit('.').next().unwrap_or(name);
                ledger_named
                    .get(&(*path, name.to_string()))
                    .or_else(|| ledger_named.get(&(*path, bare.to_string())))
                    .copied()
            });
            Some(Item {
                path: path.to_string(),
                bucket: bucket_key(ViewMode::Ledger, ledger, &h.group),
                ledger,
                old_range: h.old_range,
                new_range: h.new_range,
                mark: mark.to_string(),
                cat: h.category,
                rationale: h.rationale.clone(),
                details: h.details.clone(),
                notes: h.notes.clone(),
                edges: hunk_edges(out, h, &by_id, &item_index),
                noise: h.noise,
                comment: h.comment,
                symbols: h.symbols.clone(),
                enclosing: h.enclosing.clone(),
                group: h.group.clone(),
                findings: h.findings.clone(),
                uses_at: h.uses_at.clone(),
                executed: None,
                refined: ordo::refine::Refined::default(),
                cluster: cluster_of.get(h.id.as_str()).copied(),
            })
        })
        .collect()
}

/// hunk id → ledger entry anchored to it, and (path, name) → the entry for
/// that symbol.
type LedgerIndex<'a> = (HashMap<&'a str, usize>, HashMap<(&'a str, String), usize>);

fn ledger_index(out: &Output) -> LedgerIndex<'_> {
    let ledger_at: HashMap<&str, usize> = out
        .ledger
        .iter()
        .enumerate()
        .map(|(i, e)| (e.at.as_str(), i))
        .collect();
    // (path, name) -> the ledger entry for that symbol, under both the bare
    // name and the scope-qualified form a hunk's `enclosing` uses. A symbol's
    // entry anchors to the first hunk that touches it, so the rest of its
    // hunks find it here instead of falling into the "no definition changed"
    // bucket — they *are* that symbol's change, just not its first hunk.
    let mut ledger_named: HashMap<(&str, String), usize> = HashMap::new();
    for (i, e) in out.ledger.iter().enumerate() {
        ledger_named
            .entry((e.path.as_str(), e.name.clone()))
            .or_insert(i);
        if let Some(scope) = &e.scope {
            ledger_named
                .entry((e.path.as_str(), format!("{scope}.{}", e.name)))
                .or_insert(i);
        }
    }
    (ledger_at, ledger_named)
}

/// the def→use edges touching `h`, each labelled from this hunk's side
fn hunk_edges(
    out: &Output,
    h: &ordo::model::HunkOut,
    by_id: &HashMap<&str, (&str, &ordo::model::HunkOut)>,
    item_index: &HashMap<&str, usize>,
) -> Vec<EdgeRef> {
    let loc = |id: &str| {
        by_id
            .get(id)
            .map(|(p, h)| format!("{p}:L{}", h.new_range[0]))
            .unwrap_or_else(|| id.to_string())
    };
    out.edges
        .iter()
        .filter(|e| e.from == h.id || e.to == h.id)
        .map(|e| {
            let defines_it = e.from == h.id;
            let (label, target_id) = if defines_it {
                (format!("→ {}   {}", loc(&e.to), e.why), e.to.as_str())
            } else {
                (format!("← {}   {}", loc(&e.from), e.why), e.from.as_str())
            };
            EdgeRef {
                label,
                target: item_index.get(target_id).copied(),
                dependency: !defines_it,
            }
        })
        .collect()
}

/// Fill in each item's intra-line refinement. One `Refiner` per file, not per
/// hunk: it parses both sides, which is the expensive part, and a file usually
/// carries several hunks.
fn refine_items(items: &mut [Item], sources: &Sources) {
    let mut by_path: HashMap<String, Option<ordo::refine::Refiner>> = HashMap::new();
    for it in items.iter_mut() {
        let Some((ol, nl)) = sources.get(&it.path) else {
            continue;
        };
        let refiner = by_path
            .entry(it.path.clone())
            .or_insert_with(|| ordo::refine::Refiner::new(&it.path, ol, nl));
        if let Some(r) = refiner {
            it.refined = r.refine(it.old_range, it.new_range);
        }
    }
}

/// Which of `items` are currently visible, in order — the client-side view a
/// filter command narrows without touching `items` itself, so a filter can
/// always be widened back without re-running the engine or re-fetching
/// anything. Pure, so filter-change tests don't need a live `App`.
fn compute_view(
    items: &[Item],
    comments_only: bool,
    show_all: bool,
    glob: Option<&PathGlobs>,
) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, it)| !comments_only || it.comment)
        .filter(|(_, it)| show_all || !it.noise)
        .filter(|(_, it)| glob.is_none_or(|g| g.is_match(&it.path)))
        .map(|(i, _)| i)
        .collect()
}

/// Why each currently-hidden item is hidden, attributed in `compute_view`'s own
/// order so the counts partition the hidden set exactly (an item hidden by two
/// filters is charged to the first). `unaccounted` must always be zero: it
/// counts items `compute_view` rejected for a reason this function does not
/// know about, which can only be a bug.
#[derive(Default, Debug, PartialEq, Eq)]
struct Hidden {
    comment: usize,
    noise: usize,
    glob: usize,
    unaccounted: usize,
}

fn hidden_breakdown(
    items: &[Item],
    comments_only: bool,
    show_all: bool,
    glob: Option<&PathGlobs>,
) -> Hidden {
    let mut h = Hidden::default();
    for it in items {
        if comments_only && !it.comment {
            h.comment += 1;
        } else if !show_all && it.noise {
            h.noise += 1;
        } else if glob.is_some_and(|g| !g.is_match(&it.path)) {
            h.glob += 1;
        }
    }
    let shown = compute_view(items, comments_only, show_all, glob).len();
    h.unaccounted = items.len() - shown - h.comment - h.noise - h.glob;
    h
}

// ---------------------------------------------------------- group headers

/// One row of the reading-order list's display, once `:group` is on: a
/// non-selectable header naming a group's reason, or a hunk (an index into
/// `App.items`, same as a plain `view` entry).
enum DisplayRow {
    Header(String),
    Item(usize),
}

/// The reading-order list's rows, in render order. `view` itself never grows
/// a header entry — it stays exactly the indices `compute_view` produced, so
/// navigation (`step`/`select`/`first_visible`/`last_visible`, all of which
/// walk `view`) can never land `sel` on one; this is purely a display-time
/// overlay, walked in lockstep by `display_row_of` below so a header can
/// never appear in one function's output but not the other's index math.
/// With `:group` off, it's `view` unchanged (one `DisplayRow::Item` each).
fn display_rows(
    view: &[usize],
    items: &[Item],
    groups: &HashMap<String, String>,
    show_groups: bool,
    collapsed: &HashSet<String>,
) -> Vec<DisplayRow> {
    if !show_groups {
        return view.iter().map(|&i| DisplayRow::Item(i)).collect();
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for &i in view {
        *counts.entry(items[i].bucket.as_str()).or_insert(0) += 1;
    }
    let mut rows = Vec::with_capacity(view.len());
    let mut last: Option<&str> = None;
    for &i in view {
        let gid = items[i].bucket.as_str();
        let folded = collapsed.contains(gid);
        if last != Some(gid) {
            let reason = groups.get(gid).map(String::as_str).unwrap_or(gid);
            let n = counts.get(gid).copied().unwrap_or(0);
            // a folded group still says how much it is hiding — otherwise the
            // list silently shrinks and a reviewer can lose track of what is left
            let marker = if folded { "▸" } else { "▾" };
            rows.push(DisplayRow::Header(format!("{marker} {reason} ({n})")));
            last = Some(gid);
        }
        if !folded {
            rows.push(DisplayRow::Item(i));
        }
    }
    rows
}

/// The items a fold state actually shows — `view` minus everything inside a
/// collapsed group. The selected hunk is always kept: folding the group you are
/// standing in moves you to its header, it never leaves the selection pointing
/// at a row that isn't drawn.
fn folded_view(app: &App) -> Vec<usize> {
    if !app.show_groups || app.collapsed.is_empty() {
        return app.view.clone();
    }
    let visible: Vec<usize> = app
        .view
        .iter()
        .copied()
        .filter(|&i| !app.collapsed.contains(&app.items[i].bucket))
        .collect();
    if visible.is_empty() {
        app.view.clone()
    } else {
        visible
    }
}

/// The key a hunk groups under in a given mode. In ledger mode a hunk with no
/// symbol change anchored to it falls into one shared bucket rather than
/// vanishing — an import or a formatting hunk is still part of the review.
fn bucket_key(mode: ViewMode, ledger: Option<usize>, group: &str) -> String {
    match (mode, ledger) {
        (ViewMode::Hunks, _) => group.to_string(),
        (ViewMode::Ledger, Some(i)) => format!("L{i}"),
        (ViewMode::Ledger, None) => "L-".to_string(),
    }
}

/// Switch what the list is a list of. Re-keys every item and rebuilds the
/// header labels; headers are forced on in ledger mode, where they *are* the
/// ledger. Fold state is dropped because its keys belonged to the old mode.
fn set_mode(app: &mut App, mode: ViewMode, ledger: &[ordo::model::LedgerEntry]) {
    app.mode = mode;
    for it in &mut app.items {
        it.bucket = bucket_key(mode, it.ledger, &it.group);
    }
    if mode == ViewMode::Ledger {
        let mut labels: HashMap<String, String> = ledger
            .iter()
            .enumerate()
            .map(|(i, e)| (format!("L{i}"), ledger_label(e)))
            .collect();
        labels.insert("L-".to_string(), "no definition changed".to_string());
        app.groups = labels;
        app.show_groups = true;
    } else {
        app.groups = app.group_reasons.clone();
    }
    app.collapsed.clear();
}

/// Fold state change for the group holding the selected hunk (or every group).
fn fold(app: &mut App, how: Fold) {
    // folding is a statement about groups: turning the headers on is what the
    // reviewer meant, not an error to report
    app.show_groups = true;
    let gid = app.items[app.sel].group.clone();
    match how {
        Fold::Toggle if app.collapsed.contains(&gid) => {
            app.collapsed.remove(&gid);
        }
        Fold::Toggle | Fold::Close => {
            app.collapsed.insert(gid);
        }
        Fold::Open => {
            app.collapsed.remove(&gid);
        }
        Fold::CloseAll => {
            app.collapsed = app.items.iter().map(|it| it.group.clone()).collect();
        }
        Fold::OpenAll => app.collapsed.clear(),
    }
    // folding one group steps off it, so the selection stays on a hunk rather
    // than on a header. Folding *everything* leaves nowhere to step to: the
    // selection then stays put and its header carries the highlight.
    let all_folded = app
        .view
        .iter()
        .all(|&i| app.collapsed.contains(&app.items[i].group));
    if !all_folded && app.collapsed.contains(&app.items[app.sel].group) {
        let visible = folded_view(app);
        if let Some(&next) = visible.iter().find(|&&i| i >= app.sel).or(visible.last()) {
            select(app, next);
        }
    }
}

/// The row index `display_rows` would give the hunk at `view[pos]` — how many
/// header rows precede it, plus its own position — so `ListState::select`
/// always points at an `Item` row, never a `Header` one. Doesn't need
/// `groups` (only `display_rows` renders a header's text) — same header
/// *placement* rule as `display_rows`, is all this needs to agree with it.
fn display_row_of(
    view: &[usize],
    items: &[Item],
    show_groups: bool,
    collapsed: &HashSet<String>,
    pos: usize,
) -> usize {
    if !show_groups {
        return pos;
    }
    let mut row: usize = 0;
    let mut last: Option<&str> = None;
    for (i, &vi) in view.iter().enumerate() {
        let gid = items[vi].bucket.as_str();
        let mut header_row = None;
        if last != Some(gid) {
            header_row = Some(row);
            row += 1;
            last = Some(gid);
        }
        if i == pos {
            // inside a folded group the hunk itself isn't drawn: the highlight
            // belongs on its header, the way vim highlights the fold line the
            // cursor is inside
            return match collapsed.contains(gid) {
                true => header_row.unwrap_or(row.saturating_sub(1)),
                false => row,
            };
        }
        // a folded group draws its header and nothing else
        if !collapsed.contains(gid) {
            row += 1;
        }
    }
    row
}

// position the code view so the hunk sits a few lines below the top
fn auto_scroll(it: &Item) -> u16 {
    let [o0, o1] = it.old_range;
    let removed = if o0 >= 1 && o0 <= o1 { o1 - o0 + 1 } else { 0 };
    ((it.new_range[0].saturating_sub(1) + removed).saturating_sub(3)) as u16
}

// where a freshly-selected hunk puts the cursor: its first changed new line
fn start_cursor(it: &Item) -> Cursor {
    Cursor {
        line: it.new_range[0].saturating_sub(1),
        col: 0,
    }
}

fn cursor_for(it: &Item, sources: &Sources) -> Cursor {
    match sources.get(&it.path) {
        Some((_, nl)) if !nl.is_empty() => clamp_cursor(start_cursor(it), nl),
        _ => Cursor { line: 0, col: 0 },
    }
}

// ------------------------------------------------------------------ cursor motion

// the last valid column on a line — 0 for an empty line, otherwise its last char
fn col_max(line: &str) -> usize {
    line.chars().count().saturating_sub(1)
}

fn clamp_cursor(mut c: Cursor, lines: &[String]) -> Cursor {
    if lines.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    c.line = c.line.min(lines.len() - 1);
    c.col = c.col.min(col_max(&lines[c.line]));
    c
}

fn move_col(c: Cursor, lines: &[String], delta: isize) -> Cursor {
    let max = col_max(&lines[c.line]);
    let col = (c.col as isize + delta).clamp(0, max as isize) as usize;
    Cursor { line: c.line, col }
}

fn move_line(c: Cursor, lines: &[String], delta: isize) -> Cursor {
    let last = lines.len() - 1;
    let line = (c.line as isize + delta).clamp(0, last as isize) as usize;
    Cursor {
        line,
        col: c.col.min(col_max(&lines[line])),
    }
}

fn line_start(c: Cursor, _lines: &[String]) -> Cursor {
    Cursor {
        line: c.line,
        col: 0,
    }
}

fn line_end(c: Cursor, lines: &[String]) -> Cursor {
    Cursor {
        line: c.line,
        col: col_max(&lines[c.line]),
    }
}

// { / } — nearest blank line, vim-style paragraph motion
fn para_prev(c: Cursor, lines: &[String]) -> Cursor {
    if c.line == 0 {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = c.line - 1;
    while i > 0 && !lines[i].trim().is_empty() {
        i -= 1;
    }
    Cursor { line: i, col: 0 }
}

fn para_next(c: Cursor, lines: &[String]) -> Cursor {
    let mut i = c.line + 1;
    while i < lines.len() && !lines[i].trim().is_empty() {
        i += 1;
    }
    Cursor {
        line: i.min(lines.len() - 1),
        col: 0,
    }
}

// word motion (w/b/e) is expressed over a single flattened char index — the
// file's lines joined by '\n' — so a word run crossing a line boundary is just
// a class transition, the same as one crossing whitespace mid-line.
fn word_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

fn joined_chars(lines: &[String]) -> Vec<char> {
    let mut v = vec![];
    for (i, l) in lines.iter().enumerate() {
        v.extend(l.chars());
        if i + 1 < lines.len() {
            v.push('\n');
        }
    }
    v
}

fn to_index(lines: &[String], c: Cursor) -> usize {
    let mut idx = 0;
    for l in &lines[..c.line] {
        idx += l.chars().count() + 1;
    }
    idx + c.col
}

fn from_index(lines: &[String], mut idx: usize) -> Cursor {
    for (i, l) in lines.iter().enumerate() {
        let len = l.chars().count();
        if idx <= len {
            return Cursor { line: i, col: idx };
        }
        idx -= len + 1;
    }
    let last = lines.len() - 1;
    Cursor {
        line: last,
        col: lines[last].chars().count(),
    }
}

fn word_next(c: Cursor, lines: &[String]) -> Cursor {
    let chars = joined_chars(lines);
    if chars.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = to_index(lines, c).min(chars.len() - 1);
    let cls = word_class(chars[i]);
    if cls != 0 {
        while i + 1 < chars.len() && word_class(chars[i + 1]) == cls {
            i += 1;
        }
    }
    while i + 1 < chars.len() && word_class(chars[i + 1]) == 0 {
        i += 1;
    }
    from_index(lines, (i + 1).min(chars.len() - 1))
}

fn word_prev(c: Cursor, lines: &[String]) -> Cursor {
    let chars = joined_chars(lines);
    if chars.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = to_index(lines, c).min(chars.len() - 1);
    if i == 0 {
        return Cursor { line: 0, col: 0 };
    }
    i -= 1;
    while i > 0 && word_class(chars[i]) == 0 {
        i -= 1;
    }
    let cls = word_class(chars[i]);
    while i > 0 && word_class(chars[i - 1]) == cls {
        i -= 1;
    }
    from_index(lines, i)
}

fn word_end(c: Cursor, lines: &[String]) -> Cursor {
    let chars = joined_chars(lines);
    if chars.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = to_index(lines, c).min(chars.len() - 1);
    if i + 1 >= chars.len() {
        return from_index(lines, i);
    }
    i += 1;
    while i < chars.len() && word_class(chars[i]) == 0 {
        i += 1;
    }
    if i >= chars.len() {
        return from_index(lines, chars.len() - 1);
    }
    let cls = word_class(chars[i]);
    while i + 1 < chars.len() && word_class(chars[i + 1]) == cls {
        i += 1;
    }
    from_index(lines, i)
}

// keep `line` inside [scroll, scroll + height) by moving `scroll` the minimum
// amount necessary — the same rule any scrolling editor follows
fn follow_scroll(line: usize, scroll: u16, height: u16) -> u16 {
    let h = height.max(1) as usize;
    let s = scroll as usize;
    if line < s {
        line as u16
    } else if line >= s + h {
        (line + 1 - h) as u16
    } else {
        scroll
    }
}

// horizontal counterpart of `follow_scroll` — same rule, over columns and a
// pane's visible code-column width instead of rows and its visible height
fn follow_hscroll(col: usize, hscroll: u16, width: u16) -> u16 {
    follow_scroll(col, hscroll, width)
}

/// While loading: just a status line under the pane border, same idiom as
/// the review panes. Nothing else can be drawn yet — no items, no sources.
/// The screen before the review arrives. It used the terminal's own colours and
/// square borders while every pane that follows is themed and rounded, so the
/// app visibly changed shape the moment it finished loading.
fn draw_loading(f: &mut Frame, rev: &str, status: &str, theme: &Theme) {
    let area = f.area();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            format!(" {rev} — loading… "),
            Style::default().fg(theme.dim),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let p = Paragraph::new(Text::from(vec![Line::from(Span::styled(
        status.to_string(),
        Style::default().fg(theme.fg),
    ))]));
    f.render_widget(p, inner);
}

enum State {
    Loading(String),
    Ready(Box<App>),
}

struct Session {
    rev: String,
    review_sha: Option<String>,
    uncommitted: bool,
    /// taken by the `App` a load builds, set again before every reload back
    /// into `Loading`
    keys: Option<Keymap>,
    theme: Theme,
    docs_last: bool,
    only_comments: bool,
    /// kept around (rather than consumed by the first load) so `:e` can spawn
    /// a fresh worker later without re-parsing CLI globs — the launch-time
    /// path filter carries across a reload, same as the keymap and theme
    base_filter: Filter,
    rules: RuleSet,
    rules_report: Vec<String>,
    sarif: Vec<String>,
    coverage: Vec<String>,
}

impl Session {
    /// A worker loading `target` under the session's current settings.
    fn spawn_load(&self, target: Target) -> mpsc::Receiver<LoadMsg> {
        let (tx, rx) = mpsc::channel();
        let spec = LoadSpec {
            docs_last: self.docs_last,
            target,
            filter: self.base_filter.clone(),
            only_comments: self.only_comments,
            rev: self.rev.clone(),
            syn: self.theme.syn,
            rules: self.rules.clone(),
            sarif: self.sarif.clone(),
            coverage: self.coverage.clone(),
        };
        thread::spawn(move || load(spec, tx));
        rx
    }

    /// `:e`: everything indexing the *old* review — selection, cursor, scroll,
    /// search, the jump stack, any open popup — is dropped by simply not
    /// carrying `app` forward into the new `App` the next `Done` builds (same
    /// as a fresh launch); the reviewed marks come back from the on-disk cache
    /// keyed by the new rev (see `mark_key`/`load`), not from `app.marks`. The
    /// keymap and theme are carried forward — display preferences independent
    /// of which hunks are loaded — and the path filter is re-read from
    /// `base_filter` rather than `app.path_filter`, so a live `:filter` resets
    /// along with everything else.
    fn reload(&mut self, app: &App, target: Target, new_rev: String) -> mpsc::Receiver<LoadMsg> {
        let (carried_keys, carried_theme) = carry_across_reload(app);
        self.review_sha = review_commit_sha(&target);
        self.uncommitted = matches!(target, Target::Uncommitted | Target::WorktreeRange(_));
        self.rev = new_rev;
        self.keys = Some(carried_keys);
        self.theme = carried_theme;
        self.spawn_load(target)
    }

    /// The app for a review that just loaded.
    fn fresh_app(&mut self, r: LoadResult) -> App {
        let LoadResult {
            items,
            changes,
            file_churn,
            view,
            comments_only,
            sources,
            highlights,
            timing: _,
            reviewed,
            marks_path,
            marks,
            groups,
            ledger,
            symbol_ledger,
            notes,
            notes_path,
            deltas,
            delta_gone,
        } = r;
        let sel0 = view[0];
        let scroll = auto_scroll(&items[sel0]);
        let cursor = cursor_for(&items[sel0], &sources);
        let mut fresh = App {
            reviewed,
            items,
            changes,
            docs_last: self.docs_last,
            view,
            comments_only,
            show_all: true,
            path_filter: None,
            command: None,
            sel: sel0,
            scroll,
            hscroll: 0,
            why_scroll: 0,
            why_sel: 0,
            code_len: 0,
            why_len: 0,
            code_height: 10,
            why_height: 10,
            code_width: 10,
            focus: Pane::List,
            keys: self
                .keys
                .take()
                .expect("keys is set again before every reload back into Loading"),
            pending: None,
            sources,
            highlights,
            cursor,
            popup: None,
            trees: HashMap::new(),
            prompt: None,
            search: None,
            review_sha: self.review_sha.clone(),
            uncommitted: self.uncommitted,
            history_cache: HashMap::new(),
            churn_cache: HashMap::new(),
            file_churn,
            jumps: Vec::new(),
            rev: self.rev.clone(),
            marks_path,
            marks,
            theme: self.theme,
            show_groups: false,
            zoom: false,
            canvas: None,
            group_reasons: groups.clone(),
            groups,
            mode: ViewMode::default(),
            symbol_ledger,
            notes,
            notes_path,
            deltas,
            delta_gone,
            collapsed: HashSet::new(),
            ledger,
            rules: self.rules.rules.clone(),
            catalog: self.rules.catalog,
            disables: self.rules.disables.clone(),
            includes: self.rules.includes.clone(),
            config: None,
            strategy: "comprehension".to_string(),
            rules_report: self.rules_report.clone(),
            max_col: HashMap::new(),
        };
        // the list is a list of *symbols* by default; `:mode` switches it
        // back to hunks
        let led = fresh.symbol_ledger.clone();
        set_mode(&mut fresh, ViewMode::Ledger, &led);
        fresh
    }
}

/// A key while `:config` is open, which owns its keys rather than going
/// through the keymap: its hints have to be true whatever preset is loaded,
/// and changing the preset is one of the settings — going through the map
/// would rebind the view mid-edit. True when the key was the config's.
fn config_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    let Some(c) = app.config.as_mut() else {
        return false;
    };
    if let Some(text) = c.editing.as_mut() {
        match (code, mods) {
            (KeyCode::Enter, _) => config_commit_edit(app),
            (KeyCode::Esc, _) => c.editing = None,
            (KeyCode::Backspace, _) => {
                text.pop();
            }
            (KeyCode::Char(ch), m) if !m.contains(KeyModifiers::CONTROL) => text.push(ch),
            _ => {}
        }
        return true;
    }
    match (code, mods) {
        (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => app.config = None,
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => move_config(app, 1),
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => move_config(app, -1),
        (KeyCode::PageDown, _) => move_config(app, PAGE as isize),
        (KeyCode::PageUp, _) => move_config(app, -(PAGE as isize)),
        // `g`/`G` as well as Home/End: the rest of the app answers to them
        // and the fingers do not re-learn
        (KeyCode::Home, _) | (KeyCode::Char('g'), _) => move_config(app, isize::MIN / 2),
        (KeyCode::End, _) | (KeyCode::Char('G'), _) => move_config(app, isize::MAX / 2),
        (KeyCode::Char(' '), _) | (KeyCode::Enter, _) => config_toggle(app),
        (KeyCode::Char('w'), _) => config_write(app),
        _ => {}
    }
    true
}

/// A key while the search prompt is open: every key edits the prompt instead
/// of running an action, so chords (C-w C-w, gg) can't leak in. True when the
/// prompt took it.
fn prompt_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    if app.prompt.is_none() {
        return false;
    }
    match (code, mods) {
        (KeyCode::Enter, _) => accept_search(app),
        (KeyCode::Esc, _) => cancel_search(app),
        (KeyCode::Backspace, _) => {
            if let Some(p) = app.prompt.as_mut() {
                p.text.pop();
            }
        }
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
            if let Some(p) = app.prompt.as_mut() {
                p.text.push(c);
            }
        }
        _ => {}
    }
    true
}

/// A key against the loaded review, after the modal owners (config, prompt,
/// command bar) had their say. True means quit.
fn review_key(
    app: &mut App,
    code: KeyCode,
    mods: KeyModifiers,
    terminal: &mut ratatui::DefaultTerminal,
) -> bool {
    let key = norm(code, mods);
    match app.keys.resolve(app.pending, key) {
        Resolve::Pending => app.pending = Some(key),
        Resolve::Miss => app.pending = None,
        // handled here rather than in `apply`: it must leave the alternate
        // screen for the editor and come back
        Resolve::Act(Action::OpenEditor) => {
            app.pending = None;
            open_editor(app, terminal);
        }
        Resolve::Act(a) => {
            app.pending = None;
            // Paging and scrolling need the pane heights, which only the
            // renderer used to know — so a key pressed before the first draw
            // used the placeholder 10, and every key after a resize used the
            // previous frame's size. Both are computed here, from the same
            // function the renderer uses.
            if let Ok(size) = terminal.size() {
                set_geometry(app, Rect::new(0, 0, size.width, size.height));
            }
            return apply(app, a);
        }
    }
    false
}

/// Take the screen first, then load off the draw loop: a background thread
/// runs `gather`/`highlight_file`/`ordo::run` and reports progress over
/// `LoadMsg`, while this loop keeps drawing and polling input so `q`/`Esc`
/// (or `C-q`/`Esc`) abort a slow load without waiting for the worker. A
/// custom panic hook restores the terminal first even if the worker (or a
/// later draw) panics, so a broken terminal never outlives the crash.
/// What a review session keeps across reloads: the launch-time inputs a
/// fresh worker needs, and the display preferences that outlive one review.
#[allow(clippy::too_many_arguments)]
fn run(
    rev: String,
    keys: Keymap,
    docs_last: bool,
    target: Target,
    filter: Filter,
    only_comments: bool,
    review_sha: Option<String>,
    uncommitted: bool,
    theme: Theme,
    rules: RuleSet,
    rules_report: Vec<String>,
    sarif: Vec<String>,
    coverage: Vec<String>,
) -> std::io::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        prev_hook(info);
    }));

    let mut terminal = ratatui::init();
    let mut session = Session {
        rev,
        review_sha,
        uncommitted,
        keys: Some(keys),
        theme,
        docs_last,
        only_comments,
        base_filter: filter,
        rules,
        rules_report,
        sarif,
        coverage,
    };
    let mut rx = session.spawn_load(target);
    let mut state = State::Loading("starting…".to_string());
    let mut timing: Option<String> = None;
    let mut post_msg: Option<String> = None;
    // Nothing on screen changes on its own once the review is up, so a frame is
    // only worth painting after something moved: a worker message, or an event.
    // Without this the 50ms poll below doubles as a 20fps repaint of a review
    // nobody is touching.
    let mut dirty = true;
    let result: std::io::Result<()> = 'outer: loop {
        if let Some(msg) = drain_worker(&rx, &mut session, &mut state, &mut timing, &mut dirty) {
            post_msg = Some(msg);
            break 'outer Ok(());
        }

        if dirty {
            if let Err(e) = terminal.draw(|f| match &mut state {
                State::Loading(status) => draw_loading(f, &session.rev, status, &session.theme),
                State::Ready(app) => draw(f, app, &session.rev),
            }) {
                break 'outer Err(e);
            }
            dirty = false;
        }

        match event::poll(Duration::from_millis(50)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => break 'outer Err(e),
        }
        // Any event at all — a key, but also a resize the next frame has to
        // relayout for — means the screen is stale.
        dirty = true;
        let k = match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => k,
            Ok(_) => continue,
            Err(e) => break 'outer Err(e),
        };
        let app = match &mut state {
            State::Loading(_) => {
                let keys = session
                    .keys
                    .as_ref()
                    .expect("keys not yet taken while Loading");
                if let Resolve::Act(Action::Quit) = keys.resolve(None, norm(k.code, k.modifiers)) {
                    break 'outer Ok(());
                }
                continue;
            }
            State::Ready(app) => app,
        };
        if config_key(app, k.code, k.modifiers) || prompt_key(app, k.code, k.modifiers) {
            continue;
        }
        // the `:` command bar intercepts every key the same way the prompt
        // does, instead of resolving through the keymap
        if app.command.is_some() {
            match handle_command_key(app, k.code, k.modifiers) {
                CommandOutcome::Quit => break 'outer Ok(()),
                CommandOutcome::None => {}
                CommandOutcome::OpenQuickfix(path) => {
                    open_quickfix_editor(app, &mut terminal, &path);
                }
                CommandOutcome::Reload(target, new_rev) => {
                    rx = session.reload(app, target, new_rev.clone());
                    state = State::Loading(format!("switching to {new_rev}…"));
                }
            }
            continue;
        }
        if review_key(app, k.code, k.modifiers, &mut terminal) {
            break 'outer Ok(());
        }
    };
    ratatui::restore();
    result?;
    // The TUI takes the alternate screen, so whichever of these applies
    // stays on the shell's scrollback and is what's left on screen after
    // quitting — the same visible behaviour as printing it before init.
    if let Some(msg) = post_msg {
        eprintln!("{msg}");
    } else if let Some(t) = timing {
        eprintln!("{t}");
    }
    Ok(())
}

/// Every message the worker has queued. `Some(msg)` ends the session before
/// a review is up: an empty change set, or a worker that dropped its sender
/// without a Done/Empty — only possible if it panicked; abort rather than spin.
fn drain_worker(
    rx: &mpsc::Receiver<LoadMsg>,
    session: &mut Session,
    state: &mut State,
    timing: &mut Option<String>,
    dirty: &mut bool,
) -> Option<String> {
    loop {
        match rx.try_recv() {
            Ok(LoadMsg::Progress(s)) => {
                *dirty = true;
                if let State::Loading(status) = state {
                    *status = s;
                }
            }
            Ok(LoadMsg::Empty(msg)) => return Some(msg),
            Ok(LoadMsg::Done(r)) => {
                *dirty = true;
                let mut r = *r;
                *timing = Some(std::mem::take(&mut r.timing));
                *state = State::Ready(Box::new(session.fresh_app(r)));
            }
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                return matches!(state, State::Loading(_))
                    .then(|| "ordo: loading failed unexpectedly".to_string());
            }
        }
    }
}

/// Run one action against the focused pane; true means quit.
fn apply(app: &mut App, a: Action) -> bool {
    if app.popup.is_some() {
        apply_in_popup(app, a);
        return false;
    }
    if app.canvas.is_some() && app.focus == Pane::Deps {
        apply_on_canvas(app, a);
        return false;
    }
    if app.focus == Pane::Code && apply_in_code(app, a) {
        return false;
    }
    apply_anywhere(app, a)
}

/// An open popup consumes input itself: Esc/q dismiss it (without quitting),
/// Next/Prev/PageUp/PageDown scroll its own body rather than the pane
/// underneath, and everything else is a no-op while it's up.
fn apply_in_popup(app: &mut App, a: Action) {
    match a {
        Action::Quit => app.popup = None,
        Action::Next => popup_scroll(app, 1),
        Action::Prev => popup_scroll(app, -1),
        Action::PageDown => popup_scroll(app, PAGE as isize),
        Action::PageUp => popup_scroll(app, -(PAGE as isize)),
        Action::ScrollLeft => popup_hscroll(app, -1),
        Action::ScrollRight => popup_hscroll(app, 1),
        _ => {}
    }
}

/// The canvas is a floating view with its own selection: j/k move between
/// cards, Enter goes to one, Esc closes. Anything else is a no-op rather than
/// leaking through to the pane underneath.
fn apply_on_canvas(app: &mut App, a: Action) {
    match a {
        Action::Quit => close_deps(app),
        Action::Next => move_card(app, 1),
        Action::Prev => move_card(app, -1),
        Action::First => move_card(app, isize::MIN / 2),
        Action::Last => move_card(app, isize::MAX / 2),
        Action::JumpToEdge => jump_to_card(app),
        Action::Focus(p) if p != Pane::Deps => {
            close_deps(app);
            app.focus = p;
        }
        _ => {}
    }
}

/// The actions that only mean something with the code pane focused: cursor
/// motion, search, horizontal scroll. `false` for any other action.
fn apply_in_code(app: &mut App, a: Action) -> bool {
    match a {
        Action::CursorLeft => cursor_move(app, |c, lines| move_col(c, lines, -1)),
        Action::CursorRight => cursor_move(app, |c, lines| move_col(c, lines, 1)),
        Action::WordNext => cursor_move(app, word_next),
        Action::WordPrev => cursor_move(app, word_prev),
        Action::WordEnd => cursor_move(app, word_end),
        Action::ParaPrev => cursor_move(app, para_prev),
        Action::ParaNext => cursor_move(app, para_next),
        Action::MarkPrev => mark_move(app, false),
        Action::MarkNext => mark_move(app, true),
        Action::SearchOpen => {
            app.prompt = Some(Prompt {
                text: String::new(),
                anchor: app.cursor,
            });
        }
        Action::SymbolNext => symbol_search(app, true),
        Action::SymbolPrev => symbol_search(app, false),
        Action::SearchNext => cycle_search(app, 1),
        Action::SearchPrev => cycle_search(app, -1),
        Action::ScrollLeft => {
            app.hscroll = app.hscroll.saturating_sub(1);
        }
        Action::ScrollRight => {
            app.hscroll = app.hscroll.saturating_add(1);
        }
        _ => return false,
    }
    true
}

/// Every other action, whichever pane has focus; `true` means quit.
fn apply_anywhere(app: &mut App, a: Action) -> bool {
    match a {
        // an active search is showing state too: clear its highlights first,
        // the way dismissing a popup does, so Esc after a search doesn't end
        // the review session
        Action::Quit if app.search.is_some() => app.search = None,
        Action::Quit => return true,
        // asking for the pane you are already in means "give me more of it"
        // `4` addresses the canvas whether or not it exists yet, so it opens
        // one rather than focusing a view that is not there
        Action::Focus(Pane::Deps) => open_deps(app),
        Action::Focus(p) if app.focus == p => app.zoom = !app.zoom,
        Action::Focus(p) => app.focus = p,
        Action::Zoom => app.zoom = !app.zoom,
        Action::OpenDeps => open_deps(app),
        Action::FocusNext => app.focus = app.focus.next(),
        Action::FocusPrev => app.focus = app.focus.prev(),
        Action::ToggleReviewed => {
            let i = app.sel;
            app.reviewed[i] = !app.reviewed[i];
            persist_mark(app, i);
            let pos = view_pos(&app.view, i);
            if pos + 1 < app.view.len() {
                select(app, app.view[pos + 1]);
            }
        }
        Action::Next => step(app, 1),
        Action::Prev => step(app, -1),
        Action::PageDown => page(app, PAGE as isize),
        Action::PageUp => page(app, -(PAGE as isize)),
        Action::HalfDown => page(app, (PAGE / 2) as isize),
        Action::HalfUp => page(app, -((PAGE / 2) as isize)),
        Action::First => match app.focus {
            Pane::List => select(app, first_visible(app)),
            Pane::Code => app.scroll = 0,
            Pane::Why | Pane::Deps => app.why_scroll = 0,
        },
        Action::Last => match app.focus {
            Pane::List => select(app, last_visible(app)),
            Pane::Code => app.scroll = last_line(app.code_len),
            Pane::Why | Pane::Deps => app.why_scroll = last_line(app.why_len),
        },
        // column line-end motion: cursor in the code pane, list/why fall back
        // to `First`/`Last`'s meaning so vscode's Home/End still works there
        Action::LineStart => match app.focus {
            Pane::List => select(app, first_visible(app)),
            Pane::Code => cursor_move(app, line_start),
            Pane::Why | Pane::Deps => app.why_scroll = 0,
        },
        Action::LineEnd => match app.focus {
            Pane::List => select(app, last_visible(app)),
            Pane::Code => cursor_move(app, line_end),
            Pane::Why | Pane::Deps => app.why_scroll = last_line(app.why_len),
        },
        Action::Churn => fill_churn(app),
        // `K`/`F12` dispatches on the focused pane rather than adding a
        // second key: the code pane's symbol hover and the why pane's dep
        // preview are the same "show me more about what's under the cursor"
        // gesture, just aimed at a different cursor.
        Action::Hover => match app.focus {
            Pane::Code => hover(app),
            Pane::Why | Pane::Deps => preview_edge(app),
            Pane::List => {}
        },
        Action::Fold(how) => fold(app, how),
        // `apply_in_code` handles these when the code pane has focus; they
        // mean nothing anywhere else
        Action::CursorLeft
        | Action::CursorRight
        | Action::WordNext
        | Action::WordPrev
        | Action::WordEnd
        | Action::ParaPrev
        | Action::ParaNext
        | Action::MarkPrev
        | Action::MarkNext
        | Action::SearchOpen
        | Action::SymbolNext
        | Action::SymbolPrev
        | Action::SearchNext
        | Action::SearchPrev
        | Action::ScrollLeft
        | Action::ScrollRight => {}
        Action::Help => {
            app.popup = Some(Popup::new(
                format!("keybindings — {}", app.keys.name),
                build_help(&app.keys).into_iter().map(prose).collect(),
            ));
        }
        Action::CommandOpen => open_command_bar(app, String::new()),
        Action::CommandGoto => open_command_bar(app, "goto ".to_string()),
        // intercepted in `run` before it reaches here (needs the terminal)
        Action::OpenEditor => {}
        Action::JumpToEdge if app.focus == Pane::Why => jump_to_edge(app),
        Action::JumpToEdge => {} // only meaningful with the why pane focused
        Action::JumpBack => jump_back(app),
    }
    false
}

/// Writes a toggle to the mark cache immediately — not only on quit — so a
/// panic or a killed terminal never loses the session's marks. A no-op when
/// the item's content isn't loaded or `marks_path` never resolved (see
/// `mark_key`/`marks_file_path`); the checkmark still holds for this session
/// either way, only the write is skipped.
fn persist_mark(app: &mut App, i: usize) {
    let Some(key) = mark_key(&app.rev, &app.items[i], &app.sources) else {
        return;
    };
    if app.reviewed[i] {
        app.marks.insert(key, now_unix());
    } else {
        app.marks.remove(&key);
    }
    let Some(path) = app.marks_path.clone() else {
        return;
    };
    prune_marks(&mut app.marks, now_unix());
    save_marks(&path, &app.marks);
}

/// Popup body line with no styling — most popups are prose.
fn prose(s: impl Into<String>) -> Line<'static> {
    Line::from(s.into())
}

/// Widest popup body line, in characters — what `hscroll` clamps against.
fn popup_width(lines: &[Line<'static>]) -> usize {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
}

// scroll the open popup's body sideways by `by` columns, clamped to its content
fn popup_hscroll(app: &mut App, by: isize) {
    if let Some(p) = app.popup.as_mut() {
        let max = last_line(popup_width(&p.lines)) as isize;
        p.hscroll = (p.hscroll as isize + by).clamp(0, max.max(0)) as u16;
    }
}

// scroll the open popup's body by `by` lines, clamped to its content
fn popup_scroll(app: &mut App, by: isize) {
    if let Some(p) = app.popup.as_mut() {
        p.scroll = (p.scroll as isize + by).clamp(0, last_line(p.lines.len()) as isize) as u16;
    }
}

/// Fill `churn_cache` for the selected hunk, so the why pane can show how
/// often these lines have changed before.
///
/// On demand, never during load: `git log -L` costs ~0.2s on a large file (see
/// `hunk_churn`). Cached per (path, range), so a second `H` on the same hunk
/// is instant and a repeat costs nothing.
fn fill_churn(app: &mut App) {
    let it = &app.items[app.sel];
    let key = (it.path.clone(), it.new_range[0], it.new_range[1]);
    if app.churn_cache.contains_key(&key) {
        return;
    }
    let query = churn_query(it, app.uncommitted);
    let churn = app
        .review_sha
        .clone()
        .map(|sha| hunk_churn(&sha, &it.path, &query));
    app.churn_cache.insert(key, churn);
}

/// Step the cursor to the next (or previous) row `uses_at` marks — the same
/// rows the code gutter glyphs. Nothing marked, or nothing left in that
/// direction, leaves the cursor where it is.
fn mark_move(app: &mut App, forward: bool) {
    let mut rows: Vec<usize> = app.items[app.sel]
        .uses_at
        .iter()
        .flat_map(|u| u.rows.iter().map(|r| r.saturating_sub(1)))
        .collect();
    rows.sort_unstable();
    rows.dedup();
    let here = app.cursor.line;
    let target = if forward {
        rows.into_iter().find(|&r| r > here)
    } else {
        rows.into_iter().rev().find(|&r| r < here)
    };
    if let Some(line) = target {
        cursor_move(app, move |_, _| Cursor { line, col: 0 });
    }
}

/// Move the code cursor with `f`, clamp it to the file, and scroll to follow.
/// No-op when the selected item's file content isn't loaded.
fn cursor_move(app: &mut App, f: impl FnOnce(Cursor, &[String]) -> Cursor) {
    let Some((_, lines)) = app.sources.get(&app.items[app.sel].path) else {
        return;
    };
    if lines.is_empty() {
        return;
    }
    app.cursor = clamp_cursor(f(app.cursor, lines), lines);
    app.scroll = follow_scroll(app.cursor.line, app.scroll, app.code_height);
    app.hscroll = follow_hscroll(app.cursor.col, app.hscroll, app.code_width);
}

fn last_line(len: usize) -> u16 {
    len.saturating_sub(1).min(u16::MAX as usize) as u16
}

/// One-line motion: moves the selection in the list, the cursor in the code
/// pane (scroll follows), scrolls the why pane.
fn step(app: &mut App, by: isize) {
    match app.focus {
        Pane::List => {
            // fold-aware: a hunk inside a folded group is not on screen, so
            // j/k step over it rather than selecting something invisible
            let view = folded_view(app);
            let pos = view_pos(&view, app.sel);
            let to = (pos as isize + by).clamp(0, view.len() as isize - 1) as usize;
            select(app, view[to]);
        }
        Pane::Code | Pane::Deps => cursor_move(app, |c, lines| move_line(c, lines, by)),
        Pane::Why => why_cursor_move(app, by),
    }
}

/// The list-pane position of `sel` within `view` — 0 when it isn't found
/// (shouldn't happen: `set_filters`/`select` keep `sel` in `view`, but a
/// missing position degrading to the top is safer than panicking).
fn view_pos(view: &[usize], sel: usize) -> usize {
    view.iter().position(|&i| i == sel).unwrap_or(0)
}

/// The first/last currently visible item — `view` is never empty once
/// loading finishes (`set_filters` rejects any change that would empty it).
fn first_visible(app: &App) -> usize {
    folded_view(app)[0]
}
fn last_visible(app: &App) -> usize {
    *folded_view(app).last().expect("view is never empty")
}

/// Move the why pane's line cursor by `by`, clamp to its content, and scroll
/// to follow — the why-pane counterpart of `cursor_move`.
fn why_cursor_move(app: &mut App, by: isize) {
    app.why_sel = scrolled(app.why_sel as u16, by, app.why_len) as usize;
    app.why_scroll = follow_scroll(app.why_sel, app.why_scroll, app.why_height);
}

/// Paging drives the code pane even when the list has focus — list rows are one
/// line each, so paging them is useless while the code is what needs scrolling.
fn page(app: &mut App, by: isize) {
    match app.focus {
        Pane::Why | Pane::Deps => app.why_scroll = scrolled(app.why_scroll, by, app.why_len),
        _ => app.scroll = scrolled(app.scroll, by, app.code_len),
    }
}

fn scrolled(cur: u16, by: isize, len: usize) -> u16 {
    (cur as isize + by).clamp(0, last_line(len) as isize) as u16
}

fn select(app: &mut App, to: usize) {
    app.sel = to;
    app.scroll = auto_scroll(&app.items[to]);
    app.hscroll = 0;
    app.why_scroll = 0;
    app.why_sel = 0;
    app.cursor = cursor_for(&app.items[to], &app.sources);
    app.popup = None; // a new hunk invalidates whatever the popup was showing
                      // match positions are per-file line/col — a different hunk (possibly a
                      // different file entirely) invalidates them, so re-anchor by dropping the
                      // search rather than trying to remap it
    app.search = None;
}

// The TUI can't be driven headless (`ratatui::init()` needs a real tty), so
// what's testable here is factored into small pure functions above — cursor
// arithmetic, scroll-follow clamping, and symbol/signature/docstring
// extraction — and exercised directly.
#[cfg(test)]
mod tests;

/// Renders the parts of `README.md` and `docs/` that the code owns, and checks
/// the committed text still matches. A block is delimited by an
/// `ordo:begin <key>` / `ordo:end <key>` HTML comment pair; `UPDATE_DOCS=1
/// cargo test` rewrites them in place, the same contract `UPDATE_GOLDEN=1` has
/// for the golden fixtures.
///
/// The point is not to save typing. It is that the language list, the command
/// bar, the keymaps, the theme list, the bundled rulesets, the `[[rule]]` keys
/// and the container kinds all had a hand-written copy in the README that had
/// already drifted from the tables the program actually reads.
#[cfg(test)]
mod docs;
