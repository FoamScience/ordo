//! ordo — a terminal reviewer that is a pure client of the ordo engine.
//! It owns git (shells out for a commit's blobs), calls `ordo::run`, and renders
//! the change in comprehension order: the full file with the changed hunk
//! highlighted in context, plus rationale, advisories and def→use edges. The
//! engine stays git-free; gated behind the `tui` feature so the default build
//! never pulls a UI stack.
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ordo::model::{Change, HunkOut, Input, Options, Output, Strategy, Symbol};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use tree_sitter::{Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const PAGE: u16 = 15;

const USAGE: &str = "\
ordo — interactive review of a commit, ordered for comprehension.

usage:
  ordo [<rev>] [<glob>...] [--keys <preset>] [--theme <name>] [--rules <file>]... [--all] [--only-comments]
  ordo --init-config [--force]
  ordo --help
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
dracula, solarized (dark, light) — name every colour themselves. `:theme` lists
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
same negative-glob syntax as the CLI), `:keys <preset>`, `:strategy
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

// Which changed files reach the engine. Generated and lock files are dropped
// before their blobs are even read — parsing a lock file costs more than the
// review it would add. Globs, when given, keep a path that matches at least
// one positive pattern (or there are none) and no negative one.
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
}

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
type ParsedArgs = (String, Keymap, Filter, bool, Theme, Vec<String>);

fn parse_args() -> Result<ParsedArgs, i32> {
    let mut rev: Option<String> = None;
    let mut globs: Vec<String> = vec![];
    let mut skip_generated = true;
    let mut only_comments = false;
    // whether the preset was *chosen* (flag or env) — a config file's own
    // `preset =` only applies when it wasn't
    let mut preset_given = std::env::var("ORDO_TUI_KEYS").is_ok();
    let mut preset = std::env::var("ORDO_TUI_KEYS").unwrap_or_else(|_| "vim".to_string());
    let mut theme_given = std::env::var("ORDO_TUI_THEME").is_ok();
    let mut theme_name = std::env::var("ORDO_TUI_THEME").unwrap_or_else(|_| "dark".to_string());
    let mut want_preset = false;
    let mut want_theme = false;
    let mut want_rules = false;
    let mut extra_rules: Vec<String> = vec![];
    let mut want_init = false;
    let mut force = false;
    for a in std::env::args().skip(1) {
        if want_preset {
            preset = a;
            preset_given = true;
            want_preset = false;
            continue;
        }
        if want_theme {
            theme_name = a;
            theme_given = true;
            want_theme = false;
            continue;
        }
        if want_rules {
            extra_rules.push(a);
            want_rules = false;
            continue;
        }
        match a.as_str() {
            "--keys" => want_preset = true,
            s if s.starts_with("--keys=") => {
                preset = s["--keys=".len()..].to_string();
                preset_given = true;
            }
            "--theme" => want_theme = true,
            s if s.starts_with("--theme=") => {
                theme_name = s["--theme=".len()..].to_string();
                theme_given = true;
            }
            "--rules" => want_rules = true,
            s if s.starts_with("--rules=") => extra_rules.push(s["--rules=".len()..].to_string()),
            "--all" => skip_generated = false,
            "--only-comments" => only_comments = true,
            "--init-config" => want_init = true,
            "--force" => force = true,
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                return Err(0);
            }
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
            s if rev.is_some() => globs.push(s.to_string()),
            s => rev = Some(s.to_string()),
        }
    }
    if want_preset {
        eprintln!("ordo: --keys needs a preset name\n\n{USAGE}");
        return Err(2);
    }
    if want_theme {
        eprintln!("ordo: --theme needs a value\n\n{USAGE}");
        return Err(2);
    }
    // the config's preset is a default; an explicit --keys still wins
    let cfg = config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_key_config(&t));
    let preset = match (&cfg, preset_given) {
        (Some(c), false) => c.preset.clone().unwrap_or(preset),
        _ => preset,
    };
    let Some(keys) = keymap(&preset) else {
        eprintln!("ordo: unknown key preset '{preset}' (want: vim, vscode)");
        return Err(2);
    };
    let keys = match &cfg {
        Some(c) => {
            for p in &c.problems {
                eprintln!("ordo: tui.toml: {p}");
            }
            apply_key_config(keys, c)
        }
        None => keys,
    };
    // same precedence as the keymap: an explicit --theme beats the config's
    let theme_name = match (&cfg, theme_given) {
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
    let theme = match &cfg {
        Some(c) => apply_theme_colors(theme, &c.colors),
        None => theme,
    };
    if want_init {
        // `--init-config` is a whole run of its own: nothing is reviewed, and
        // the exit code is the write's own (`Err(0)` on success, as `--help`
        // and `--version` already report a clean stop)
        write_init_config(&preset, &theme_name, force)?;
        return Err(0);
    }
    let globs = build_globs(&globs).map_err(|e| {
        eprintln!("ordo: {e}");
        2
    })?;
    let filter = Filter {
        globs,
        skip_generated,
        negatives_emptied: std::cell::Cell::new(false),
        tally: std::cell::Cell::new(Ledger::default()),
    };
    Ok((
        rev.unwrap_or_else(|| "HEAD".to_string()),
        keys,
        filter,
        only_comments,
        theme,
        extra_rules,
    ))
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
    let (rev, keys, filter, only_comments, theme, extra_rules) = match parse_args() {
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
    let rules = report.rules;
    run(
        rev,
        keys,
        target,
        filter,
        only_comments,
        review_sha,
        uncommitted,
        theme,
        rules,
        rules_report,
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
fn load(
    target: Target,
    filter: Filter,
    only_comments: bool,
    rev: String,
    // highlighting happens here, off the draw loop, so the worker needs the
    // theme's syntax colours rather than re-highlighting on every redraw
    syn: Syntax,
    // the reviewer's own rules (user + repo), collected by `main`
    rules: Vec<ordo::model::Rule>,
    tx: mpsc::Sender<LoadMsg>,
) {
    let progress = |msg: String| {
        let _ = tx.send(LoadMsg::Progress(msg));
    };
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
    let sources: Sources = input
        .changes
        .iter()
        .map(|c| {
            let split = |s: Option<&String>| {
                s.map(|t| t.lines().map(String::from).collect())
                    .unwrap_or_default()
            };
            (
                c.path.clone(),
                (split(c.old.as_ref()), split(c.new.as_ref())),
            )
        })
        .collect();
    // syntax highlight each new file once, up front (indexed by path)
    let candidates: Vec<&Change> = input.changes.iter().filter(|c| c.new.is_some()).collect();
    let total = candidates.len();
    let mut highlights: Highlights = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        progress(highlight_progress(i, total));
        if let Some(h) = highlight_file(&c.path, c.new.as_ref().unwrap(), &syn) {
            highlights.insert(c.path.clone(), h);
        }
    }
    let (hl_ms, hl_files) = (t.elapsed().as_millis(), highlights.len());
    let t = std::time::Instant::now();
    progress("ordering…".to_string());
    let mut input = input;
    input.options.rules = rules;
    let out = ordo::run(input);
    // the engine records what it dropped and why; fold it into the same ledger
    // the path filter has been filling in, so `:audit` reads one set of numbers
    let mut ledger = filter.tally.get();
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
    let mut items = build_items(&out);
    refine_items(&mut items, &sources);
    let groups = group_reasons(&out);
    let view = compute_view(&items, only_comments, true, None);
    if view.is_empty() {
        let msg = if only_comments {
            format!(
                "ordo: nothing to review in {rev} — no comment changes{}",
                filter.note()
            )
        } else {
            format!("ordo: nothing to review in {rev}{}", filter.note())
        };
        let _ = tx.send(LoadMsg::Empty(msg));
        return;
    }
    let timing = format!(
        "ordo: read {files} file{} in {read_ms}ms · highlighted {hl_files} in {hl_ms}ms · \
         ordered {} hunks into {} groups, {} cluster{} in {}ms",
        plural(files),
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
    // carry a note across a rename: the ledger knows the name the symbol had,
    // so the note written against the old identity finds its way to the new one
    let mut migrated = false;
    for e in out.ledger.iter().filter(|e| e.from.is_some()) {
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
    if migrated {
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

// ------------------------------------------------------------------- git layer

// Empty string when git fails, so a missing blob reads as empty content. The
// status check matters: `rev-parse --verify -q` still prints on failure (a range
// echoes both endpoints), and taking that output would be read as a sha.
fn git(args: &[&str]) -> String {
    run_cmd("git", args)
}

// Runs a binary, returning its stdout as a string or empty on any failure to
// launch or a nonzero exit — the shared body behind `git` and `but`.
fn run_cmd(bin: &str, args: &[&str]) -> String {
    Command::new(bin)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// Same, with the arg list fed on stdin — for `check-attr --stdin`, where the
// path list can outgrow what a command line takes.
fn git_stdin(args: &[&str], input: &str) -> String {
    use std::io::Write;
    let mut child = match Command::new("git")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    if let Some(mut w) = child.stdin.take() {
        let _ = w.write_all(input.as_bytes());
    }
    child
        .wait_with_output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// Fetches many `<rev>:<path>` blobs in one `git cat-file --batch` process
// instead of one `git show` per spec — the same fork/exec is paid once for the
// whole file list instead of once per file per side. Missing objects (a path
// added or deleted on one side) come back absent from the map, same as `git`
// returning empty on failure; look them up with `.unwrap_or_default()` to
// match. Lossy UTF-8, same as `run_cmd`, so binary-ish content behaves the
// same as the old per-file `git show` path. Stdin is written and dropped
// (closing it) before stdout is read, to avoid deadlocking on a full pipe
// buffer with a large spec list.
fn git_cat_file_batch(specs: &[String]) -> HashMap<String, String> {
    use std::io::{Read, Write};
    let mut result = HashMap::with_capacity(specs.len());
    if specs.is_empty() {
        return result;
    }
    let mut child = match Command::new("git")
        .args(["cat-file", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return result,
    };
    {
        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => return result,
        };
        for spec in specs {
            if writeln!(stdin, "{spec}").is_err() {
                break;
            }
        }
        // `stdin` drops here, closing the pipe so the child's stdout can flush
        // fully instead of the two of us deadlocking on a full pipe buffer.
    }
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => return result,
    };
    let mut buf = Vec::new();
    if stdout.read_to_end(&mut buf).is_err() {
        let _ = child.wait();
        return result;
    }
    let _ = child.wait();

    let mut i = 0;
    for spec in specs {
        let Some(nl) = buf[i..].iter().position(|&b| b == b'\n').map(|p| i + p) else {
            break;
        };
        let header = String::from_utf8_lossy(&buf[i..nl]).into_owned();
        i = nl + 1;
        if header.ends_with("missing") {
            continue;
        }
        // header: "<oid> <type> <size>" — take size by splitting from the
        // right so an oid or type never gets mistaken for it.
        let Some(size) = header
            .rsplit(' ')
            .next()
            .and_then(|s| s.parse::<usize>().ok())
        else {
            break;
        };
        let Some(content) = buf.get(i..i + size) else {
            break;
        };
        result.insert(spec.clone(), String::from_utf8_lossy(content).into_owned());
        i += size + 1; // skip the trailing newline after the content block
    }
    result
}

// GitButler CLI: empty string if `but` isn't installed or the call fails, so the
// plain-git path is unaffected on non-GitButler repos.
fn but(args: &[&str]) -> String {
    run_cmd("but", args)
}

// Paths the repo's own `.gitattributes` marks as generated: `linguist-generated`
// (the marker GitHub collapses a file by) or an explicit `-diff`. One batched
// call — `-z` makes both the path list and the output NUL-separated, so paths
// with colons or newlines survive the round trip.
fn declared_generated(paths: &[String]) -> std::collections::HashSet<String> {
    let mut input = paths.join("\0");
    input.push('\0');
    let out = git_stdin(
        &["check-attr", "-z", "--stdin", "linguist-generated", "diff"],
        &input,
    );
    let mut fields = out.split('\0');
    let mut generated = std::collections::HashSet::new();
    while let (Some(path), Some(attr), Some(value)) = (fields.next(), fields.next(), fields.next())
    {
        if path.is_empty() {
            break;
        }
        let declared = match attr {
            // a bare `linguist-generated` reads as "set", `=true` as "true"
            "linguist-generated" => value == "set" || value == "true",
            "diff" => value == "unset",
            _ => false,
        };
        if declared {
            generated.insert(path.to_string());
        }
    }
    generated
}

// -------------------------------------------------------------- gitbutler layer

/// `but --json status` — the whole workspace in one call: the uncommitted
/// changes, each stack's branches and their commits, and the merge base. None
/// when `but` is missing or the repo isn't GitButler-managed.
fn workspace() -> Option<serde_json::Value> {
    serde_json::from_str(&but(&["--json", "status"])).ok()
}

fn field<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
    v.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

fn branches(ws: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    ws.get("stacks")
        .and_then(|s| s.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.get("branches")?.as_array())
        .flatten()
}

/// A workspace branch — by the CLI ID `but status` prints, or by its name —
/// reviews as the range spanning its own commits, which is what reviewing one
/// branch of a stack means. Takes precedence over git's reading of the same
/// name, where a branch is only its tip commit.
fn branch_target(ws: &serde_json::Value, arg: &str) -> Option<Target> {
    let b = branches(ws).find(|b| field(b, "cliId") == arg || field(b, "name") == arg)?;
    let commits = b.get("commits")?.as_array()?;
    fn commit_id(c: &serde_json::Value) -> Option<&str> {
        Some(field(c, "commitId")).filter(|s| !s.is_empty())
    }
    // `but` lists a branch newest-first, so the range runs from below the last
    // commit up to the first
    let tip = commit_id(commits.first()?)?.to_string();
    let oldest = commit_id(commits.last()?)?;
    let base = git(&["rev-parse", "--verify", "-q", &format!("{oldest}^")]);
    let base = base.trim();
    let base = if base.is_empty() { EMPTY_TREE } else { base };
    Some(Target::Range(base.to_string(), tip))
}

/// A workspace commit by its CLI ID, or by a prefix of its change ID or commit
/// ID — the same identifiers `but status` accepts.
fn commit_target(ws: &serde_json::Value, arg: &str) -> Option<Target> {
    let c = branches(ws)
        .filter_map(|b| b.get("commits")?.as_array())
        .flatten()
        .find(|c| {
            field(c, "cliId") == arg
                || field(c, "changeId").starts_with(arg)
                || field(c, "commitId").starts_with(arg)
        })?;
    Some(Target::Commit(field(c, "commitId").to_string()))
}

enum Target {
    Commit(String),
    /// A `base..tip` / `base...tip` range, already resolved to two commit shas.
    Range(String, String),
    Uncommitted,
    /// `base..zz` / `base...zz` — a resolved base commit versus the worktree
    /// (not HEAD). `...` has already been reduced to the merge base of `base`
    /// and HEAD, same as `Range`'s symmetric case.
    WorktreeRange(String),
}

// `base..tip` is a plain two-endpoint diff; `base...tip` diffs from the merge
// base, which is what you want for a branch that trails its target. An omitted
// side means HEAD, as in git. `tip == "zz"` diffs against the worktree instead
// of a commit — see `WorktreeRange`; `zz` on the left is meaningless (nothing
// precedes the uncommitted area) and rejected.
fn resolve_range(arg: &str) -> Option<Target> {
    let (base, tip, symmetric) = match arg.split_once("...") {
        Some((b, t)) => (b, t, true),
        None => {
            let (b, t) = arg.split_once("..")?;
            (b, t, false)
        }
    };
    // a second `..` is not a range we understand (e.g. `a..b..c`)
    if tip.contains("..") {
        return None;
    }
    if base == "zz" {
        return None;
    }
    let rev = |s: &str| {
        let s = if s.is_empty() { "HEAD" } else { s };
        let sha = git(&["rev-parse", "--verify", "-q", &format!("{s}^{{commit}}")]);
        let sha = sha.trim().to_string();
        (!sha.is_empty()).then_some(sha)
    };
    if tip == "zz" {
        let base = rev(base)?;
        if !symmetric {
            return Some(Target::WorktreeRange(base));
        }
        let merge_base = git(&["merge-base", &base, "HEAD"]);
        let merge_base = merge_base.trim();
        return (!merge_base.is_empty()).then(|| Target::WorktreeRange(merge_base.to_string()));
    }
    let (base, tip) = (rev(base)?, rev(tip)?);
    if !symmetric {
        return Some(Target::Range(base, tip));
    }
    let merge_base = git(&["merge-base", &base, &tip]);
    let merge_base = merge_base.trim();
    (!merge_base.is_empty()).then(|| Target::Range(merge_base.to_string(), tip))
}

// Resolve an arg to a commit, a range, or the uncommitted area. `zz` is
// GitButler's constant for the uncommitted area and never a commit. Ranges come
// next, then a GitButler workspace *branch* — ahead of git so a branch label
// reviews the branch's commits rather than just its tip. Plain git commit-ishes
// (sha, HEAD~2, tag) follow, and finally a GitButler commit CLI ID. The
// GitButler steps are skipped entirely on a repo `but` doesn't manage.
fn resolve(arg: &str) -> Option<Target> {
    if arg == "zz" {
        return Some(Target::Uncommitted);
    }
    // `..` is illegal in a refname, so it can only mean a range here
    if arg.contains("..") {
        return resolve_range(arg);
    }
    let ws = workspace();
    if let Some(t) = ws.as_ref().and_then(|ws| branch_target(ws, arg)) {
        return Some(t);
    }
    let sha = git(&["rev-parse", "--verify", "-q", &format!("{arg}^{{commit}}")]);
    let sha = sha.trim();
    if !sha.is_empty() {
        return Some(Target::Commit(sha.to_string()));
    }
    ws.as_ref().and_then(|ws| commit_target(ws, arg))
}

// The single commit the hover popup's history section is relative to: a
// range reviews as its tip, and `zz` (the uncommitted area) has no commit of
// its own, so HEAD stands in for it. None only if HEAD itself can't resolve
// (an empty repo) — history is then just unavailable.
fn review_commit_sha(target: &Target) -> Option<String> {
    match target {
        Target::Commit(sha) => Some(sha.clone()),
        Target::Range(_, tip) => Some(tip.clone()),
        Target::Uncommitted | Target::WorktreeRange(_) => {
            let sha = git(&["rev-parse", "--verify", "-q", "HEAD"]);
            let sha = sha.trim();
            (!sha.is_empty()).then(|| sha.to_string())
        }
    }
}

fn gather(rev: &str, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let parent = git(&["rev-parse", "--verify", "-q", &format!("{rev}^")]);
    let parent = parent.trim();
    let parent = if parent.is_empty() {
        EMPTY_TREE
    } else {
        parent
    };
    gather_range(parent, rev, filter, progress)
}

// Both sides' full blobs (empty when a file is added or deleted), so hunks get
// full semantics instead of degrading to a context-limited patch.
fn gather_range(base: &str, tip: &str, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let names = git(&["diff", "--name-only", base, tip]);
    let paths: Vec<String> = names
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let paths = filter.apply(paths);
    let total = paths.len();
    let specs: Vec<String> = paths
        .iter()
        .flat_map(|p| [format!("{base}:{p}"), format!("{tip}:{p}")])
        .collect();
    let mut blobs = git_cat_file_batch(&specs);
    let changes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            progress(read_progress(&path, i, total));
            let old = blobs.remove(&format!("{base}:{path}")).unwrap_or_default();
            let new = blobs.remove(&format!("{tip}:{path}")).unwrap_or_default();
            Change {
                old: Some(old),
                new: Some(new),
                diff: None,
                path,
            }
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// The uncommitted area (`zz`): GitButler owns what counts as uncommitted (it
// includes untracked adds git-diff wouldn't show), so take the file list from
// the workspace status — which lists it per file, unlike `but diff`'s per-hunk
// entries. Changes assigned to a stack are picked up the same way. Full old/new
// content — old from HEAD, new from the worktree — so hunks get full semantics
// rather than degrading to a context-limited patch.
// Every path GitButler's workspace status lists as changed: `stacks[].assignedChanges`
// plus `uncommittedChanges`, each entry's `filePath`. Shared by `gather_uncommitted`
// and `gather_worktree_range`'s untracked-file branch — neither sorts nor dedups
// here, that's each caller's own business.
fn workspace_change_paths(ws: &serde_json::Value) -> Vec<String> {
    let assigned = ws
        .get("stacks")
        .and_then(|s| s.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.get("assignedChanges")?.as_array())
        .flatten();
    ws.get("uncommittedChanges")
        .and_then(|c| c.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .chain(assigned)
        .map(|c| field(c, "filePath").to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn gather_uncommitted(filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let ws = workspace().unwrap_or(serde_json::Value::Null);
    let mut paths = workspace_change_paths(&ws);
    paths.dedup();
    let paths = filter.apply(paths);
    let total = paths.len();
    let specs: Vec<String> = paths.iter().map(|p| format!("HEAD:{p}")).collect();
    let mut blobs = git_cat_file_batch(&specs);
    let changes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            progress(read_progress(&path, i, total));
            let old = blobs.remove(&format!("HEAD:{path}")).unwrap_or_default();
            Change {
                old: Some(old),
                new: Some(std::fs::read_to_string(&path).unwrap_or_default()),
                diff: None,
                path,
            }
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// `base..zz` / `base...zz`: everything done on a branch including what's not
// yet committed — `base`'s tree versus the worktree. `git diff --name-only
// <base>` (one revision, no second) already compares a tree to the working
// directory, but it never lists untracked files, so the changed-file set is
// that plus untracked paths — from `but --json status`, same source
// `gather_uncommitted` uses, on a GitButler-managed repo, else `git ls-files
// --others --exclude-standard`. `old` comes from `base`'s tree (empty when
// the path didn't exist there); `new` is read straight off disk. A path
// absent from the worktree (deleted since `base`) gets an empty `new`, not a
// read error; a path that exists but can't be read as UTF-8 is dropped
// rather than folded into an empty string, which would misreport it as a
// deletion.
fn gather_worktree_range(base: &str, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let mut paths: Vec<String> = git(&["diff", "--name-only", base])
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let untracked: Vec<String> = match workspace() {
        Some(ws) => workspace_change_paths(&ws),
        None => git(&["ls-files", "--others", "--exclude-standard"])
            .split('\n')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
    };
    paths.extend(untracked);
    paths.sort();
    paths.dedup();
    let paths = filter.apply(paths);
    let total = paths.len();
    let specs: Vec<String> = paths.iter().map(|p| format!("{base}:{p}")).collect();
    let mut blobs = git_cat_file_batch(&specs);
    let changes = paths
        .into_iter()
        .enumerate()
        .filter_map(|(i, path)| {
            progress(read_progress(&path, i, total));
            let new = match std::fs::read(&path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(_) => {
                    filter.note_unreadable();
                    return None;
                }
            };
            let old = blobs.remove(&format!("{base}:{path}")).unwrap_or_default();
            Some(Change {
                old: Some(old),
                new: Some(new),
                diff: None,
                path,
            })
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
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
    cat: String,
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
    advisories: Vec<(String, String, bool)>,
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
    /// reviewing rules that matched this hunk (`ordo::model::Rule`)
    rules: Vec<ordo::model::RuleHit>,
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

// ---------------------------------------------------------- reviewed-mark persistence

/// Hand-rolled FNV-1a 64-bit. Deliberately not `DefaultHasher` — its output is
/// explicitly unspecified across Rust releases, so a toolchain upgrade would
/// silently invalidate every mark ever written. This is ~10 lines and never
/// changes.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// The symbol-identity component of a mark key: every symbol this hunk
/// defines (name + tree-sitter kind + scope — the fields that answer "is
/// this the same symbol"), sorted first so the key never depends on the
/// engine's emission order. Falls back to `enclosing` for a hunk that defines
/// no symbol of its own (a body edit, or a top-level/comment-only hunk), and
/// to the empty string when that's absent too.
///
/// KNOWN LIMITATION: two identical hunks under the same symbol in the same
/// file hash to the same key, so marking one marks both. A positional
/// tiebreak would reintroduce exactly the fragility this key is designed to
/// avoid — a hunk's position shifts whenever anything above it changes, so a
/// position-based key would drop marks on edits that never touched the hunk
/// itself. This case is rare, and its failure is visible (two rows tick
/// together at once) rather than silent.
fn symbol_identity_key(item: &Item) -> String {
    if !item.symbols.is_empty() {
        symbols_identity(&item.symbols)
    } else {
        item.enclosing.clone().unwrap_or_default()
    }
}

/// name + kind + scope for each symbol, order-independent — the identity both
/// `mark_key` and `note_key` are built on.
fn symbols_identity(symbols: &[ordo::model::Symbol]) -> String {
    let mut syms = symbols.to_vec();
    syms.sort();
    syms.iter()
        .map(|s| {
            format!(
                "{}\u{1f}{}\u{1f}{}",
                s.name,
                s.kind,
                s.scope.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\u{1e}")
}

/// Hashes both sides of the hunk's content — old lines, then new — so any
/// change at all (reformat, body edit, signature change) drops the mark. A
/// false "already reviewed" is far worse than a lost one, which is the whole
/// point of covering both sides rather than just the new one. `None` when the
/// item's file content isn't loaded (nothing to hash).
fn hunk_content_hash(item: &Item, sources: &Sources) -> Option<u64> {
    let (ol, nl) = sources.get(&item.path)?;
    let [o0, o1] = item.old_range;
    let [n0, n1] = item.new_range;
    let mut buf = String::new();
    if o0 >= 1 && o0 <= o1 && o1 <= ol.len() {
        for l in &ol[o0 - 1..o1] {
            buf.push_str(l);
            buf.push('\n');
        }
    }
    buf.push('\u{0}');
    if n0 >= 1 && n0 <= n1 && n1 <= nl.len() {
        for l in &nl[n0 - 1..n1] {
            buf.push_str(l);
            buf.push('\n');
        }
    }
    Some(fnv1a(buf.as_bytes()))
}

/// The full persistence key: `(rev, path, symbol identity, hunk content
/// hash)`, folded into one FNV-1a hash so the file on disk records only an
/// opaque number — never a path, a symbol name, or source text. `None` when
/// there's nothing to hash the content from.
fn mark_key(rev: &str, item: &Item, sources: &Sources) -> Option<u64> {
    let content_hash = hunk_content_hash(item, sources)?;
    let sym_key = symbol_identity_key(item);
    let combined = format!("{rev}\u{0}{}\u{0}{sym_key}\u{0}{content_hash:x}", item.path);
    Some(fnv1a(combined.as_bytes()))
}

/// A review note's key: the symbol's identity and **nothing else**.
///
/// The exact opposite of `mark_key`, and deliberately so. A mark folds in the
/// revision, the path and a hash of both sides of the hunk, because a stale
/// "already reviewed" is worse than a lost one. A note is a thought about a
/// *symbol* — it must outlive a rebase (no revision), a move to another file
/// (no path) and an edit to the body (no content hash). Renames are handled by
/// migrating the old identity's key when the ledger reports one, so the note
/// follows the symbol through its new name too.
///
/// `None` when the hunk declares no symbol: there is nothing stable to anchor
/// to, and anchoring to the enclosing name would silently drift.
fn note_key(item: &Item) -> Option<u64> {
    if item.symbols.is_empty() {
        return None;
    }
    Some(fnv1a(symbols_identity(&item.symbols).as_bytes()))
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
fn draft_rule(app: &App, i: usize) -> Vec<String> {
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
fn cascade(app: &App, i: usize) -> Vec<usize> {
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
fn cascade_line(app: &App, i: usize) -> Option<String> {
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
fn delta_line(app: &App, i: usize) -> Option<&'static str> {
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
fn out_of_order_labels(app: &App, i: usize) -> Vec<String> {
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
fn coverage(app: &App) -> (usize, usize, usize, usize) {
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

/// The note anchored to item `i`'s symbol, if any.
fn note_for(app: &App, i: usize) -> Option<&str> {
    let key = note_key(&app.items[i])?;
    app.notes.get(&key).map(String::as_str)
}

/// The key a symbol *would* have had under its previous name, so a note
/// written before a rename can be carried across to it.
fn note_key_named(item: &Item, old_name: &str) -> Option<u64> {
    if item.symbols.is_empty() {
        return None;
    }
    let mut renamed = item.symbols.clone();
    for sym in &mut renamed {
        sym.name = old_name.to_string();
    }
    Some(fnv1a(symbols_identity(&renamed).as_bytes()))
}

const MARK_TTL_SECS: u64 = 90 * 24 * 60 * 60;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_home() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        if !x.trim().is_empty() {
            return Some(PathBuf::from(x));
        }
    }
    let home = std::env::var("HOME").ok()?;
    (!home.trim().is_empty()).then(|| PathBuf::from(home).join(".cache"))
}

/// `${XDG_CACHE_HOME:-$HOME/.cache}/ordo/reviewed/<repo>.json` — cache, not
/// `.git/`: derived state, safe to lose, and nothing a user could accidentally
/// commit. `<repo>` is the repo's toplevel path hashed rather than written
/// verbatim, so the filename itself gives nothing away either. `None` when
/// neither env var resolves — callers then just skip persistence.
fn marks_file_path(repo_root: &str) -> Option<PathBuf> {
    let dir = cache_home()?.join("ordo").join("reviewed");
    Some(dir.join(format!("{:016x}.json", fnv1a(repo_root.as_bytes()))))
}

/// What one hunk looked like the last time this review was opened: what it
/// contained, where it sat in the reading order, and what it depended on.
#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct Snap {
    /// hex hash of both sides of the hunk
    c: String,
    /// its position in the reading order
    p: usize,
    /// keys of the hunks defining what it uses
    d: Vec<String>,
}

/// How a hunk differs from the last time this review was opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Delta {
    /// not in the previous run at all
    New,
    /// same symbol, different content
    Changed,
    /// **byte-identical, but it reads in a different place now** — because
    /// what it depends on changed. No other tool reports this: a diff sees
    /// nothing, so the hunk looks untouched while the reason to read it moved.
    Moved,
    /// same content, same position, same dependencies
    Same,
}

/// A hunk's identity across runs: its symbol, and the file it lives in. Not
/// the content and not the revision — those are what the delta is measuring.
fn snap_key(item: &Item) -> String {
    format!(
        "{:016x}",
        fnv1a(format!("{}\u{0}{}", item.path, symbol_identity_key(item)).as_bytes())
    )
}

/// `${XDG_CACHE_HOME:-$HOME/.cache}/ordo/runs/<repo>.json` — the previous run,
/// so the next one can say what moved. Overwritten each time the review is
/// opened, so a delta always answers "since I last looked".
fn runs_file_path(repo_root: &str) -> Option<PathBuf> {
    let dir = cache_home()?.join("ordo").join("runs");
    Some(dir.join(format!("{:016x}.json", fnv1a(repo_root.as_bytes()))))
}

fn load_snaps(path: &Path) -> HashMap<String, Snap> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_snaps(path: &Path, snaps: &HashMap<String, Snap>) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string(snaps) {
        let _ = std::fs::write(path, text);
    }
}

/// This run's snapshot, and how each item differs from the stored one.
fn compare_runs(
    items: &[Item],
    order: &[usize],
    sources: &Sources,
    prev: &HashMap<String, Snap>,
) -> (HashMap<String, Snap>, Vec<Delta>) {
    let key_of: Vec<String> = items.iter().map(snap_key).collect();
    let pos_of: HashMap<usize, usize> = order.iter().enumerate().map(|(p, &i)| (i, p)).collect();
    let mut now: HashMap<String, Snap> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        let deps: Vec<String> = it
            .edges
            .iter()
            .filter(|e| e.dependency)
            .filter_map(|e| e.target)
            .map(|t| key_of[t].clone())
            .collect();
        now.insert(
            key_of[i].clone(),
            Snap {
                c: format!("{:016x}", hunk_content_hash(it, sources).unwrap_or(0)),
                p: pos_of.get(&i).copied().unwrap_or(usize::MAX),
                d: deps,
            },
        );
    }
    let deltas = items
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let k = &key_of[i];
            let (Some(was), Some(is)) = (prev.get(k), now.get(k)) else {
                return Delta::New;
            };
            if was.c != is.c {
                Delta::Changed
            } else if was.p != is.p || was.d != is.d {
                Delta::Moved
            } else {
                Delta::Same
            }
        })
        .collect();
    (now, deltas)
}

/// `${XDG_CACHE_HOME:-$HOME/.cache}/ordo/notes/<repo>.json` — beside the marks,
/// same reasoning about the cache and the hashed repo name. Kept in its own
/// file because a note outlives the mark on the same hunk: marks expire with
/// the content, notes follow the symbol.
fn notes_file_path(repo_root: &str) -> Option<PathBuf> {
    let dir = cache_home()?.join("ordo").join("notes");
    Some(dir.join(format!("{:016x}.json", fnv1a(repo_root.as_bytes()))))
}

/// Reads the note file: hex key -> note text. Degrades to "no notes" on any
/// failure, exactly as the marks do — the cache is a convenience.
fn load_notes(path: &Path) -> HashMap<u64, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(raw) = serde_json::from_str::<HashMap<String, String>>(&text) else {
        return HashMap::new();
    };
    raw.into_iter()
        .filter_map(|(k, v)| u64::from_str_radix(&k, 16).ok().map(|k| (k, v)))
        .collect()
}

fn save_notes(path: &Path, notes: &HashMap<u64, String>) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let body: HashMap<String, String> = notes
        .iter()
        .map(|(k, v)| (format!("{k:016x}"), v.clone()))
        .collect();
    if let Ok(text) = serde_json::to_string(&body) {
        let _ = std::fs::write(path, text);
    }
}

/// Reads the mark file: hex key -> unix-seconds written. Any failure at all —
/// missing file, unreadable, corrupt JSON — degrades to "no marks" rather
/// than a crash or a blocked UI; the cache is a convenience, the review is
/// the product.
fn load_marks(path: &Path) -> HashMap<u64, u64> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(raw) = serde_json::from_str::<HashMap<String, u64>>(&text) else {
        return HashMap::new();
    };
    raw.into_iter()
        .filter_map(|(k, v)| u64::from_str_radix(&k, 16).ok().map(|k| (k, v)))
        .collect()
}

/// Drops anything older than `MARK_TTL_SECS` so the file doesn't grow
/// unbounded across every repo/review a user ever runs.
fn prune_marks(marks: &mut HashMap<u64, u64>, now: u64) {
    marks.retain(|_, ts| now.saturating_sub(*ts) <= MARK_TTL_SECS);
}

/// Best-effort write: creates the parent directory if needed, and silently
/// gives up on any failure (read-only filesystem, permission, missing
/// `$HOME`) rather than surfacing it — a lost mark is the accepted cost, a
/// crash or a blocked review is not. The files are tiny, so this runs on
/// every toggle rather than only at quit, and a panic or killed terminal
/// never loses the session's marks.
fn save_marks(path: &Path, marks: &HashMap<u64, u64>) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let body: HashMap<String, u64> = marks
        .iter()
        .map(|(k, v)| (format!("{k:016x}"), *v))
        .collect();
    if let Ok(text) = serde_json::to_string(&body) {
        let _ = std::fs::write(path, text);
    }
}

// ----------------------------------------------------------------------- keys

/// The three panes, in focus-cycle order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pane {
    List,
    Code,
    Why,
}

impl Pane {
    fn next(self) -> Pane {
        match self {
            Pane::List => Pane::Code,
            Pane::Code => Pane::Why,
            Pane::Why => Pane::List,
        }
    }
    fn prev(self) -> Pane {
        match self {
            Pane::List => Pane::Why,
            Pane::Code => Pane::List,
            Pane::Why => Pane::Code,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
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
enum ViewMode {
    #[default]
    Ledger,
    Hunks,
}

/// Which way a fold key moves the group under the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fold {
    Toggle,
    Open,
    Close,
    OpenAll,
    CloseAll,
}

type Key = (KeyCode, KeyModifiers);
/// An optional prefix key (for chords like `C-w C-w`), the key, and its action.
type Bind = (Option<Key>, Key, Action);

fn ch(c: char) -> Key {
    (KeyCode::Char(c), KeyModifiers::NONE)
}
fn ctrl(c: char) -> Key {
    (KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn plain(c: KeyCode) -> Key {
    (c, KeyModifiers::NONE)
}
fn ctrlk(c: KeyCode) -> Key {
    (c, KeyModifiers::CONTROL)
}

/// An uppercase char already carries its case, so SHIFT is dropped there to keep
/// the tables independent of how a terminal reports it.
fn norm(code: KeyCode, mods: KeyModifiers) -> Key {
    match code {
        KeyCode::Char(_) => (code, mods.difference(KeyModifiers::SHIFT)),
        _ => (code, mods),
    }
}

#[derive(Clone)]
struct Keymap {
    name: &'static str,
    hint: &'static str,
    binds: Vec<Bind>,
}

enum Resolve {
    Act(Action),
    /// The key opens a chord; hold it and wait for the next one.
    Pending,
    Miss,
}

impl Keymap {
    fn resolve(&self, pending: Option<Key>, key: Key) -> Resolve {
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

fn keymap(name: &str) -> Option<Keymap> {
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
enum Category {
    General,
    Navigation,
    Panes,
    Search,
    Review,
    Editor,
    Help,
}

fn category_label(c: Category) -> &'static str {
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
fn action_help(a: Action) -> (Category, &'static str) {
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
        Action::FocusNext => (Category::Panes, "focus the next pane"),
        Action::FocusPrev => (Category::Panes, "focus the previous pane"),
        Action::Focus(Pane::List) => (Category::Panes, "focus the reading-order pane"),
        Action::Focus(Pane::Code) => (Category::Panes, "focus the code pane"),
        Action::Focus(Pane::Why) => (Category::Panes, "focus the why pane"),
        Action::CursorLeft => (Category::Navigation, "move the code cursor left"),
        Action::CursorRight => (Category::Navigation, "move the code cursor right"),
        Action::WordNext => (Category::Navigation, "move the code cursor to the next word"),
        Action::WordPrev => (Category::Navigation, "move the code cursor to the previous word"),
        Action::WordEnd => (Category::Navigation, "move the code cursor to the end of the word"),
        Action::LineStart => (Category::Navigation, "move to line start / pane top"),
        Action::LineEnd => (Category::Navigation, "move to line end / pane bottom"),
        Action::ParaPrev => (Category::Navigation, "jump to the previous blank line"),
        Action::ParaNext => (Category::Navigation, "jump to the next blank line"),
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
            "open the command bar (:only-comments, :all, :filter, :keys, :strategy, :goto, :quickfix, :help, :q)",
        ),
        Action::CommandGoto => (Category::General, "open the command bar pre-filled with `goto `"),
    }
}

/// Every action, by the name a config file calls it. The inverse of
/// `key_label`'s job: `key_label` renders a key for humans, this names an
/// action for them. One table, used to parse a config and to check it — a name
/// missing here simply cannot be bound, and the error says which names exist.
const ACTION_NAMES: &[(&str, Action)] = &[
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
    ("cursor-left", Action::CursorLeft),
    ("cursor-right", Action::CursorRight),
    ("word-next", Action::WordNext),
    ("word-prev", Action::WordPrev),
    ("word-end", Action::WordEnd),
    ("line-start", Action::LineStart),
    ("line-end", Action::LineEnd),
    ("para-prev", Action::ParaPrev),
    ("para-next", Action::ParaNext),
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

fn action_by_name(name: &str) -> Option<Action> {
    ACTION_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, a)| *a)
}

/// Parse one key as a config writes it: `j`, `C-w`, `S-F3`, `Esc`, `Space`.
/// The inverse of `key_label`, and checked against it by test.
fn parse_key(text: &str) -> Option<Key> {
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
struct KeyConfig {
    preset: Option<String>,
    binds: Vec<(Option<Key>, Key, Option<Action>)>,
    /// `[theme] name = "…"`, and any per-role `#rrggbb` overrides on top of it
    theme: Option<String>,
    colors: Vec<(String, Color)>,
    problems: Vec<String>,
}

/// `#rrggbb` (or `rrggbb`) as a colour. Deliberately the only accepted form: a
/// theme file names colours the way every palette publishes them.
fn parse_hex(text: &str) -> Option<Color> {
    let t = text.trim().trim_start_matches('#');
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex(u32::from_str_radix(t, 16).ok()?))
}

/// Declares the theme roles a config file may set, each the name of a `Theme`
/// field, generating the write side (`apply_theme_colors`) and read side
/// (`theme_role_color`) from one list — so a role can't drift between the
/// two, which is how "match-bg" once read back the wrong field.
macro_rules! theme_roles {
    ($($role:literal => $($seg:ident).+),* $(,)?) => {
        const THEME_ROLES: &[&str] = &[$($role),*];

        /// Overlay a config's colour overrides onto a theme. Unknown roles are
        /// rejected at parse time, so everything reaching here names a field.
        fn apply_theme_colors(mut t: Theme, colors: &[(String, Color)]) -> Theme {
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
        fn theme_role_color(t: &Theme, role: &str) -> Color {
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

// ------------------------------------------------------------ reviewing rules

/// Where rules come from, in the order they are read: this user's own, then the
/// repository's. Both apply — a personal preference and a team convention are
/// different things, and a reviewer wants both. The repo's file is read last so
/// its rules are reported after the user's on a hunk they both match.
///
/// Rule *files* are the client's business: the engine reads nothing (see
/// `ordo::model::Options::rules`), which is what keeps `ordo order --json` a
/// function of its arguments and the corpus tests meaningful.
/// The rulesets shipped with ordo, bundled so `include = ["go-uber-guide"]`
/// (or `--rules go-uber-guide`) needs no path. `rulesets/` is the source of
/// truth; a test checks every file there is listed here.
const PRESETS: &[(&str, &str)] = &[
    (
        "c-power-of-ten",
        include_str!("../../rulesets/c-power-of-ten.toml"),
    ),
    (
        "cpp-default-guidelines",
        include_str!("../../rulesets/cpp-default-guidelines.toml"),
    ),
    (
        "go-uber-guide",
        include_str!("../../rulesets/go-uber-guide.toml"),
    ),
    (
        "java-effective-java",
        include_str!("../../rulesets/java-effective-java.toml"),
    ),
    (
        "javascript-airbnb",
        include_str!("../../rulesets/javascript-airbnb.toml"),
    ),
    (
        "lua-style-guide",
        include_str!("../../rulesets/lua-style-guide.toml"),
    ),
    ("markdown", include_str!("../../rulesets/markdown.toml")),
    (
        "python-google-style",
        include_str!("../../rulesets/python-google-style.toml"),
    ),
    (
        "rust-api-guidelines",
        include_str!("../../rulesets/rust-api-guidelines.toml"),
    ),
    (
        "typescript-clean-code",
        include_str!("../../rulesets/typescript-clean-code.toml"),
    ),
];

fn preset(name: &str) -> Option<&'static str> {
    PRESETS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

fn rule_sources(repo_root: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(dir) = config_path().and_then(|p| p.parent().map(Path::to_path_buf)) {
        out.push(dir.join("rules.toml"));
    }
    if !repo_root.is_empty() {
        out.push(PathBuf::from(repo_root).join(".ordo").join("rules.toml"));
    }
    out
}

/// Everything `load_rules_report` learned: the rules to run, and the record a
/// reviewer needs to trust them — where each came from, which definitions
/// replaced an earlier one, which names were disabled. A silenced rule looks
/// exactly like a convention nobody breaks, so the silencing is shown.
struct RulesReport {
    rules: Vec<ordo::model::Rule>,
    problems: Vec<String>,
    /// (origin, active rules from it), in load order
    origins: Vec<(String, usize)>,
    replaced: Vec<String>,
    disabled: Vec<String>,
}

impl RulesReport {
    /// The `:rules` popup, one line per fact.
    fn lines(&self) -> Vec<String> {
        let mut out = vec![format!(
            "{} rule{} active",
            self.rules.len(),
            if self.rules.len() == 1 { "" } else { "s" }
        )];
        for (origin, n) in &self.origins {
            out.push(format!("  {n:>3}  {origin}"));
        }
        if !self.replaced.is_empty() {
            out.push(String::new());
            out.push("replaced (a later definition with the same name):".to_string());
            out.extend(self.replaced.iter().map(|r| format!("  {r}")));
        }
        if !self.disabled.is_empty() {
            out.push(String::new());
            out.push("disabled:".to_string());
            out.extend(self.disabled.iter().map(|d| format!("  {d}")));
        }
        if !self.problems.is_empty() {
            out.push(String::new());
            out.push("problems:".to_string());
            out.extend(self.problems.iter().map(|p| format!("  {p}")));
        }
        if self.rules.is_empty() && self.origins.is_empty() {
            out.push(String::new());
            out.push("no rules loaded — `include = [\"go-uber-guide\"]` in .ordo/rules.toml, or --rules <preset|file>".to_string());
        }
        out
    }
}

/// Merge one parsed rules document into the layered list: its `include`s first
/// (a bundled preset by name, or a path relative to the file), then its own
/// rules, where a name already present is *replaced* in place. Disables are
/// only collected here; they apply once everything is layered, so a user can
/// silence a rule the repo includes and the repo one a user includes.
#[allow(clippy::too_many_arguments)]
fn layer_rules(
    text: &str,
    origin: &str,
    base: &Path,
    depth: usize,
    layered: &mut Vec<(ordo::model::Rule, String)>,
    disables: &mut Vec<String>,
    replaced: &mut Vec<String>,
    problems: &mut Vec<String>,
) {
    if depth > 8 {
        problems.push(format!(
            "{origin}: include nesting deeper than 8 — a cycle?"
        ));
        return;
    }
    let doc = parse_rules_doc(text, base);
    for p in doc.problems {
        problems.push(format!("{origin}: {p}"));
    }
    for inc in &doc.include {
        if let Some(t) = preset(inc) {
            layer_rules(
                t,
                inc,
                Path::new("."),
                depth + 1,
                layered,
                disables,
                replaced,
                problems,
            );
        } else {
            let path = base.join(inc);
            match std::fs::read_to_string(&path) {
                Ok(t) => {
                    let label = path.display().to_string();
                    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
                    layer_rules(
                        &t,
                        &label,
                        &parent,
                        depth + 1,
                        layered,
                        disables,
                        replaced,
                        problems,
                    );
                }
                Err(e) => problems.push(format!("{origin}: include `{inc}`: {e}")),
            }
        }
    }
    disables.extend(doc.disable);
    for rule in doc.rules {
        match layered.iter().position(|(r, _)| r.name == rule.name) {
            Some(i) => {
                replaced.push(format!("{}  ({} → {origin})", rule.name, layered[i].1));
                layered[i] = (rule, origin.to_string());
            }
            None => layered.push((rule, origin.to_string())),
        }
    }
}

/// Read the rule files that exist — this user's, then this repository's, then
/// any `--rules` file or preset — layer them, then apply every `disable`.
fn load_rules_report(repo_root: &str, extra: &[String]) -> RulesReport {
    report_from(rule_sources(repo_root), extra)
}

/// `implicit` sources (the user's and the repo's files) may be absent; every
/// `extra` — a `--rules` argument — was asked for, so its absence is reported,
/// unless it names a bundled preset.
fn report_from(implicit: Vec<PathBuf>, extra: &[String]) -> RulesReport {
    let mut layered: Vec<(ordo::model::Rule, String)> = vec![];
    let mut disables = vec![];
    let mut replaced = vec![];
    let mut problems = vec![];
    let n_implicit = implicit.len();
    for (i, src) in implicit
        .into_iter()
        .chain(extra.iter().map(PathBuf::from))
        .enumerate()
    {
        let implicit = i < n_implicit;
        let name = src.to_string_lossy().into_owned();
        if !implicit && !src.exists() {
            if let Some(t) = preset(&name) {
                layer_rules(
                    t,
                    &name,
                    Path::new("."),
                    0,
                    &mut layered,
                    &mut disables,
                    &mut replaced,
                    &mut problems,
                );
                continue;
            }
        }
        let text = match std::fs::read_to_string(&src) {
            Ok(t) => t,
            Err(_) if implicit => continue,
            Err(e) => {
                problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        let base = src.parent().unwrap_or(Path::new(".")).to_path_buf();
        layer_rules(
            &text,
            &name,
            &base,
            0,
            &mut layered,
            &mut disables,
            &mut replaced,
            &mut problems,
        );
    }
    // disables win, whoever wrote them
    let mut set = globset::GlobSetBuilder::new();
    for d in &disables {
        match globset::Glob::new(d) {
            Ok(g) => {
                set.add(g);
            }
            Err(e) => problems.push(format!("disable `{d}` is not a glob: {e}")),
        }
    }
    let set = set.build().unwrap_or_else(|_| globset::GlobSet::empty());
    let mut disabled = vec![];
    layered.retain(|(r, origin)| {
        let keep = !set.is_match(&r.name);
        if !keep {
            disabled.push(format!("{}  ({origin})", r.name));
        }
        keep
    });
    let mut origins: Vec<(String, usize)> = vec![];
    for (_, origin) in &layered {
        match origins.iter_mut().find(|(o, _)| o == origin) {
            Some((_, n)) => *n += 1,
            None => origins.push((origin.clone(), 1)),
        }
    }
    RulesReport {
        rules: layered.into_iter().map(|(r, _)| r).collect(),
        problems,
        origins,
        replaced,
        disabled,
    }
}

#[cfg(test)]
fn load_rules(repo_root: &str, extra: &[String]) -> (Vec<ordo::model::Rule>, Vec<String>) {
    let r = load_rules_report(repo_root, extra);
    (r.rules, r.problems)
}

/// `kind = "x"` and `kind = ["x", "y"]` both read; a one-entry list is the
/// common case and shouldn't need brackets. Mirrors `model::string_or_vec`,
/// which is private to that module.
fn string_or_vec<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<V>::deserialize(d)? {
        None => None,
        Some(V::One(s)) => Some(vec![s]),
        Some(V::Many(v)) => Some(v),
    })
}

/// Flat TOML shape of one `[[rule]]` block: the file keeps rule fields and
/// `when` conditions in one table, while `ordo::model::Rule` nests the
/// conditions under `when` — this is the shape that gets converted.
#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RuleToml {
    name: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    warn: Option<String>,
    #[serde(default)]
    noise: bool,
    #[serde(default)]
    priority: i64,

    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    path_not: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    category: Option<ordo::model::Category>,
    #[serde(default)]
    enclosing_kind: Option<String>,
    #[serde(default)]
    defines: Option<String>,
    #[serde(default)]
    uses: Option<String>,
    #[serde(default)]
    imports: Option<String>,
    #[serde(default)]
    noise_when: Option<bool>,
    #[serde(default)]
    comment: Option<bool>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    query_file: Option<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    kind: Option<Vec<String>>,
    #[serde(default, deserialize_with = "string_or_vec")]
    with: Option<Vec<String>>,
    #[serde(default, deserialize_with = "string_or_vec")]
    without: Option<Vec<String>>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    text_not: Option<String>,
    #[serde(default)]
    max_params: Option<usize>,
    #[serde(default)]
    max_lines: Option<usize>,
    #[serde(default)]
    max_nesting: Option<usize>,
    #[serde(default)]
    max_file_lines: Option<usize>,
    #[serde(default)]
    recursive: Option<bool>,
    #[serde(default)]
    container_with: Option<String>,
    #[serde(default)]
    container_without: Option<String>,
    #[serde(default)]
    member_uninitialized: Option<bool>,
}

/// Convert one already-parsed `[[rule]]` table into a `Rule`, independently of
/// every other rule in the file — a bad type or an unknown key in one block
/// must not cost the file its other, good rules. `query-file` is read
/// relative to `base` and lands in `When.query`, same as an inline `query`;
/// a missing file is a problem, not a panic.
fn rule_from_toml(
    v: toml::Value,
    idx: usize,
    base: &Path,
    problems: &mut Vec<String>,
) -> Option<ordo::model::Rule> {
    let label = v
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("rule {}", idx + 1));
    let parsed: RuleToml = match serde::Deserialize::deserialize(v) {
        Ok(p) => p,
        Err(e) => {
            problems.push(format!("{label}: {e}"));
            return None;
        }
    };
    let mut when = ordo::model::When {
        path: parsed.path,
        path_not: parsed.path_not,
        lang: parsed.lang,
        category: parsed.category,
        enclosing_kind: parsed.enclosing_kind,
        defines: parsed.defines,
        uses: parsed.uses,
        imports: parsed.imports,
        noise: parsed.noise_when,
        comment: parsed.comment,
        query: parsed.query,
        kind: parsed.kind,
        with: parsed.with,
        without: parsed.without,
        text: parsed.text,
        text_not: parsed.text_not,
        max_params: parsed.max_params,
        max_lines: parsed.max_lines,
        max_nesting: parsed.max_nesting,
        max_file_lines: parsed.max_file_lines,
        recursive: parsed.recursive,
        container_with: parsed.container_with,
        container_without: parsed.container_without,
        member_uninitialized: parsed.member_uninitialized,
    };
    if let Some(qf) = &parsed.query_file {
        match std::fs::read_to_string(base.join(qf)) {
            Ok(q) => when.query = Some(q),
            Err(e) => problems.push(format!("{label}: {qf}: {e}")),
        }
    }
    Some(ordo::model::Rule {
        name: parsed.name,
        when,
        note: parsed.note,
        warn: parsed.warn,
        noise: parsed.noise,
        priority: parsed.priority,
    })
}

/// The rules file: a sequence of `[[rule]]` blocks, real TOML — arrays
/// (`kind = [...]`) and multi-line `'''...'''` query strings read like
/// anywhere else in TOML. A syntax error, or a key outside `rule`, fails the
/// whole file (there is no document to salvage rules from); once the document
/// itself parses, each rule converts independently so one bad rule can't sink
/// the rest (see `rule_from_toml`).
/// One rules file, read: its rules, what it includes, what it disables.
struct RulesDoc {
    rules: Vec<ordo::model::Rule>,
    include: Vec<String>,
    disable: Vec<String>,
    problems: Vec<String>,
}

fn parse_rules_doc(text: &str, base: &Path) -> RulesDoc {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RulesFile {
        #[serde(default)]
        include: Vec<String>,
        #[serde(default)]
        disable: Vec<String>,
        #[serde(default)]
        rule: Vec<toml::Value>,
    }
    let empty = |problems| RulesDoc {
        rules: vec![],
        include: vec![],
        disable: vec![],
        problems,
    };
    let doc: RulesFile = match toml::from_str(text) {
        Ok(d) => d,
        Err(e) => return empty(vec![e.to_string()]),
    };
    let mut rules: Vec<ordo::model::Rule> = vec![];
    let mut problems = vec![];
    for (i, v) in doc.rule.into_iter().enumerate() {
        if let Some(r) = rule_from_toml(v, i, base, &mut problems) {
            // the engine keys hits by name; two rules sharing one within a
            // file would be indistinguishable, so it is a mistake to report
            if rules.iter().any(|x| x.name == r.name) {
                problems.push(format!("rule `{}` is defined twice in this file", r.name));
                continue;
            }
            rules.push(r);
        }
    }
    RulesDoc {
        rules,
        include: doc.include,
        disable: doc.disable,
        problems,
    }
}

#[cfg(test)]
fn parse_rules(text: &str, base: &Path) -> (Vec<ordo::model::Rule>, Vec<String>) {
    let d = parse_rules_doc(text, base);
    (d.rules, d.problems)
}

/// The config file ordo would write for the current preset and theme — every
/// binding and every colour, commented out, at its real value.
///
/// Generated from the same tables the program reads (`keymap`, `ACTION_NAMES`,
/// `THEME_ROLES`, the resolved `Theme`), never from a hand-written template, so
/// it cannot drift from what the program actually accepts. A test uncomments
/// the whole thing and checks it parses with no complaints and changes nothing.
fn init_config(preset: &str, theme_name: &str) -> String {
    let mut out = String::new();
    let km = keymap(preset).unwrap_or_else(|| keymap("vim").expect("vim preset exists"));
    let t = theme(theme_name).unwrap_or_else(|| theme("dark").expect("dark theme exists"));
    for line in [
        "# ordo configuration — every line below is this build's own default,",
        "# commented out. Uncomment and edit what you want to change.",
        "#",
        "# Written by `ordo --init-config`; the values are this build's, for",
        &format!("# preset `{preset}` and theme `{theme_name}`."),
    ] {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "preset = \"{}\"", km.name);
    let _ = writeln!(out, "theme = \"{}\"\n", t.name);

    for line in [
        "",
        "# ---------------------------------------------------------------- keys",
        "#",
        "# A line binds one key to one action; the action `none` removes a binding.",
        "# A chord is two keys separated by a space: `\"g d\"`, `\"C-w l\"`.",
        "# Modifiers are `C-`, `S-`, `A-`; named keys are Esc, Enter, Tab, Space,",
        "# Backspace, Up, Down, Left, Right, Home, End, PageUp, PageDown, F1..F12.",
        "[binds]",
    ] {
        let _ = writeln!(out, "{line}");
    }
    for (prefix, key, action) in &km.binds {
        let keys = match prefix {
            Some(p) => format!("{} {}", key_label(*p), key_label(*key)),
            None => key_label(*key),
        };
        let name = ACTION_NAMES
            .iter()
            .find(|(_, a)| a == action)
            .map(|(n, _)| *n)
            .unwrap_or("");
        let (_, help) = action_help(*action);
        let _ = writeln!(out, "# \"{keys}\" = \"{name}\"  # {help}");
    }

    for line in [
        "".to_string(),
        "# --------------------------------------------------------------- theme".to_string(),
        "#".to_string(),
        "# `name` picks a built-in palette; the roles below override it, as #rrggbb.".to_string(),
        format!("# Built-in: {}.", theme_names().join(", ")),
        "[theme]".to_string(),
        format!("# name = \"{}\"", t.name),
    ] {
        let _ = writeln!(out, "{line}");
    }
    // a terminal theme leaves some roles to the terminal's own palette: there
    // is no honest hex to print for those, and printing a placeholder would
    // mean this file stops being valid the moment someone uncomments it
    let inherited: Vec<&str> = THEME_ROLES
        .iter()
        .filter(|r| !matches!(theme_role_color(&t, r), Color::Rgb(..)))
        .copied()
        .collect();
    if !inherited.is_empty() {
        let _ = writeln!(
            out,
            "# These follow the terminal's own palette on this theme, so they have no\n             # default to show — set any of them to a colour to take it over:\n             #   {}",
            inherited.join(", ")
        );
    }
    for role in THEME_ROLES {
        if let Color::Rgb(r, g, b) = theme_role_color(&t, role) {
            let _ = writeln!(out, "# {role} = \"#{r:02x}{g:02x}{b:02x}\"");
        }
    }
    out
}

/// `--init-config`: write the generated config, refusing to clobber one that
/// already exists unless asked. Reports the path either way — the file is no
/// use if the reviewer can't find it.
fn write_init_config(preset: &str, theme_name: &str, force: bool) -> Result<(), i32> {
    let Some(path) = config_path() else {
        eprintln!("ordo: no config directory (set $XDG_CONFIG_HOME or $HOME)");
        return Err(2);
    };
    if path.exists() && !force {
        eprintln!(
            "ordo: {} already exists — pass --force to overwrite it",
            path.display()
        );
        return Err(1);
    }
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("ordo: {}: {e}", dir.display());
            return Err(1);
        }
    }
    match std::fs::write(&path, init_config(preset, theme_name)) {
        Ok(()) => {
            println!("wrote {}", path.display());
            Err(0)
        }
        Err(e) => {
            eprintln!("ordo: {}: {e}", path.display());
            Err(1)
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("ordo").join("tui.toml"))
}

/// A config line without its trailing comment. `#` opens a comment only
/// *outside* quotes: a colour is written `"#89b4fa"`, and cutting at the first
/// `#` regardless would eat every palette value in the file.
fn strip_comment(line: &str) -> &str {
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

/// Which section of the config the reader is in.
#[derive(PartialEq, Eq)]
enum Section {
    Top,
    Binds,
    Theme,
}

fn parse_key_config(text: &str) -> KeyConfig {
    let mut cfg = KeyConfig {
        preset: None,
        binds: vec![],
        theme: None,
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
        match section {
            Section::Top => match k.as_str() {
                "preset" => cfg.preset = Some(v),
                // `theme` reads naturally at the top of the file as well as
                // inside `[theme]`, and a config is read, not just written
                "theme" => cfg.theme = Some(v),
                _ => cfg
                    .problems
                    .push(format!("line {}: unknown setting `{k}`", n + 1)),
            },
            Section::Theme => {
                if k == "name" {
                    cfg.theme = Some(v);
                } else if !THEME_ROLES.contains(&k.as_str()) {
                    cfg.problems
                        .push(format!("line {}: unknown theme role `{k}`", n + 1));
                } else {
                    match parse_hex(&v) {
                        Some(c) => cfg.colors.push((k, c)),
                        None => cfg
                            .problems
                            .push(format!("line {}: `{v}` is not a #rrggbb colour", n + 1)),
                    }
                }
            }
            Section::Binds => {
                let Some((prefix, key)) = parse_bind_keys(&k) else {
                    cfg.problems
                        .push(format!("line {}: `{k}` is not a key", n + 1));
                    continue;
                };
                if v == "none" {
                    cfg.binds.push((prefix, key, None));
                    continue;
                }
                match action_by_name(&v) {
                    Some(a) => cfg.binds.push((prefix, key, Some(a))),
                    None => cfg
                        .problems
                        .push(format!("line {}: unknown action `{v}`", n + 1)),
                }
            }
        }
    }
    cfg
}

/// Apply overrides to a preset: a bound key replaces whatever held it, and
/// `none` removes it. Order is the config's, so a file can be read top to
/// bottom to know what it did.
fn apply_key_config(mut km: Keymap, cfg: &KeyConfig) -> Keymap {
    for (prefix, key, action) in &cfg.binds {
        km.binds.retain(|(p, k, _)| !(p == prefix && k == key));
        if let Some(a) = action {
            km.binds.push((*prefix, *key, *a));
        }
    }
    km
}

/// One key, rendered legibly: `C-`/`S-`/`A-` modifier prefixes, named special
/// keys, `F<n>` for function keys, the char itself otherwise.
fn key_label(key: Key) -> String {
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

/// A bind's full chord: `gg`/`ge` (no space — vim's own convention for a
/// plain-char chord) vs. `C-w C-w` (spaced — either half carries a modifier
/// or a named key, and vim always writes those chords spaced).
fn chord_label(prefix: Option<Key>, key: Key) -> String {
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
fn build_help(keys: &Keymap) -> Vec<String> {
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
    let order = [
        Category::General,
        Category::Navigation,
        Category::Panes,
        Category::Search,
        Category::Review,
        Category::Editor,
        Category::Help,
    ];
    let mut out = vec![];
    for &cat in &order {
        let group: Vec<&Row> = rows.iter().filter(|r| r.category == cat).collect();
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(category_label(cat).to_uppercase());
        for r in group {
            out.push(format!("  {:<16} {}", r.keys.join(", "), r.desc));
        }
    }
    out
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
    // hunk id -> the ledger entry anchored to it
    let ledger_at: HashMap<&str, usize> = out
        .ledger
        .iter()
        .enumerate()
        .map(|(i, e)| (e.at.as_str(), i))
        .collect();
    let by_id: HashMap<&str, (&str, &ordo::model::HunkOut)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks
                .iter()
                .map(move |h| (h.id.as_str(), (f.path.as_str(), h)))
        })
        .collect();
    let loc = |id: &str| {
        by_id
            .get(id)
            .map(|(p, h)| format!("{p}:L{}", h.new_range[0]))
            .unwrap_or_else(|| id.to_string())
    };
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
            let cat = format!("{:?}", h.category).to_lowercase();
            let warned = h.rules.iter().any(|r| r.level == "warn");
            let mark = if !h.advisories.is_empty() || warned {
                "⚠ "
            } else if h.noise {
                "· "
            } else {
                ""
            };
            let edges = out
                .edges
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
                .collect();
            let ledger = ledger_at.get(h.id.as_str()).copied();
            Some(Item {
                path: path.to_string(),
                bucket: bucket_key(ViewMode::Ledger, ledger, &h.group),
                ledger,
                old_range: h.old_range,
                new_range: h.new_range,
                mark: mark.to_string(),
                cat: cat.clone(),
                rationale: h.rationale.clone(),
                details: h.details.clone(),
                notes: h.notes.clone(),
                edges,
                advisories: h
                    .advisories
                    .iter()
                    .map(|a| (a.construct.clone(), a.message.clone(), a.verdict))
                    .collect(),
                noise: h.noise,
                comment: h.comment,
                symbols: h.symbols.clone(),
                enclosing: h.enclosing.clone(),
                group: h.group.clone(),
                rules: h.rules.clone(),
                refined: ordo::refine::Refined::default(),
                cluster: cluster_of.get(h.id.as_str()).copied(),
            })
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
        labels.insert("L-".to_string(), "no symbol changed".to_string());
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

// ------------------------------------------------------------------- highlight

// tree-sitter highlight capture names we color, with their fg. The `Highlight`
// index a walk yields is the position of the matched name in this list.
/// Highlight capture → role. The names are tree-sitter's; the roles are what a
/// theme colours. Verified against each grammar's own highlights query.
const HL: &[(&str, Role)] = &[
    ("attribute", Role::Attribute),
    ("boolean", Role::Number),
    ("comment", Role::Comment),
    ("constant", Role::Number),
    ("constant.builtin", Role::Number),
    ("constructor", Role::Type),
    ("escape", Role::Number),
    ("function", Role::Function),
    ("function.builtin", Role::Function),
    ("function.method", Role::Function),
    ("keyword", Role::Keyword),
    ("label", Role::Attribute),
    ("number", Role::Number),
    ("operator", Role::Operator),
    ("property", Role::Property),
    ("punctuation", Role::Operator),
    ("punctuation.bracket", Role::Operator),
    ("punctuation.delimiter", Role::Operator),
    ("punctuation.special", Role::Operator),
    ("string", Role::Str),
    ("string.escape", Role::Number),
    ("string.special", Role::Str),
    ("tag", Role::Attribute),
    ("text.emphasis", Role::Keyword),
    ("text.literal", Role::Str),
    ("text.reference", Role::Property),
    ("text.strong", Role::Keyword),
    ("text.title", Role::Function),
    ("text.uri", Role::Property),
    ("type", Role::Type),
    ("type.builtin", Role::Type),
    ("variable", Role::Variable),
    ("variable.builtin", Role::Builtin),
    ("variable.parameter", Role::Param),
];

type LineSpans = Vec<(String, Color)>;
type Highlights = HashMap<String, Vec<LineSpans>>;

// grammar + highlights query for a path (mirrors the engine's extension map;
// kept here because highlighting is a TUI-only presentation concern). The query
// is owned so cpp can inherit C's rules (Neovim `; inherits: c`, which
// tree-sitter-highlight doesn't resolve) by prepending the C query.
fn owned_query(l: tree_sitter::Language, q: &str) -> (tree_sitter::Language, String) {
    (l, q.to_string())
}

fn highlight_spec(path: &str) -> Option<(tree_sitter::Language, String)> {
    // a template highlights as the format underneath it (`values.yaml.j2` is
    // yaml with jinja in it); the jinja itself is left plain, which is close
    // enough to how most editors render one
    let path = match path.rsplit_once('.') {
        Some((head, "j2" | "jinja" | "jinja2" | "tmpl" | "tpl" | "erb" | "ejs" | "gotmpl")) => head,
        _ => path,
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    let (path, name) = match name.rsplit_once('.') {
        Some((head, "local")) if !head.is_empty() => {
            (path.strip_suffix(".local").unwrap_or(path), head)
        }
        _ => (path, name),
    };
    if name == "CMakeLists.txt" {
        return Some(owned_query(
            tree_sitter_cmake::LANGUAGE.into(),
            tree_sitter_cmake::HIGHLIGHTS_QUERY,
        ));
    }
    if matches!(name, ".bashrc" | ".bash_profile" | ".profile" | ".env") {
        return Some(owned_query(
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY,
        ));
    }
    if matches!(
        name,
        "Makefile" | "makefile" | "GNUmakefile" | "Makefile.am" | "Makefile.in"
    ) {
        return Some(owned_query(
            tree_sitter_make::LANGUAGE.into(),
            tree_sitter_make::HIGHLIGHTS_QUERY,
        ));
    }
    let ini_by_name = matches!(
        name,
        ".gitconfig"
            | ".gitmodules"
            | ".editorconfig"
            | ".npmrc"
            | ".hgrc"
            | ".flake8"
            | ".pylintrc"
            | ".coveragerc"
    ) || path.ends_with(".git/config")
        || path.ends_with(".dvc/config");
    if ini_by_name {
        return Some(owned_query(
            tree_sitter_ini::LANGUAGE.into(),
            tree_sitter_ini::HIGHLIGHTS_QUERY,
        ));
    }
    let ext = name.rsplit('.').next()?;
    let owned = |l: tree_sitter::Language, q: &str| (l, q.to_string());
    Some(match ext {
        "py" | "pyi" => owned(
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
        ),
        // the xonsh crate does not export a highlights query yet; python's
        // compiles against the superset grammar, so the python half of a
        // `.xsh` file highlights and the shell forms stay plain
        "xsh" | "xonsh" | "xonshrc" => owned(
            tree_sitter_xonsh::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
        ),
        "js" | "mjs" | "cjs" | "jsx" => owned(
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::HIGHLIGHT_QUERY,
        ),
        "rs" => owned(
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
        "ts" | "mts" | "cts" => owned(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ),
        "tsx" => owned(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ),
        "go" => owned(
            tree_sitter_go::LANGUAGE.into(),
            tree_sitter_go::HIGHLIGHTS_QUERY,
        ),
        "c" | "h" => owned(
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY,
        ),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => (
            tree_sitter_cpp::LANGUAGE.into(),
            format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            ),
        ),
        "java" => owned(
            tree_sitter_java::LANGUAGE.into(),
            tree_sitter_java::HIGHLIGHTS_QUERY,
        ),
        "lua" => owned(
            tree_sitter_lua::LANGUAGE.into(),
            tree_sitter_lua::HIGHLIGHTS_QUERY,
        ),
        "toml" => owned(
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        ),
        "json" => owned(
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY,
        ),
        "yml" | "yaml" => owned(
            tree_sitter_yaml::LANGUAGE.into(),
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
        ),
        "cmake" => owned(
            tree_sitter_cmake::LANGUAGE.into(),
            tree_sitter_cmake::HIGHLIGHTS_QUERY,
        ),
        "mk" | "mak" | "make" => owned(
            tree_sitter_make::LANGUAGE.into(),
            tree_sitter_make::HIGHLIGHTS_QUERY,
        ),
        "sh" | "bash" => owned(
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY,
        ),
        "html" | "htm" | "vue" => owned(
            tree_sitter_html::LANGUAGE.into(),
            tree_sitter_html::HIGHLIGHTS_QUERY,
        ),
        "svelte" => owned(
            tree_sitter_svelte_ng::LANGUAGE.into(),
            tree_sitter_svelte_ng::HIGHLIGHTS_QUERY,
        ),
        "css" => owned(
            tree_sitter_css::LANGUAGE.into(),
            tree_sitter_css::HIGHLIGHTS_QUERY,
        ),
        "nix" => owned(
            tree_sitter_nix::LANGUAGE.into(),
            tree_sitter_nix::HIGHLIGHTS_QUERY,
        ),
        "ini" | "cfg" => owned(
            tree_sitter_ini::LANGUAGE.into(),
            tree_sitter_ini::HIGHLIGHTS_QUERY,
        ),
        "md" | "markdown" => (tree_sitter_md::LANGUAGE.into(), md_block_query()),
        _ => return None,
    })
}

// The block query's `[(link_title)(indented_code_block)(fenced_code_block)]
// @text.literal` wraps a fenced code block's *entire* span, content included,
// starting at the exact byte where the fence-language injection's own first
// token also starts. `tree-sitter-highlight` breaks that starting-byte tie by
// opening the *deeper* (injected) scope first, which — since scopes must
// close in the order they opened — forces that first token to stay "open"
// (and coloured) all the way to wherever `text.literal` closes, i.e. the rest
// of the fence. Dropping `fenced_code_block` from that one alternation
// (leaving `link_title`/`indented_code_block` untouched) removes the base
// layer's competing scope, so the fence body carries no ambient colour and
// the injected grammar alone colours it — which is also why `@none` on
// `code_fence_content` (the block query's own attempt at this) is left out
// of `HL` entirely rather than mapped to a role: giving it a colour of its
// own would reproduce the exact same starting-byte tie against the
// injection. Falls back to the query unmodified if upstream ever reformats
// that line — a missed match just brings the wash back, it doesn't break.
fn md_block_query() -> String {
    tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.replace("\n  (fenced_code_block)\n", "\n")
}

// Fence info-string (```rust, ```py, …) → a representative extension, so a
// fenced code block's language resolves through `highlight_spec` — the same
// table a real file uses — instead of a second copy of the grammar list.
// Mirrors `lang::for_lang_name`'s canonical names. A language ordo has no
// grammar for (`console`, `json`, `diff`, …) returns None and stays plain.
fn fence_ext(name: &str) -> Option<&'static str> {
    let word = name.trim().split([' ', ',', '{', ':']).next()?.trim();
    Some(match word.to_ascii_lowercase().as_str() {
        "py" | "python" | "python3" | "pyi" => "py",
        "xsh" | "xonsh" => "xsh",
        "js" | "javascript" | "node" | "mjs" | "cjs" | "jsx" => "js",
        "ts" | "typescript" | "mts" | "cts" => "ts",
        "tsx" => "tsx",
        "rs" | "rust" => "rs",
        "go" | "golang" => "go",
        "c" => "c",
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "lua" => "lua",
        "toml" => "toml",
        _ => return None,
    })
}

// Distinct fence languages (info-string text) named by fenced code blocks in
// `src`, in first-seen order. Parsed with the block grammar directly, ahead
// of highlighting, so the injected per-fence configs can be built before the
// highlighter borrows them.
fn md_fence_languages(src: &str) -> Vec<String> {
    let language: tree_sitter::Language = tree_sitter_md::LANGUAGE.into();
    let mut parser = Parser::new();
    let Ok(()) = parser.set_language(&language) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(
        &language,
        "(fenced_code_block (info_string (language) @lang))",
    ) else {
        return Vec::new();
    };
    let mut cursor = QueryCursor::new();
    let mut names = Vec::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src.as_bytes());
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if let Ok(text) = cap.node.utf8_text(src.as_bytes()) {
                if !names.iter().any(|n| n == text) {
                    names.push(text.to_string());
                }
            }
        }
    }
    names
}

// `tree_sitter_md::INJECTION_QUERY_BLOCK` looks like the obvious choice for
// both injections below, but under `tree-sitter-highlight` neither rule works
// as shipped, so this query is hand-written instead — don't "simplify" it
// back to the crate's constant, that regresses markdown highlighting
// silently:
//
// - `(inline) @injection.content (#set! injection.language
//   "markdown_inline")` produces zero highlight events: the callback does
//   get invoked for `markdown_inline`, but the layer it returns never emits
//   anything unless the injection also carries `injection.include-children`.
// - the fenced-code rule fires and *looks* right for a one-line body, but
//   `code_fence_content` is not one opaque text node — tree-sitter-markdown's
//   scanner splits it around a `block_continuation` per line *and* around
//   stray punctuation like `(`, `{`, `=`, `;` that it tokenizes for its own
//   purposes. Without `include-children`, those child ranges are excised
//   from what the fence-language parser sees, so it's handed something like
//   "fn add \n    let x  1\n\n" instead of the real source — which silently
//   breaks the fence grammar's own parse (a `let` after that mangled prefix
//   no longer parses as a keyword). `include-children` restores the full
//   contiguous span.
const MD_INJECTION_QUERY: &str = r#"
((inline) @injection.content
 (#set! injection.language "markdown_inline")
 (#set! injection.include-children))

(fenced_code_block
  (info_string (language) @injection.language)
  (code_fence_content) @injection.content
  (#set! injection.include-children))
"#;

thread_local! {
    // Compiling a `HighlightConfiguration` (parsing its query into a
    // capture-index table) is the expensive part of highlighting, and it only
    // depends on the (language, query, injection-query) triple — not on which
    // file it's for. Cache by the query text, which is a fixed string per
    // grammar (see `highlight_spec`/`md_block_query`), so a run touching many
    // files of one language compiles that language's query once. Lives on the
    // loader worker thread that calls `highlight_file`, so a plain
    // `thread_local!` needs no locking.
    static HL_CFG_CACHE: std::cell::RefCell<HashMap<String, HighlightConfiguration>> = std::cell::RefCell::new(HashMap::new());
}

// Ensures `cache[key]` holds a `HighlightConfiguration` for `language`/`query`,
// configured with `names` exactly once. Returns whether it's present after the
// call (false only if construction failed).
fn ensure_hl_cfg(
    cache: &std::cell::RefCell<HashMap<String, HighlightConfiguration>>,
    key: &str,
    language: tree_sitter::Language,
    name: &str,
    query: &str,
    injections: &str,
    names: &[&str],
) -> bool {
    if cache.borrow().contains_key(key) {
        return true;
    }
    let Ok(mut cfg) = HighlightConfiguration::new(language, name, query, injections, "") else {
        return false;
    };
    cfg.configure(names);
    cache.borrow_mut().insert(key.to_string(), cfg);
    true
}

// Syntax-highlight `src` into per-line colored segments. None when the language
// is unsupported or the grammar/query fails to build → caller renders plain.
fn highlight_file(path: &str, src: &str, syn: &Syntax) -> Option<Vec<LineSpans>> {
    let (language, query) = highlight_spec(path)?;
    let names: Vec<&str> = HL.iter().map(|(n, _)| *n).collect();
    let is_markdown = matches!(path.rsplit('.').next(), Some("md" | "markdown"));
    let injections = if is_markdown { MD_INJECTION_QUERY } else { "" };

    HL_CFG_CACHE.with(|cache| {
        if !ensure_hl_cfg(cache, &query, language, path, &query, injections, &names) {
            return None;
        }

        // Injected-layer configs: `markdown_inline` plus one per fenced-code
        // language actually present. Keyed into the same cache, by query text,
        // so they're built at most once per grammar too.
        let mut injected_keys: Vec<(String, String)> = Vec::new();
        if is_markdown {
            let inline_query = tree_sitter_md::HIGHLIGHT_QUERY_INLINE;
            if ensure_hl_cfg(
                cache,
                inline_query,
                tree_sitter_md::INLINE_LANGUAGE.into(),
                path,
                inline_query,
                "",
                &names,
            ) {
                injected_keys.push(("markdown_inline".to_string(), inline_query.to_string()));
            }
            for lang_name in md_fence_languages(src) {
                if injected_keys.iter().any(|(n, _)| *n == lang_name) {
                    continue;
                }
                let Some(ext) = fence_ext(&lang_name) else {
                    continue;
                };
                let Some((l, q)) = highlight_spec(&format!("x.{ext}")) else {
                    continue;
                };
                if ensure_hl_cfg(cache, &q, l, &lang_name, &q, "", &names) {
                    injected_keys.push((lang_name, q));
                }
            }
        }

        let cache_ref = cache.borrow();
        let cfg = cache_ref.get(&query)?;
        let injected: Vec<(String, &HighlightConfiguration)> = injected_keys
            .iter()
            .filter_map(|(n, k)| cache_ref.get(k).map(|c| (n.clone(), c)))
            .collect();

        let mut hl = Highlighter::new();
        let events = hl
            .highlight(cfg, src.as_bytes(), None, |name| {
                injected.iter().find(|(n, _)| n == name).map(|(_, c)| *c)
            })
            .ok()?;

        let mut lines: Vec<LineSpans> = vec![vec![]];
        let mut stack: Vec<Color> = vec![];
        for ev in events {
            match ev.ok()? {
                HighlightEvent::HighlightStart(h) => {
                    stack.push(HL.get(h.0).map(|(_, r)| syn.of(*r)).unwrap_or(syn.variable));
                }
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    let color = stack.last().copied().unwrap_or(syn.variable);
                    let mut first = true;
                    for piece in src.get(start..end).unwrap_or("").split('\n') {
                        if !first {
                            lines.push(vec![]);
                        }
                        first = false;
                        if !piece.is_empty() {
                            lines.last_mut().unwrap().push((piece.to_string(), color));
                        }
                    }
                }
            }
        }
        Some(lines)
    })
}

// ------------------------------------------------------------------- code view

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
const fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// What a highlight capture *means*. A theme colours these twelve roles rather
/// than the twenty-six capture names `HL` maps onto them, so adding a grammar's
/// capture never means touching every theme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
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
struct Syntax {
    comment: Color,
    keyword: Color,
    string: Color,
    number: Color,
    function: Color,
    type_: Color,
    property: Color,
    operator: Color,
    variable: Color,
    builtin: Color,
    param: Color,
    attribute: Color,
}

impl Syntax {
    fn of(&self, role: Role) -> Color {
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
struct Theme {
    name: &'static str,
    // ---- chrome
    /// default text; `Reset` on a terminal theme, so the terminal's own
    /// foreground shows through
    fg: Color,
    /// gutters, context line numbers, noise rows — present but recessive
    dim: Color,
    border: Color,
    /// the focused pane's border, the one piece of chrome that must be obvious
    border_focus: Color,
    /// line numbers in the reading order
    accent: Color,
    /// a hunk's `[category]`
    category: Color,
    /// the `⚠` advisory mark
    mark: Color,
    /// a reviewed row's `✓`
    reviewed: Color,
    /// an advisory's verdict line, and the command bar's error text
    warn: Color,
    // ---- diff
    add_fg: Color,
    del_fg: Color,
    add_bg: Color,
    del_bg: Color,
    /// the *changed part* of a line whose counterpart was identified — the
    /// line keeps the quiet add/del tint, and only what actually changed gets
    /// these (Neovim's DiffText over DiffChange)
    add_strong_bg: Color,
    del_strong_bg: Color,
    /// reading-order list's selected-row tint — a neutral slate, subtle next
    /// to add_bg/del_bg rather than the full fg/bg swap REVERSED gives
    select_bg: Color,
    /// search-match backgrounds — current match brighter than the rest, so
    /// it reads as "here" among however many others are also highlighted
    match_bg: Color,
    match_cur_bg: Color,
    // ---- code
    syn: Syntax,
}

impl Theme {
    /// The terminal's own palette for text, with truecolor diff tints. The
    /// default, and the only themes that inherit the user's terminal colours.
    fn terminal(name: &'static str, light: bool) -> Theme {
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

/// Every theme name, terminal ones first — the order `--theme` and `:theme`
/// report, and the order `:theme` completes in.
fn theme_names() -> Vec<String> {
    ["dark".to_string(), "light".to_string()]
        .into_iter()
        .chain(PALETTES.iter().map(|p| p.name.to_string()))
        .collect()
}

fn theme(name: &str) -> Option<Theme> {
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
fn slice_range(spans: Vec<Span<'static>>, start: usize, width: usize) -> Vec<Span<'static>> {
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

// The whole new file with the changed hunk highlighted in place: removed lines
// on a red-tinted row (shown at the change point), added lines on a green tint,
// the rest plain context. Code is syntax-highlighted via tree-sitter.
// prefix width: 1-char sign bar + 4-digit line number + 1 space
const GUTTER_W: usize = 1 + 5;

// rendering knobs (pane width, scroll offsets, cursor, active search) rather
// than a natural struct's worth of related data — bundling them wouldn't
// clarify the call sites below, just move the same count of fields around
#[allow(clippy::too_many_arguments)]
fn code_view(
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
    let num = Style::default().fg(theme.dim);
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
    // fill the rest of the row so the background tint spans the full width
    let pad = |spans: &mut Vec<Span<'static>>, used: usize, bg: Color| {
        if width > used {
            spans.push(Span::styled(
                " ".repeat(width - used),
                Style::default().bg(bg),
            ));
        }
    };
    // slice `content` (the code portion only — never the gutter built ahead
    // of it) to the horizontally visible window, reporting whether this line
    // has anything past the right edge of that window (the caller folds that
    // into `right_clip` — kept a plain function, not a mutably-capturing
    // closure, so it can be called from inside another closure below)
    let window = |content: Vec<Span<'static>>, len: usize| -> (Vec<Span<'static>>, usize, bool) {
        let clipped = len > hscroll + avail;
        let shown = len.saturating_sub(hscroll).min(avail);
        (slice_range(content, hscroll, avail), shown, clipped)
    };
    // a line paired with its counterpart (see `ordo::refine`) keeps the quiet
    // tint and gets the strong one only where it actually differs; an unpaired
    // line has no counterpart to compare against and tints whole
    let emphasize = |mut spans: Vec<Span<'static>>,
                     refined: Option<&Vec<(usize, usize)>>,
                     bg: Color|
     -> Vec<Span<'static>> {
        for &(cs, ce) in refined.map(|v| v.as_slice()).unwrap_or(&[]) {
            if ce <= hscroll || cs >= hscroll + avail {
                continue;
            }
            let (ls, le) = (cs.saturating_sub(hscroll), (ce - hscroll).min(avail));
            spans = overlay_range(spans, GUTTER_W + ls, GUTTER_W + le, |st| st.bg(bg));
        }
        spans
    };
    let emit_removed = |out: &mut Vec<Line<'static>>, row: &mut usize| {
        for (k, r) in removed.iter().enumerate() {
            let here = *row;
            *row += 1;
            if here < start || here >= end {
                continue;
            }
            let content = vec![Span::styled(
                (*r).clone(),
                Style::default().fg(theme.del_fg).bg(theme.del_bg),
            )];
            let (visible, shown, _) = window(content, r.chars().count());
            let mut spans = vec![
                Span::styled(BAR, Style::default().fg(theme.del_fg)),
                Span::styled("     ".to_string(), num.bg(theme.del_bg)),
            ];
            spans.extend(visible);
            pad(&mut spans, GUTTER_W + shown, theme.del_bg);
            spans = emphasize(
                spans,
                it.refined.removed.get(k).and_then(|s| s.as_ref()),
                theme.del_strong_bg,
            );
            out.push(Line::from(spans));
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
        let bg = if added { theme.add_bg } else { Color::Reset };
        let mut spans = vec![
            Span::styled(
                if added { BAR } else { " " },
                Style::default().fg(theme.add_fg),
            ),
            Span::styled(format!("{ln:>4} "), num.bg(bg)),
        ];
        // syntax-colored code segments (fall back to the raw line if unhighlighted)
        let content: Vec<Span<'static>> = match hl.and_then(|h| h.get(i)) {
            Some(segs) if !segs.is_empty() => segs
                .iter()
                .map(|(text, color)| Span::styled(text.clone(), Style::default().fg(*color).bg(bg)))
                .collect(),
            _ => vec![Span::styled(
                line.clone(),
                Style::default().fg(theme.fg).bg(bg),
            )],
        };
        let (visible, shown, _) = window(content, line.chars().count());
        spans.extend(visible);
        if added {
            pad(&mut spans, GUTTER_W + shown, bg);
            let k = ln - n0;
            spans = emphasize(
                spans,
                it.refined.added.get(k).and_then(|s| s.as_ref()),
                theme.add_strong_bg,
            );
        }
        for (mi, &(ml, s, e)) in matches.iter().enumerate() {
            // only a match that intersects the visible horizontal window can
            // be shown at all — one further off-screen is reached by jumping
            // to it (`n`/`N`, `*`/`#`), which scrolls the window to include it
            if ml != i || e <= hscroll || s >= hscroll + avail {
                continue;
            }
            let (ls, le) = (s.saturating_sub(hscroll), (e - hscroll).min(avail));
            let mbg = if Some(mi) == cur_match {
                theme.match_cur_bg
            } else {
                theme.match_bg
            };
            spans = overlay_range(spans, GUTTER_W + ls, GUTTER_W + le, |st| st.bg(mbg));
        }
        if let Some(c) =
            cursor.filter(|c| c.line == i && c.col >= hscroll && c.col < hscroll + avail)
        {
            spans = overlay_cursor(spans, GUTTER_W + (c.col - hscroll));
        }
        out.push(Line::from(spans));
    }
    if n0 > nl.len() {
        emit_removed(&mut out, &mut row); // deletion at/after EOF
    }
    (out, right_clip, total)
}

// ---------------------------------------------------------------------- hover

fn node_text(n: Node, src: &str) -> String {
    src.get(n.start_byte()..n.end_byte())
        .unwrap_or("")
        .to_string()
}

// byte offset of a char column within one line — tree-sitter Points are byte-indexed
fn char_byte(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

// node kinds counted as a "definition" worth showing, by file extension —
// deliberately a small, curated set rather than every grammar's declaration
// kinds, so a hover only ever lands on something with a clear signature/body.
fn def_kinds(path: &str) -> &'static [&'static str] {
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
fn find_definition<'a>(root: Node<'a>, kinds: &[&str], name: &str, src: &str) -> Option<Node<'a>> {
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
fn signature(n: Node, src: &str) -> String {
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

fn doc_for(path: &str, n: Node, src: &str) -> Option<String> {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" => python_docstring(n, src),
        _ => leading_comment(n, src),
    }
}

// Ensure `path`'s new-content is parsed and cached, returning the cached tree.
// Shared by `hover` and the symbol-occurrence search so both cache the same tree.
fn parse_cached<'a>(
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
fn identifier_at<'a>(parsed: &'a ParsedFile, nl: &[String], cursor: Cursor) -> Option<Node<'a>> {
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
fn hover(app: &mut App) {
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

// ------------------------------------------------------------ cross-commit history

// bound on how many candidate commits per direction (earlier/later) get run
// through the engine — a long-lived file's full history would otherwise stall
// the UI on a single `K` press
const HISTORY_WINDOW: usize = 10;

fn commit_list(args: &[&str]) -> Vec<String> {
    git(args)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// `scope.name`, or bare `name` at top level — matches the qualified form a
/// hunk's own `enclosing` field carries, so a body-edit match (found via
/// `enclosing`) and a defines-match (found via symbol identity) read the
/// same string in `label_for`.
fn qualified_name(sym: &Symbol) -> String {
    match &sym.scope {
        Some(s) => format!("{s}.{}", sym.name),
        None => sym.name.clone(),
    }
}

/// The project's identity rule, verbatim: "tree sitter type + scope for the
/// symbol must match, otherwise it's a different symbol". Name alone is never
/// enough — a method `run` on class `A` and a module-level `run` don't match,
/// nor do two same-named, same-scope defs of different tree-sitter kinds.
fn symbol_eq(a: &Symbol, b: &Symbol) -> bool {
    a.name == b.name && a.kind == b.kind && a.scope == b.scope
}

/// Bound `git rev-list <rev> -- <file>`'s output (`rev` itself first, if it
/// touched the file, then its nearest ancestor, then the next, ...) to the
/// nearest `window` ancestors, oldest-of-the-window first — so the entry
/// nearest `current` prints right above the CURRENT line.
fn bound_earlier(mut shas: Vec<String>, current: &str, window: usize) -> Vec<String> {
    if shas.first().map(String::as_str) == Some(current) {
        shas.remove(0);
    }
    shas.truncate(window);
    shas.reverse();
    shas
}

/// Bound `git rev-list <rev>..HEAD -- <file>`'s output (HEAD first, walking
/// back to the nearest descendant of `rev` last) to the nearest `window`
/// descendants, nearest-to-`rev` first — so the entry nearest `rev` prints
/// right below the CURRENT line.
fn bound_later(mut shas: Vec<String>, window: usize) -> Vec<String> {
    shas.reverse();
    shas.truncate(window);
    shas
}

/// The engine's identity (name + tree-sitter kind + qualified scope) for the
/// definition named `name` (tree-sitter kind `kind`) at 0-based `row` in
/// `content`. Runs the engine as though the whole file were freshly added, so
/// every definition in it — not just ones inside a hunk that happens to be
/// selected — shows up in some hunk's `symbols`, keyed to its own row.
fn symbol_identity(
    path: &str,
    content: &str,
    name: &str,
    kind: &str,
    row: usize,
) -> Option<Symbol> {
    let out = ordo::run(Input {
        changes: vec![Change {
            path: path.to_string(),
            old: Some(String::new()),
            new: Some(content.to_string()),
            diff: None,
        }],
        options: Options::default(),
    });
    let file = out.files.iter().find(|f| f.path == path)?;
    let line = row + 1;
    file.hunks
        .iter()
        .find(|h| h.new_range[0] <= line && line <= h.new_range[1])
        .and_then(|h| h.symbols.iter().find(|s| s.name == name && s.kind == kind))
        // a hunk-boundary mismatch in the synthetic whole-file diff shouldn't
        // lose the def entirely — any hunk carrying the right name+kind is it
        .or_else(|| {
            file.hunks
                .iter()
                .flat_map(|h| h.symbols.iter())
                .find(|s| s.name == name && s.kind == kind)
        })
        .cloned()
}

/// Pull the fragment of a (possibly multi-symbol) rationale that names this
/// symbol — "adds helper; changes signature of run" reads as just "changes
/// signature of run" for `run`'s own history line. Falls back to the whole
/// rationale, which covers the enclosing-only body-edit case ("edits
/// A.run") where the fragment already names nothing else.
fn label_for(h: &HunkOut, target: &Symbol) -> String {
    let qualified = qualified_name(target);
    h.rationale
        .split("; ")
        .find(|frag| frag.contains(target.name.as_str()) || frag.contains(qualified.as_str()))
        .unwrap_or(&h.rationale)
        .to_string()
}

/// What commit `sha` did to `target`, in ordo's own rationale wording — or
/// None when this commit touched `path` but never reached `target` itself (a
/// path-based window can't help but include unrelated changes to the file;
/// see `earlier`/`later` in `compute_history`).
fn classify_commit(sha: &str, path: &str, target: &Symbol) -> Option<String> {
    let parent = git(&["rev-parse", "--verify", "-q", &format!("{sha}^")]);
    let parent = parent.trim();
    let old = if parent.is_empty() {
        String::new()
    } else {
        git(&["show", &format!("{parent}:{path}")])
    };
    let new = git(&["show", &format!("{sha}:{path}")]);
    let out = ordo::run(Input {
        changes: vec![Change {
            path: path.to_string(),
            old: Some(old),
            new: Some(new),
            diff: None,
        }],
        options: Options::default(),
    });
    let file = out.files.iter().find(|f| f.path == path)?;
    let qualified = qualified_name(target);
    file.hunks.iter().find_map(|h| {
        let defines_it = h.symbols.iter().any(|s| symbol_eq(s, target));
        let body_edit = h.enclosing.as_deref() == Some(qualified.as_str());
        (defines_it || body_edit).then(|| label_for(h, target))
    })
}

// Direction semantics (decided): `earlier` = ancestors of the reviewed rev,
// `later` = descendants — topological, not date-based, so this is correct
// when reviewing an old or mid-stack commit. No `--follow`: `git rev-list`
// doesn't support it at all (only `git log` does — passing it errors), so a
// rename before the window's oldest commit silently ends this symbol's
// history there. This engine call is single-file, one commit at a time, so it
// sees only same-file edits — a call site added in another file never shows.
fn compute_history(review_sha: &str, path: &str, target: &Symbol) -> Vec<String> {
    let earlier_all = commit_list(&["rev-list", review_sha, "--", path]);
    let earlier = bound_earlier(earlier_all, review_sha, HISTORY_WINDOW);
    let later_all = commit_list(&["rev-list", &format!("{review_sha}..HEAD"), "--", path]);
    let later = bound_later(later_all, HISTORY_WINDOW);

    let mut out = vec![];
    history_lines_for(&earlier, "earlier", path, target, &mut out);
    out.push(format!("{:<7}  {}", "CURRENT", short_sha(review_sha)));
    history_lines_for(&later, "later", path, target, &mut out);
    out
}

// One direction's rows in `compute_history`'s output: `tag` labels only the
// first commit that actually classifies, matching the reading-order convention
// where a repeated column reads as blank rather than restating itself.
fn history_lines_for(
    shas: &[String],
    tag: &str,
    path: &str,
    target: &Symbol,
    out: &mut Vec<String>,
) {
    let mut first = true;
    for sha in shas {
        if let Some(label) = classify_commit(sha, path, target) {
            let tag = if first { tag } else { "" };
            first = false;
            out.push(format!("{:<7}  {}  {label}", tag, short_sha(sha)));
        }
    }
}

/// The `K` popup's history section: resolves the hovered def's identity, then
/// its cross-commit history (cached per identity so a repeat `K` is instant).
/// Computed synchronously on this keypress — no async restructure, kept fast
/// by `HISTORY_WINDOW` and the single-file-per-commit engine calls.
fn history_lines(
    app: &mut App,
    path: &str,
    name: &str,
    kind: &str,
    row: usize,
    content: &str,
) -> Vec<String> {
    let Some(review_sha) = app.review_sha.clone() else {
        return vec!["history: unavailable (no commit to review from)".to_string()];
    };
    let Some(target) = symbol_identity(path, content, name, kind, row) else {
        return vec!["history: could not resolve this symbol's identity".to_string()];
    };
    let key = (
        path.to_string(),
        target.name.clone(),
        target.kind.clone(),
        target.scope.clone(),
    );
    if let Some(cached) = app.history_cache.get(&key) {
        return cached.clone();
    }
    let lines = compute_history(&review_sha, path, &target);
    app.history_cache.insert(key, lines.clone());
    lines
}

// --------------------------------------------------------------------- search

// inverse of `char_byte`: the char index a byte offset falls at within one line
fn byte_to_char_col(line: &str, byte_col: usize) -> usize {
    line.char_indices()
        .take_while(|(b, _)| *b < byte_col)
        .count()
}

/// Every identifier node in the tree whose text equals `name`, as document-order
/// (line, start_col, end_col) char ranges. Never a text scan: a short name that
/// occurs as a substring of a longer identifier (`may_refine` inside
/// `may_refine_camber_span`) is never counted, because node text equality is
/// exact, not a substring test.
fn symbol_matches(
    root: Node,
    name: &str,
    src: &str,
    lines: &[String],
) -> Vec<(usize, usize, usize)> {
    let mut out = vec![];
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind().contains("identifier") && node_text(n, src) == name {
            let (sp, ep) = (n.start_position(), n.end_position());
            if sp.row == ep.row && sp.row < lines.len() {
                let scol = byte_to_char_col(&lines[sp.row], sp.column);
                let ecol = byte_to_char_col(&lines[sp.row], ep.column);
                out.push((sp.row, scol, ecol));
            }
        }
        let mut cursor = n.walk();
        stack.extend(n.children(&mut cursor));
    }
    out.sort();
    out
}

/// `/` text search: literal substring occurrences of `pattern`, one per
/// character position. This IS a text scan — the one place that's correct,
/// since the user is typing characters, not asking about a symbol.
fn text_matches(lines: &[String], pattern: &str) -> Vec<(usize, usize, usize)> {
    if pattern.is_empty() {
        return vec![];
    }
    let plen = pattern.chars().count();
    let mut out = vec![];
    for (i, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < plen {
            continue;
        }
        for start in 0..=chars.len() - plen {
            if chars[start..start + plen].iter().collect::<String>() == pattern {
                out.push((i, start, start + plen));
            }
        }
    }
    out
}

// first match at/after (inclusive) or strictly after (!inclusive) `cursor`,
// wrapping to the first match when nothing qualifies
fn seek_forward(
    matches: &[(usize, usize, usize)],
    cursor: Cursor,
    inclusive: bool,
) -> Option<usize> {
    let after = |l: usize, s: usize| {
        if inclusive {
            (l, s) >= (cursor.line, cursor.col)
        } else {
            (l, s) > (cursor.line, cursor.col)
        }
    };
    matches
        .iter()
        .position(|&(l, s, _)| after(l, s))
        .or((!matches.is_empty()).then_some(0))
}

// mirror of `seek_forward`, wrapping to the last match when nothing qualifies
fn seek_backward(
    matches: &[(usize, usize, usize)],
    cursor: Cursor,
    inclusive: bool,
) -> Option<usize> {
    let before = |l: usize, s: usize| {
        if inclusive {
            (l, s) <= (cursor.line, cursor.col)
        } else {
            (l, s) < (cursor.line, cursor.col)
        }
    };
    matches
        .iter()
        .rposition(|&(l, s, _)| before(l, s))
        .or_else(|| matches.len().checked_sub(1))
}

// `n`/`N` index arithmetic: advance/retreat by one, wrapping. `len == 0` never
// panics (rem_euclid by zero would) — it's the empty-result case.
fn cycle_index(index: usize, len: usize, dir: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + dir).rem_euclid(len as isize) as usize
}

// move the cursor to `search`'s current match and scroll to follow, then store it
fn jump_search(app: &mut App, mut search: Search, forward: bool, inclusive: bool) {
    if !search.matches.is_empty() {
        let idx = if forward {
            seek_forward(&search.matches, app.cursor, inclusive)
        } else {
            seek_backward(&search.matches, app.cursor, inclusive)
        };
        search.index = idx.unwrap_or(0);
        let (line, col, _) = search.matches[search.index];
        app.cursor = Cursor { line, col };
        app.scroll = follow_scroll(line, app.scroll, app.code_height);
        app.hscroll = follow_hscroll(col, app.hscroll, app.code_width);
    }
    app.search = Some(search);
}

/// `*` / `#` — the symbol under the cursor, resolved via tree-sitter (sharing
/// the `K` popup's parse cache), jumping to the next/previous occurrence.
fn symbol_search(app: &mut App, forward: bool) {
    let path = app.items[app.sel].path.clone();
    let Some((_, nl)) = app.sources.get(&path) else {
        return;
    };
    if nl.is_empty() {
        return;
    }
    let Some((lang, _)) = highlight_spec(&path) else {
        return;
    };
    let Some(parsed) = parse_cached(&mut app.trees, &path, nl, lang) else {
        return;
    };
    let Some(node) = identifier_at(parsed, nl, app.cursor) else {
        return;
    };
    let name = node_text(node, &parsed.src);
    let matches = symbol_matches(parsed.tree.root_node(), &name, &parsed.src, nl);
    let search = Search {
        kind: SearchKind::Symbol,
        pattern: name,
        matches,
        index: 0,
    };
    // strict inequality: the occurrence the cursor is already on doesn't count
    // as "next"
    jump_search(app, search, forward, false);
}

/// `n` / `N` — cycle through the currently active search's matches, wrapping.
fn cycle_search(app: &mut App, dir: isize) {
    let Some(search) = app.search.as_mut() else {
        return;
    };
    if search.matches.is_empty() {
        return;
    }
    search.index = cycle_index(search.index, search.matches.len(), dir);
    let (line, col, _) = search.matches[search.index];
    app.cursor = Cursor { line, col };
    app.scroll = follow_scroll(line, app.scroll, app.code_height);
    app.hscroll = follow_hscroll(col, app.hscroll, app.code_width);
}

/// `Enter` on the `/` prompt: run the text search and jump to the first match
/// at or after the cursor.
fn accept_search(app: &mut App) {
    let Some(prompt) = app.prompt.take() else {
        return;
    };
    let path = app.items[app.sel].path.clone();
    let matches = match app.sources.get(&path) {
        Some((_, nl)) => text_matches(nl, &prompt.text),
        None => vec![],
    };
    let search = Search {
        kind: SearchKind::Text,
        pattern: prompt.text,
        matches,
        index: 0,
    };
    jump_search(app, search, true, true);
}

/// `Esc` on the `/` prompt: cancel, restoring the cursor position from before
/// the prompt opened.
fn cancel_search(app: &mut App) {
    if let Some(prompt) = app.prompt.take() {
        app.cursor = prompt.anchor;
        app.scroll = follow_scroll(app.cursor.line, app.scroll, app.code_height);
        app.hscroll = follow_hscroll(app.cursor.col, app.hscroll, app.code_width);
    }
}

// ----------------------------------------------------------------------- edit

/// `$VISUAL` then `$EDITOR`, whitespace-split so trailing arguments (`code
/// --wait`, `emacsclient -nw`) survive; `vi` when neither is set or both are
/// blank — a POSIX-guaranteed binary rather than a guess.
fn resolve_editor() -> Vec<String> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());
    split_command(&raw)
}

fn split_command(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// The location arguments for one program, keyed by its basename (not the
/// full `$EDITOR` string, which may carry flags ahead of the program). An
/// editor outside this curated set gets just the file — passing a guessed
/// `+LINE` or `file:LINE` syntax to something that doesn't understand it risks
/// it being read as a second filename.
fn editor_args(basename: &str, path: &str, line: usize) -> Vec<String> {
    match basename {
        "vim" | "nvim" | "vi" | "nano" | "emacs" | "kak" => {
            vec![format!("+{line}"), path.to_string()]
        }
        "hx" => vec![format!("{path}:{line}")],
        "code" | "codium" => vec!["-g".to_string(), format!("{path}:{line}")],
        _ => vec![path.to_string()],
    }
}

/// The full `(program, args)` invocation for `spec` (as returned by
/// `resolve_editor`/`split_command`) opening `path` at 1-based `line`. Any
/// arguments already in `spec` (e.g. `--wait`) are kept ahead of the location
/// arguments. None only for an empty `spec`.
fn build_command(spec: &[String], path: &str, line: usize) -> Option<(String, Vec<String>)> {
    let (program, extra) = spec.split_first()?;
    let basename = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program.as_str());
    let mut args = extra.to_vec();
    args.extend(editor_args(basename, path, line));
    Some((program.clone(), args))
}

/// The selected hunk's `(path, 1-based line, approximate)` to open in the
/// editor. For `zz` the worktree file IS the new content, so the line is
/// exact. For a historical commit or range the worktree may have moved on
/// since — cheap to check by comparing that revision's blob for the path
/// against the file on disk; a mismatch is reported as approximate rather
/// than silently sending the user to an unrelated line.
fn edit_target(app: &App) -> (String, usize, bool) {
    let it = &app.items[app.sel];
    let line = it.new_range[0].max(1);
    if app.uncommitted {
        return (it.path.clone(), line, false);
    }
    let approx = match &app.review_sha {
        Some(sha) => {
            let blob = git(&["show", &format!("{sha}:{}", it.path)]);
            let disk = std::fs::read_to_string(&it.path).unwrap_or_default();
            blob != disk
        }
        None => true, // no revision resolved at all — can't vouch for the line
    };
    (it.path.clone(), line, approx)
}

// --------------------------------------------------------------------- tui loop

/// Leaves the alternate screen, restores the terminal, runs `program args`
/// with inherited stdio, then re-enters and forces a full redraw — the shared
/// body behind `ge`'s editor handoff and `:quickfix`'s vim handoff, so there
/// is exactly one terminal save/restore path.
fn run_suspended(
    terminal: &mut ratatui::DefaultTerminal,
    program: &str,
    args: &[String],
) -> std::io::Result<std::process::ExitStatus> {
    ratatui::restore();
    let outcome = Command::new(program).args(args).status();
    *terminal = ratatui::init();
    let _ = terminal.clear(); // the screen underneath may have changed; force a full redraw
    outcome
}

/// `ge` / `C-o` — hand the terminal to `$VISUAL`/`$EDITOR` for the selected
/// hunk's file and line, then take it back. A missing or misbehaving editor
/// must never leave the terminal broken, so failures are reported in the
/// existing popup instead of propagated. The popup also carries the
/// exact/approximate line note from `edit_target`, so the user is never
/// silently sent to a line that may not be right.
fn open_editor(app: &mut App, terminal: &mut ratatui::DefaultTerminal) {
    let (path, line, approx) = edit_target(app);
    let spec = resolve_editor();
    let Some((program, args)) = build_command(&spec, &path, line) else {
        app.popup = Some(Popup::new(
            "edit",
            vec![prose("no editor command to run ($VISUAL/$EDITOR)")],
        ));
        return;
    };
    let outcome = run_suspended(terminal, &program, &args);

    let note = if approx {
        " (approximate — this file has changed since the reviewed revision)"
    } else {
        ""
    };
    let msg = match outcome {
        Ok(status) if status.success() => format!("opened {path}:{line}{note}"),
        Ok(status) => format!("{program} exited with {status}{note}"),
        Err(e) => format!("failed to launch '{program}': {e}"),
    };
    app.popup = Some(Popup::new("edit", vec![prose(msg)]));
}

// ------------------------------------------------------------------ quickfix

/// One `setqflist()` item — the pure-data slice of an `Item`/its reviewed
/// flag that `quickfix_script` renders. Filled in from the reading-order
/// pane's current `view` order, so exporting is exactly "what the user sees".
struct QfHunk {
    filename: String,
    lnum: usize,
    /// `Some('W')`/`Some('I')` for the sign column; `None` renders as `''`
    /// (no sign) — see `qf_kind`.
    kind: Option<char>,
    /// the hunk's cluster label (`"cluster N"`), prefixed onto the item's
    /// text. NOT emitted as vim's `module` key: vim renders `module` *instead
    /// of* the filename in the quickfix window, which would cost a reviewer
    /// the one column they navigate by.
    cluster: Option<String>,
    text: String,
}

/// `:quickfix`'s per-hunk `type`: `'W'` when the hunk carries a warn-level
/// rule hit or an advisory (warn wins when both apply), `'I'` when it's
/// marked reviewed, otherwise no sign at all.
fn qf_kind(warn: bool, reviewed: bool) -> Option<char> {
    if warn {
        Some('W')
    } else if reviewed {
        Some('I')
    } else {
        None
    }
}

/// Vim single-quoted string literal escaping: doubles every embedded `'` (the
/// only escape a single-quoted vim string recognises) and folds out any
/// literal newline, which would otherwise split the `-S` script mid-statement
/// — vim's `\n` escape only exists inside double-quoted strings.
fn vim_single_quote(s: &str) -> String {
    s.replace('\'', "''").replace(['\n', '\r'], " ")
}

/// Builds the one `setqflist()` call `:quickfix` writes to `quickfix.vim` —
/// a pure function of the already-filtered, already-ordered hunk list, so
/// it's cheap to test directly without touching git or a real editor.
/// `'nr': '$'` pushes a new list onto vim's quickfix stack instead of
/// clobbering whatever the user already had open (`:colder` gets it back).
fn quickfix_script(rev: &str, strategy: &str, hunks: &[QfHunk]) -> String {
    let mut clusters: Vec<&str> = hunks.iter().filter_map(|h| h.cluster.as_deref()).collect();
    clusters.sort_unstable();
    clusters.dedup();
    let n_clusters = clusters.len().max(1);
    let title = format!(
        "ordo: {rev} — {} hunk{}, {n_clusters} cluster{}",
        hunks.len(),
        plural(hunks.len()),
        plural(n_clusters),
    );
    let mut out = String::new();
    out.push_str("call setqflist([], ' ', {\n");
    out.push_str("  \\ 'nr': '$',\n");
    let _ = writeln!(out, "  \\ 'title': '{}',", vim_single_quote(&title));
    let _ = writeln!(
        out,
        "  \\ 'context': {{'rev': '{}', 'strategy': '{}'}},",
        vim_single_quote(rev),
        vim_single_quote(strategy),
    );
    out.push_str("  \\ 'items': [\n");
    for h in hunks {
        let mut fields = vec![
            format!("'filename': '{}'", vim_single_quote(&h.filename)),
            format!("'lnum': {}", h.lnum),
            "'col': 1".to_string(),
            format!("'type': '{}'", h.kind.map(String::from).unwrap_or_default()),
        ];
        let text = match &h.cluster {
            Some(c) => format!("[{c}] {}", h.text),
            None => h.text.clone(),
        };
        fields.push(format!("'text': '{}'", vim_single_quote(&text)));
        let _ = writeln!(out, "  \\   {{{}}},", fields.join(", "));
    }
    out.push_str("  \\ ]})\n");
    out
}

/// Whether `spec` (as returned by `resolve_editor`) is vim, neovim, or a
/// close variant thereof — the ones `:quickfix`'s `-S <script> -c copen -c
/// "silent! cfirst"` invocation works with. Judged by argv[0]'s basename, so
/// `/usr/bin/nvim` matches just as `nvim` does; an unrelated editor (`code`,
/// `emacs`) does not.
fn is_vim_family(spec: &[String]) -> bool {
    let Some(program) = spec.first() else {
        return false;
    };
    let basename = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program.as_str());
    matches!(basename, "vim" | "nvim" | "vi" | "gvim" | "mvim")
}

/// `:quickfix`'s vim-family invocation: `-S` *sources* the generated script
/// (`-q` would instead parse it as an errorfile through 'errorformat', which
/// can't run vim commands), `copen` shows the resulting list, and `silent!
/// cfirst` selects its first entry — `silent!` because an empty export makes
/// bare `cfirst` raise `E42: No Errors` and strand the user at a
/// press-enter prompt.
fn quickfix_command(spec: &[String], path: &Path) -> Option<(String, Vec<String>)> {
    let (program, extra) = spec.split_first()?;
    let mut args = extra.to_vec();
    args.push("-S".to_string());
    args.push(path.display().to_string());
    args.push("-c".to_string());
    args.push("copen".to_string());
    args.push("-c".to_string());
    args.push("silent! cfirst".to_string());
    Some((program.clone(), args))
}

/// Resolves and writes `<git-dir>/ordo/quickfix.vim`, creating the `ordo/`
/// directory first. Inside the git dir so the file is never tracked and
/// needs no gitignore entry. `Err` names whichever step failed — git dir
/// resolution, directory creation, or the write — for `:quickfix`'s error
/// popup; never a panic and never a silent no-op.
fn write_quickfix_script(script: &str) -> Result<PathBuf, String> {
    let git_dir = git(&["rev-parse", "--git-dir"]);
    let git_dir = git_dir.trim();
    if git_dir.is_empty() {
        return Err(
            "could not resolve the git directory (`git rev-parse --git-dir` failed)".to_string(),
        );
    }
    let dir = PathBuf::from(git_dir).join("ordo");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join("quickfix.vim");
    std::fs::write(&path, script)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// `:quickfix`'s vim-family handoff — same suspend/run/redraw path as `ge`,
/// reporting a failed launch in the same popup idiom rather than a panic or a
/// silent no-op.
fn open_quickfix_editor(app: &mut App, terminal: &mut ratatui::DefaultTerminal, path: &Path) {
    let spec = resolve_editor();
    let Some((program, args)) = quickfix_command(&spec, path) else {
        app.popup = Some(Popup::new(
            "quickfix",
            vec![prose("no editor command to run ($VISUAL/$EDITOR)")],
        ));
        return;
    };
    let msg = match run_suspended(terminal, &program, &args) {
        Ok(status) if status.success() => format!("opened {} in {program}", path.display()),
        Ok(status) => format!("{program} exited with {status}"),
        Err(e) => format!("failed to launch '{program}': {e}"),
    };
    app.popup = Some(Popup::new("quickfix", vec![prose(msg)]));
}

/// While loading: just a status line under the pane border, same idiom as
/// the review panes. Nothing else can be drawn yet — no items, no sources.
fn draw_loading(f: &mut Frame, rev: &str, status: &str) {
    let area = f.area();
    let block = Block::bordered().title(format!(" {rev} — loading… "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let p = Paragraph::new(Text::from(vec![Line::from(status.to_string())]));
    f.render_widget(p, inner);
}

enum State {
    Loading(String),
    Ready(Box<App>),
}

/// Take the screen first, then load off the draw loop: a background thread
/// runs `gather`/`highlight_file`/`ordo::run` and reports progress over
/// `LoadMsg`, while this loop keeps drawing and polling input so `q`/`Esc`
/// (or `C-q`/`Esc`) abort a slow load without waiting for the worker. A
/// custom panic hook restores the terminal first even if the worker (or a
/// later draw) panics, so a broken terminal never outlives the crash.
#[allow(clippy::too_many_arguments)]
fn run(
    rev: String,
    keys: Keymap,
    target: Target,
    filter: Filter,
    only_comments: bool,
    review_sha: Option<String>,
    uncommitted: bool,
    theme: Theme,
    rules: Vec<ordo::model::Rule>,
    rules_report: Vec<String>,
) -> std::io::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        prev_hook(info);
    }));

    let mut terminal = ratatui::init();
    // kept around (rather than consumed by the first load) so `:e` can spawn
    // a fresh worker later without re-parsing CLI globs — the launch-time
    // path filter carries across a reload, same as the keymap and theme
    let base_filter = filter.clone();
    let (tx, rx) = mpsc::channel();
    let mut rx = rx;
    let worker_rev = rev.clone();
    let worker_rules = rules.clone();
    thread::spawn(move || {
        load(
            target,
            filter,
            only_comments,
            worker_rev,
            theme.syn,
            worker_rules,
            tx,
        )
    });

    let mut rev = rev;
    let mut review_sha = review_sha;
    let mut uncommitted = uncommitted;
    let mut state = State::Loading("starting…".to_string());
    let mut keys = Some(keys);
    let mut theme = theme;
    let mut timing: Option<String> = None;
    let mut post_msg: Option<String> = None;
    // Nothing on screen changes on its own once the review is up, so a frame is
    // only worth painting after something moved: a worker message, or an event.
    // Without this the 50ms poll below doubles as a 20fps repaint of a review
    // nobody is touching.
    let mut dirty = true;
    let result: std::io::Result<()> = 'outer: loop {
        loop {
            match rx.try_recv() {
                Ok(LoadMsg::Progress(s)) => {
                    dirty = true;
                    if let State::Loading(status) = &mut state {
                        *status = s;
                    }
                }
                Ok(LoadMsg::Empty(msg)) => {
                    post_msg = Some(msg);
                    break 'outer Ok(());
                }
                Ok(LoadMsg::Done(r)) => {
                    dirty = true;
                    let LoadResult {
                        items,
                        view,
                        comments_only,
                        sources,
                        highlights,
                        timing: t,
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
                    } = *r;
                    let sel0 = view[0];
                    let scroll = auto_scroll(&items[sel0]);
                    let cursor = cursor_for(&items[sel0], &sources);
                    timing = Some(t);
                    let mut fresh = App {
                        reviewed,
                        items,
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
                        keys: keys
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
                        review_sha: review_sha.clone(),
                        uncommitted,
                        history_cache: HashMap::new(),
                        jumps: Vec::new(),
                        rev: rev.clone(),
                        marks_path,
                        marks,
                        theme,
                        show_groups: false,
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
                        rules: rules.clone(),
                        strategy: "comprehension".to_string(),
                        rules_report: rules_report.clone(),
                        max_col: HashMap::new(),
                    };
                    // the list is a list of *symbols* by default; `:mode`
                    // switches it back to hunks
                    let led = fresh.symbol_ledger.clone();
                    set_mode(&mut fresh, ViewMode::Ledger, &led);
                    state = State::Ready(Box::new(fresh));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // the worker dropped its sender without a Done/Empty —
                    // only possible if it panicked; abort rather than spin
                    if matches!(state, State::Loading(_)) {
                        post_msg = Some("ordo: loading failed unexpectedly".to_string());
                        break 'outer Ok(());
                    }
                    break;
                }
            }
        }

        if dirty {
            if let Err(e) = terminal.draw(|f| match &mut state {
                State::Loading(status) => draw_loading(f, &rev, status),
                State::Ready(app) => draw(f, app, &rev),
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
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match &mut state {
                State::Loading(_) => {
                    let key = norm(k.code, k.modifiers);
                    if let Resolve::Act(Action::Quit) = keys
                        .as_ref()
                        .expect("keys not yet taken while Loading")
                        .resolve(None, key)
                    {
                        break 'outer Ok(());
                    }
                }
                State::Ready(app) => {
                    // input mode: every key edits the search prompt instead of
                    // running an action, so chords (C-w C-w, gg) can't leak in
                    // while it's open
                    if app.prompt.is_some() {
                        match (k.code, k.modifiers) {
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
                        continue;
                    }
                    // same interception scheme as the search prompt above,
                    // for the `:` command bar — every key drives it instead
                    // of resolving through the keymap
                    if app.command.is_some() {
                        match handle_command_key(app, k.code, k.modifiers) {
                            CommandOutcome::Quit => break 'outer Ok(()),
                            CommandOutcome::None => {}
                            CommandOutcome::OpenQuickfix(path) => {
                                open_quickfix_editor(app, &mut terminal, &path);
                            }
                            CommandOutcome::Reload(target, new_rev) => {
                                // `:e`: everything indexing the *old* review —
                                // selection, cursor, scroll, search, the jump
                                // stack, any open popup — is dropped by simply
                                // not carrying `app` forward into the new
                                // `App` the next `Done` builds (same as a
                                // fresh launch); the reviewed marks come back
                                // from the on-disk cache keyed by the new rev
                                // (see `mark_key`/`load`), not from `app.marks`.
                                // The keymap and theme are carried forward —
                                // display preferences independent of which
                                // hunks are loaded — and the path filter is
                                // re-read from `base_filter` rather than
                                // `app.path_filter`, so a live `:filter`
                                // resets along with everything else.
                                let (carried_keys, carried_theme) = carry_across_reload(app);
                                review_sha = review_commit_sha(&target);
                                uncommitted = matches!(
                                    target,
                                    Target::Uncommitted | Target::WorktreeRange(_)
                                );
                                rev = new_rev.clone();
                                keys = Some(carried_keys);
                                theme = carried_theme;
                                state = State::Loading(format!("switching to {new_rev}…"));
                                let (new_tx, new_rx) = mpsc::channel();
                                rx = new_rx;
                                let filt = base_filter.clone();
                                let reload_rules = rules.clone();
                                thread::spawn(move || {
                                    load(
                                        target,
                                        filt,
                                        only_comments,
                                        new_rev,
                                        theme.syn,
                                        reload_rules,
                                        new_tx,
                                    )
                                });
                            }
                        }
                        continue;
                    }
                    let key = norm(k.code, k.modifiers);
                    match app.keys.resolve(app.pending, key) {
                        Resolve::Pending => app.pending = Some(key),
                        Resolve::Miss => app.pending = None,
                        // handled here rather than in `apply`: it must leave the
                        // alternate screen for the editor and come back
                        Resolve::Act(Action::OpenEditor) => {
                            app.pending = None;
                            open_editor(app, &mut terminal);
                        }
                        Resolve::Act(a) => {
                            app.pending = None;
                            if apply(app, a) {
                                break 'outer Ok(());
                            }
                        }
                    }
                }
            },
            Ok(_) => {}
            Err(e) => break 'outer Err(e),
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

/// Run one action against the focused pane; true means quit.
fn apply(app: &mut App, a: Action) -> bool {
    // An open popup consumes input itself: Esc/q dismiss it (without
    // quitting), Next/Prev/PageUp/PageDown scroll its own body rather than
    // the pane underneath, and everything else is a no-op while it's up.
    if app.popup.is_some() {
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
        return false;
    }
    match a {
        // an active search is showing state too: clear its highlights first,
        // the way dismissing a popup does, so Esc after a search doesn't end
        // the review session
        Action::Quit if app.search.is_some() => app.search = None,
        Action::Quit => return true,
        Action::Focus(p) => app.focus = p,
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
            Pane::Why => app.why_scroll = 0,
        },
        Action::Last => match app.focus {
            Pane::List => select(app, last_visible(app)),
            Pane::Code => app.scroll = last_line(app.code_len),
            Pane::Why => app.why_scroll = last_line(app.why_len),
        },
        // column line-end motion: cursor in the code pane, list/why fall back
        // to `First`/`Last`'s meaning so vscode's Home/End still works there
        Action::LineStart => match app.focus {
            Pane::List => select(app, first_visible(app)),
            Pane::Code => cursor_move(app, line_start),
            Pane::Why => app.why_scroll = 0,
        },
        Action::LineEnd => match app.focus {
            Pane::List => select(app, last_visible(app)),
            Pane::Code => cursor_move(app, line_end),
            Pane::Why => app.why_scroll = last_line(app.why_len),
        },
        Action::CursorLeft if app.focus == Pane::Code => {
            cursor_move(app, |c, lines| move_col(c, lines, -1))
        }
        Action::CursorRight if app.focus == Pane::Code => {
            cursor_move(app, |c, lines| move_col(c, lines, 1))
        }
        Action::WordNext if app.focus == Pane::Code => cursor_move(app, word_next),
        Action::WordPrev if app.focus == Pane::Code => cursor_move(app, word_prev),
        Action::WordEnd if app.focus == Pane::Code => cursor_move(app, word_end),
        Action::ParaPrev if app.focus == Pane::Code => cursor_move(app, para_prev),
        Action::ParaNext if app.focus == Pane::Code => cursor_move(app, para_next),
        Action::CursorLeft
        | Action::CursorRight
        | Action::WordNext
        | Action::WordPrev
        | Action::WordEnd
        | Action::ParaPrev
        | Action::ParaNext => {} // only meaningful with the code pane focused
        // `K`/`F12` dispatches on the focused pane rather than adding a
        // second key: the code pane's symbol hover and the why pane's dep
        // preview are the same "show me more about what's under the cursor"
        // gesture, just aimed at a different cursor.
        Action::Hover => match app.focus {
            Pane::Code => hover(app),
            Pane::Why => preview_edge(app),
            Pane::List => {}
        },
        Action::SearchOpen if app.focus == Pane::Code => {
            app.prompt = Some(Prompt {
                text: String::new(),
                anchor: app.cursor,
            });
        }
        Action::SymbolNext if app.focus == Pane::Code => symbol_search(app, true),
        Action::SymbolPrev if app.focus == Pane::Code => symbol_search(app, false),
        Action::SearchNext if app.focus == Pane::Code => cycle_search(app, 1),
        Action::SearchPrev if app.focus == Pane::Code => cycle_search(app, -1),
        Action::SearchOpen
        | Action::SymbolNext
        | Action::SymbolPrev
        | Action::SearchNext
        | Action::SearchPrev => {} // only meaningful with the code pane focused
        Action::Fold(how) => fold(app, how),
        Action::ScrollLeft if app.focus == Pane::Code => {
            app.hscroll = app.hscroll.saturating_sub(1);
        }
        Action::ScrollRight if app.focus == Pane::Code => {
            app.hscroll = app.hscroll.saturating_add(1);
        }
        Action::ScrollLeft | Action::ScrollRight => {} // only meaningful with the code pane focused
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
        Pane::Code => cursor_move(app, |c, lines| move_line(c, lines, by)),
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
        Pane::Why => app.why_scroll = scrolled(app.why_scroll, by, app.why_len),
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

// --------------------------------------------------------------- why pane

/// One line of the why pane's content, independent of rendering — shared by
/// `draw` and the edge actions (`preview_edge`/`jump_to_edge`) below so both
/// agree on what line `why_sel` is pointing at.
enum WhyKind {
    Text,
    /// a `dep` line; `Some(idx)` is its target hunk's index into `app.items`,
    /// `None` when the referenced hunk isn't part of this review
    Edge(Option<usize>),
}

struct WhyRow {
    text: String,
    style: Style,
    kind: WhyKind,
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
fn why_rows(
    it: &Item,
    view: &[usize],
    theme: &Theme,
    note: Option<&str>,
    out_of_order: &[String],
    delta: Option<&str>,
    cascade: Option<&str>,
) -> Vec<WhyRow> {
    let mut rows = vec![];
    if let Some(c) = cascade {
        rows.push(WhyRow {
            text: format!("· {c}"),
            style: Style::default().fg(theme.accent),
            kind: WhyKind::Text,
        });
    }
    if let Some(d) = delta {
        rows.push(WhyRow {
            text: format!("· {d}"),
            style: Style::default().fg(theme.accent),
            kind: WhyKind::Text,
        });
    }
    // approving a call before its callee is the one review-order mistake the
    // graph can actually prove
    if !out_of_order.is_empty() {
        rows.push(WhyRow {
            text: format!(
                "⚠ marked reviewed, but depends on unreviewed {}",
                out_of_order.join(", ")
            ),
            style: Style::default().fg(theme.warn),
            kind: WhyKind::Text,
        });
    }
    // a review note leads: it is the reviewer's own words about this symbol,
    // and it outranks anything the engine derived
    if let Some(n) = note {
        rows.push(WhyRow {
            text: format!("note: {n}"),
            style: Style::default().fg(theme.warn),
            kind: WhyKind::Text,
        });
    }
    // The engine's terminal fallback rationale: it found nothing to say about
    // the hunk, so a "reason: change" line says nothing either — leave it out.
    if it.rationale != "change" {
        rows.push(WhyRow {
            text: format!("reason: {}", it.rationale),
            style: Style::default().fg(theme.border_focus),
            kind: WhyKind::Text,
        });
    }
    for d in &it.details {
        rows.push(WhyRow {
            text: format!("- {d}"),
            style: Style::default().fg(theme.accent),
            kind: WhyKind::Text,
        });
    }
    for r in &it.rules {
        let (color, tag) = match r.level {
            "warn" => (theme.warn, "⚠"),
            _ => (theme.reviewed, "·"),
        };
        rows.push(WhyRow {
            text: format!("{tag} {} — {}", r.rule, r.message),
            style: Style::default().fg(color),
            kind: WhyKind::Text,
        });
    }
    if !it.notes.is_empty() {
        rows.push(WhyRow {
            text: format!("notes: {}", it.notes.join("; ")),
            style: Style::default().fg(theme.mark),
            kind: WhyKind::Text,
        });
    }
    for e in &it.edges {
        let target = e.target.filter(|t| view.contains(t));
        rows.push(WhyRow {
            text: format!("dep {}", e.label),
            style: edge_style(target, theme),
            kind: WhyKind::Edge(target),
        });
    }
    for (construct, message, verdict) in &it.advisories {
        let (head, color) = if *verdict {
            (format!("⚠ {construct}"), theme.warn)
        } else {
            (construct.clone(), theme.category)
        };
        rows.push(WhyRow {
            text: head,
            style: Style::default().fg(color).add_modifier(Modifier::BOLD),
            kind: WhyKind::Text,
        });
        for ml in message.lines() {
            rows.push(WhyRow {
                text: format!("  {ml}"),
                style: Style::default().fg(theme.fg),
                kind: WhyKind::Text,
            });
        }
    }
    rows
}

// the dep-line target at `app.why_sel`, if the cursor is on one at all
fn edge_at_cursor(app: &App) -> Option<Option<usize>> {
    let rows = why_rows(
        &app.items[app.sel],
        &app.view,
        &app.theme,
        note_for(app, app.sel),
        &out_of_order_labels(app, app.sel),
        delta_line(app, app.sel),
        cascade_line(app, app.sel).as_deref(),
    );
    match rows.get(app.why_sel)?.kind {
        WhyKind::Edge(target) => Some(target),
        WhyKind::Text => None,
    }
}

/// `K`/`F12` while the why pane is focused: preview the dep line's target
/// hunk — its location, rationale, and a code excerpt — without leaving the
/// current hunk. No-op when `why_sel` isn't on a dep line. When the target
/// isn't part of this review, says so plainly rather than showing nothing.
/// A syntax-highlighted excerpt of `lines[n0..=n1]` (1-based, inclusive) with a
/// line-number gutter, for the dep preview. Falls back to unstyled text for a
/// file with no grammar, the same way the code pane does.
fn excerpt(
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

fn preview_edge(app: &mut App) {
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
fn jump_to_edge(app: &mut App) {
    let Some(Some(idx)) = edge_at_cursor(app) else {
        return;
    };
    stack_push(&mut app.jumps, (app.sel, app.cursor));
    select(app, idx);
    app.focus = Pane::Code;
}

/// `C-o` (vim) / `Alt-Left` (vscode): pop the position stack and return
/// there — skipping (not just the out-of-range entries `stack_pop_valid`
/// already drops, but also) any entry a live filter currently hides, the
/// same "not part of this review" treatment `jump_to_edge` gives it. A
/// no-op once the stack is exhausted.
fn jump_back(app: &mut App) {
    let (idx, cursor) = loop {
        match stack_pop_valid(&mut app.jumps, app.items.len()) {
            Some((idx, cursor)) if app.view.contains(&idx) => break (idx, cursor),
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
fn pane_block(title: String, focused: bool, theme: &Theme) -> Block<'static> {
    let border = if focused {
        theme.border_focus
    } else {
        theme.border
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            title,
            Style::default().fg(if focused {
                theme.border_focus
            } else {
                theme.dim
            }),
        ))
}

fn draw(f: &mut Frame, app: &mut App, rev: &str) {
    let cols = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(f.area());

    // left — reading order
    let symbol = "▶ ";
    let text_w = (cols[0].width as usize).saturating_sub(2 + symbol.chars().count());
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
                let spans = vec![Span::styled(
                    format!("· {reason}"),
                    Style::default()
                        .fg(app.theme.border_focus)
                        .add_modifier(Modifier::BOLD),
                )];
                ListItem::new(Line::from(slice_range(spans, 0, text_w)))
            }
            DisplayRow::Item(i) => {
                let i = *i;
                let it = &app.items[i];
                let style = if it.noise {
                    Style::default().fg(app.theme.dim)
                } else if app.reviewed[i] {
                    Style::default().fg(app.theme.reviewed)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                // head only — the full rationale lives in the "why" pane.
                // Path, line number and category are separate spans so each
                // reads at a glance; a noise or reviewed row overrides all
                // three, because *that* is what the row is saying.
                let tinted = it.noise || app.reviewed[i];
                let dim = |c: Color| if tinted { style } else { style.fg(c) };
                let spans = vec![
                    Span::styled(it.mark.clone(), dim(app.theme.mark)),
                    Span::styled(it.path.clone(), style),
                    Span::styled(format!(":L{}", it.new_range[0]), dim(app.theme.accent)),
                    Span::styled(format!(" [{}]", it.cat), dim(app.theme.category)),
                ];
                ListItem::new(Line::from(slice_range(spans, 0, text_w)))
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
                " {rev} — {done}/{total} reviewed{}{} · {} ",
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
    f.render_stateful_widget(list, cols[0], &mut state);

    // right — code (top) + why (bottom)
    let rhs =
        Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).split(cols[1]);
    let it = &app.items[app.sel];

    let code_w = (rhs[0].width as usize).saturating_sub(2);
    app.code_height = rhs[0].height.saturating_sub(2);
    app.code_width = code_w.saturating_sub(GUTTER_W).min(u16::MAX as usize) as u16;
    // clamp to the selected file's longest line so hscroll can't run away
    // past any content it could ever bring into view; cached per path since
    // it only changes on a load/`:e`, not every frame
    let sources = &app.sources;
    let max_col = *app.max_col.entry(it.path.clone()).or_insert_with(|| {
        sources
            .get(&it.path)
            .map(|(_, nl)| nl.iter().map(|l| l.chars().count()).max().unwrap_or(0))
            .unwrap_or(0)
    });
    app.hscroll = app.hscroll.min(max_col.min(u16::MAX as usize) as u16);
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
    );
    // `‹`/`›` mark content clipped off the left/right of the horizontal
    // window — truncation must never be silent, so this is always shown
    // rather than only surfaced by scrolling into it.
    let clip = match (app.hscroll > 0, right_clip) {
        (true, true) => " ‹›",
        (true, false) => " ‹",
        (false, true) => " ›",
        (false, false) => "",
    };
    // The `/` prompt and the active-search status both reuse the code pane's
    // border title rather than a separate widget — one line is enough for
    // either, and it keeps the layout unchanged while typing or searching.
    let code_title = if let Some(p) = &app.prompt {
        format!(" search: {}▏ ", p.text)
    } else if let Some(s) = &app.search {
        let glyph = match s.kind {
            SearchKind::Text => '/',
            SearchKind::Symbol => '*',
        };
        let status = if s.matches.is_empty() {
            format!("no matches for {glyph}{}", s.pattern)
        } else {
            format!("[{}/{}] {glyph}{}", s.index + 1, s.matches.len(), s.pattern)
        };
        format!(" {}{clip}  {status}  ({}) ", it.path, app.keys.hint)
    } else {
        format!(" {}{clip}  ({}) ", it.path, app.keys.hint)
    };

    let why_content = why_rows(
        it,
        &app.view,
        &app.theme,
        note_for(app, app.sel),
        &out_of_order_labels(app, app.sel),
        delta_line(app, app.sel),
        cascade_line(app, app.sel).as_deref(),
    );
    // `why` wraps, so this counts logical lines — enough to keep the scroll in range
    app.code_len = code_total;
    app.why_len = why_content.len();
    app.why_height = rhs[1].height.saturating_sub(2);
    app.scroll = app.scroll.min(last_line(app.code_len));
    app.why_scroll = app.why_scroll.min(last_line(app.why_len));
    app.why_sel = app.why_sel.min(last_line(app.why_len) as usize);
    // the current line takes the list's selection tint, not REVERSED: this pane
    // is prose, and swapping fg/bg on a whole wrapped paragraph reads as a
    // block of colour rather than as "you are here". The code pane's cursor
    // stays reversed — one cell, where the swap is exactly right.
    let why: Vec<Line> = why_content
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

    // `code` is already the slice starting at `app.scroll`, so the paragraph
    // renders it from the top rather than scrolling within it
    let code_view = Paragraph::new(Text::from(code))
        .block(pane_block(code_title, app.focus == Pane::Code, &app.theme))
        .scroll((0, 0));
    f.render_widget(code_view, rhs[0]);

    let info = Paragraph::new(Text::from(why))
        .block(pane_block(
            " why ".to_string(),
            app.focus == Pane::Why,
            &app.theme,
        ))
        .scroll((app.why_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(info, rhs[1]);

    if let Some(popup) = app.popup.as_mut() {
        popup.scroll = popup.scroll.min(last_line(popup.lines.len()));
        popup.hscroll = popup.hscroll.min(last_line(popup_width(&popup.lines)));
    }
    if let Some(popup) = &app.popup {
        let rect = popup_rect(rhs[0], popup.lines.len());
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
        let block = Block::bordered()
            .title(format!(
                " {}{} ",
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

    if let Some(bar) = &app.command {
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

        if !bar.candidates.is_empty() {
            let menu_rect = command_menu_rect(bar_rect, area, bar.candidates.len());
            f.render_widget(Clear, menu_rect);
            // completing the command itself: each row carries the command's
            // help sentence, since there is no central help to look it up in.
            // Completing an argument: just the candidates.
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
    }
}

/// One row of the completion menu. While the command *name* is being
/// completed the row is `name <args>` padded to a common column, then the
/// command's help sentence, cut to what fits; an alias or an argument
/// candidate has no sentence and is shown as is.
fn command_menu_row(
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

// -------------------------------------------------------------- command mode

/// One `:` command — name, a short argument hint for `:help` (empty when it
/// takes none), and the help line itself.
struct Cmd {
    name: &'static str,
    args: &'static str,
    help: &'static str,
}

/// The command table — the single source of truth for command names,
/// `:help`'s listing, and command-name completion. Adding a command here is
/// the only step that makes it discoverable; `execute_command`'s `match`
/// still has to know what to *do* with it, but a name present here with no
/// matching arm there falls into that match's `unknown command` case rather
/// than silently doing nothing.
const COMMANDS: &[Cmd] = &[
    Cmd {
        name: "only-comments",
        args: "",
        help: "toggle showing only comment/docstring hunks",
    },
    Cmd {
        name: "all",
        args: "",
        help: "toggle showing generated/formatting-noise hunks",
    },
    Cmd {
        name: "rules",
        args: "",
        help: "where the active rules came from, and what was replaced or disabled",
    },
    Cmd {
        name: "filter",
        args: "<glob>",
        help: "narrow the review to paths matching <glob>; no argument clears it",
    },
    Cmd {
        name: "keys",
        args: "<preset>",
        help: "swap the keymap live (vim, vscode)",
    },
    Cmd {
        name: "theme",
        args: "<name>",
        help: "swap the palette live (:theme with no name lists them)",
    },
    Cmd {
        name: "strategy",
        args: "<name>",
        help: "re-order the review (comprehension, defs-first, file)",
    },
    Cmd {
        name: "group",
        args: "",
        help: "toggle group-reason headers in the reading-order list",
    },
    Cmd {
        name: "rule",
        args: "",
        help: "draft a rule matching the selected hunk's shape",
    },
    Cmd {
        name: "delta",
        args: "",
        help: "what changed since this review was last opened",
    },
    Cmd {
        name: "note",
        args: "[text]",
        help: "anchor a note to the selected hunk's symbol; no text clears it",
    },
    Cmd {
        name: "mode",
        args: "[ledger|hunks]",
        help: "list by symbol (default) or by hunk; no argument toggles",
    },
    Cmd {
        name: "goto",
        args: "<path>",
        help: "select the first hunk of <path>, focus the code pane",
    },
    Cmd {
        name: "e",
        args: "<rev>",
        help: "review a different revision, without restarting",
    },
    Cmd {
        name: "audit",
        args: "",
        help: "account for every hunk and file not on screen, and why",
    },
    Cmd {
        name: "quickfix",
        args: "",
        help:
            "export the reading order to a vim quickfix list and open it (aliases: :qf, :vim-qfl)",
    },
    Cmd {
        name: "qf",
        args: "",
        help: "alias for :quickfix",
    },
    Cmd {
        name: "vim-qfl",
        args: "",
        help: "alias for :quickfix",
    },
    Cmd {
        name: "q",
        args: "",
        help: "quit",
    },
    Cmd {
        name: "help",
        args: "",
        help: "list these commands",
    },
];

/// `:audit`'s body — every hunk and file that isn't on screen, charged to the
/// thing that removed it. The point is the last line: if the reasons don't add
/// up to what's missing, it says so instead of implying the review is complete.
fn build_audit(
    items: &[Item],
    view_len: usize,
    hidden: &Hidden,
    ledger: &Ledger,
    path_filter: Option<&str>,
) -> Vec<String> {
    let mut out = vec![
        format!("{view_len} of {} hunks shown", items.len()),
        String::new(),
        "hidden in the view".to_string(),
    ];
    let row = |n: usize, what: &str| format!("  {n:>4}  {what}");
    out.push(row(
        hidden.noise,
        "generated/formatting noise (:all shows them)",
    ));
    out.push(row(
        ledger.hunks_import,
        "of which import hunks — noise, but shown by default where the diff put them",
    ));
    out.push(row(hidden.comment, "not a comment change (:only-comments)"));
    out.push(row(
        hidden.glob,
        &match path_filter {
            Some(g) => format!("outside the path filter '{g}'"),
            None => "outside the path filter".to_string(),
        },
    ));

    out.push(String::new());
    out.push("dropped by the engine before ordering".to_string());
    out.push(row(
        ledger.hunks_non_comment,
        "not a comment change (--only-comments)",
    ));

    out.push(String::new());
    out.push(format!("files: {} changed, of which", ledger.files_seen));
    out.push(row(
        ledger.files_generated,
        "generated or lock file (--all keeps them)",
    ));
    out.push(row(
        ledger.files_declared,
        "declared generated by .gitattributes",
    ));
    out.push(row(
        ledger.files_globbed,
        "never fetched: excluded by a launch-time glob",
    ));
    out.push(row(
        ledger.files_unreadable,
        "listed as changed but unreadable",
    ));

    out.push(String::new());
    out.push(if hidden.unaccounted == 0 {
        "every hidden hunk is accounted for".to_string()
    } else {
        format!(
            "{} hidden hunk{} unaccounted for — this is a bug, please report it",
            hidden.unaccounted,
            plural(hidden.unaccounted)
        )
    });
    out
}

fn command_names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

/// `:help`'s body, generated straight from `COMMANDS` — same discipline as
/// `build_help` for keybindings, so the listing can't name a command that
/// doesn't exist or omit one that does.
fn build_command_help() -> Vec<String> {
    COMMANDS
        .iter()
        .map(|c| {
            let head = if c.args.is_empty() {
                format!(":{}", c.name)
            } else {
                format!(":{} {}", c.name, c.args)
            };
            format!("  {head:<20} {}", c.help)
        })
        .collect()
}

/// Splits a typed command-bar line (without its leading `:`) into the command
/// name and its argument, trimming both — `"filter   src/*  "` ->
/// `("filter", "src/*")`. A bare name or an empty line gets an empty argument.
fn parse_command_line(line: &str) -> (&str, &str) {
    let line = line.trim();
    match line.split_once(char::is_whitespace) {
        Some((name, rest)) => (name, rest.trim()),
        None => (line, ""),
    }
}

/// The completion matcher: prefix match first (case-insensitive), substring
/// as a fallback when nothing prefixes. Empty input lists every candidate
/// unfiltered; no match at all — of either kind — yields an empty menu
/// rather than a stale or wrong one.
fn complete(input: &str, candidates: &[String]) -> Vec<String> {
    if input.is_empty() {
        return candidates.to_vec();
    }
    let needle = input.to_lowercase();
    let prefix: Vec<String> = candidates
        .iter()
        .filter(|c| c.to_lowercase().starts_with(&needle))
        .cloned()
        .collect();
    if !prefix.is_empty() {
        return prefix;
    }
    candidates
        .iter()
        .filter(|c| c.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// The argument candidates for one command name — empty for a command that
/// takes none, which is how `command_completions` knows not to show a menu.
/// `goto_paths`/`filter_dirs`/`rev_candidates` are handed in rather than
/// computed here so this stays a pure function of plain data (see
/// `recompute_candidates` for where they come from).
fn arg_candidates(
    cmd: &str,
    goto_paths: &[String],
    filter_dirs: &[String],
    rev_candidates: &[String],
) -> Vec<String> {
    match cmd {
        "keys" => vec!["vim".to_string(), "vscode".to_string()],
        "theme" => theme_names(),
        "strategy" => vec![
            "comprehension".to_string(),
            "defs-first".to_string(),
            "file".to_string(),
        ],
        "goto" => goto_paths.to_vec(),
        "filter" => filter_dirs.to_vec(),
        "e" => rev_candidates.to_vec(),
        _ => vec![],
    }
}

/// The completion menu for the command bar's current text: command names
/// while the first word is still being typed, that command's own argument
/// candidates once a space follows it.
fn command_completions(
    text: &str,
    goto_paths: &[String],
    filter_dirs: &[String],
    rev_candidates: &[String],
) -> Vec<String> {
    match text.find(char::is_whitespace) {
        None => complete(text, &command_names()),
        Some(pos) => {
            let name = &text[..pos];
            let arg = text[pos..].trim_start();
            let pool = arg_candidates(name, goto_paths, filter_dirs, rev_candidates);
            if pool.is_empty() {
                vec![]
            } else {
                complete(arg, &pool)
            }
        }
    }
}

/// Splice an accepted completion into the command line, replacing whichever
/// word is being completed and leaving a trailing space so the next word (an
/// argument, once the command name itself was just completed) can start
/// right away.
fn apply_completion(text: &str, candidate: &str) -> String {
    match text.find(char::is_whitespace) {
        None => format!("{candidate} "),
        Some(pos) => format!("{}{candidate} ", &text[..=pos]),
    }
}

fn distinct_sorted<'a>(vals: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut v: Vec<String> = vals.map(str::to_string).collect();
    v.sort();
    v.dedup();
    v
}

fn dir_prefix(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

fn open_command_bar(app: &mut App, text: String) {
    app.command = Some(CommandBar {
        text,
        candidates: vec![],
        selected: None,
        rev_cache: None,
    });
    recompute_candidates(app);
}

/// `:e`'s own argument pool: `zz` and `HEAD` plus every branch and tag, via
/// one `git for-each-ref` — shelled out here, lazily, only when
/// `recompute_candidates` is actually completing an `:e ` argument, never at
/// startup.
fn rev_completions() -> Vec<String> {
    let mut v = vec!["zz".to_string(), "HEAD".to_string()];
    let out = git(&["for-each-ref", "--format=%(refname:short)"]);
    v.extend(
        out.lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    );
    v
}

/// Recomputes the open command bar's completion menu for its current text —
/// called after every edit (typing, backspace, accepting a completion).
fn recompute_candidates(app: &mut App) {
    let text = match &app.command {
        Some(bar) => bar.text.clone(),
        None => return,
    };
    // Only built for the command that actually consumes them — no point
    // sorting every path/dir on every keystroke for a command that ignores
    // them. `:goto` only offers currently visible paths (jumping to a hidden
    // one would strand `sel` outside `view`); `:filter` offers every loaded
    // path's directory, since narrowing is the point of typing one.
    let cmd_name = text.find(char::is_whitespace).map(|pos| &text[..pos]);
    let goto_paths = if cmd_name == Some("goto") {
        distinct_sorted(app.view.iter().map(|&i| app.items[i].path.as_str()))
    } else {
        vec![]
    };
    let filter_dirs = if cmd_name == Some("filter") {
        distinct_sorted(app.items.iter().filter_map(|it| dir_prefix(&it.path)))
    } else {
        vec![]
    };
    // `:e`'s ref pool is fetched at most once per opened bar (see
    // `CommandBar::rev_cache`) rather than shelling out to `git for-each-ref`
    // on every keystroke.
    let rev_candidates = if cmd_name == Some("e") {
        let bar = app.command.as_mut().expect("checked above");
        if bar.rev_cache.is_none() {
            bar.rev_cache = Some(rev_completions());
        }
        bar.rev_cache.clone().expect("just set")
    } else {
        vec![]
    };
    let candidates = command_completions(&text, &goto_paths, &filter_dirs, &rev_candidates);
    if let Some(bar) = app.command.as_mut() {
        bar.candidates = candidates;
        bar.selected = None;
    }
}

fn cycle_candidate(app: &mut App, dir: isize) {
    let Some(bar) = app.command.as_mut() else {
        return;
    };
    if bar.candidates.is_empty() {
        return;
    }
    let len = bar.candidates.len();
    bar.selected = Some(match bar.selected {
        None if dir >= 0 => 0,
        None => len - 1,
        Some(i) => cycle_index(i, len, dir),
    });
}

/// `:e <rev>`'s carry-forward: the keymap and theme, both display
/// preferences independent of which revision is loaded. Everything else a
/// fresh `App` would otherwise not have is deliberately left behind (see
/// `run`'s `CommandOutcome::Reload` handling, which this feeds): selection,
/// cursor, scroll, search, the jump stack, any open popup, and the
/// comment/noise/path filters all reset to the same defaults a plain launch
/// starts with, because the new revision's `App` is built by the exact same
/// `Ok(LoadMsg::Done)` path a first load takes — there's no second
/// construction site to keep in sync. The reviewed marks aren't "reset" so
/// much as recomputed: they're keyed by `(rev, ...)` (see `mark_key`), so the
/// new rev's marks come back from the on-disk cache on their own, without
/// referencing the old session's `app.marks` at all.
fn carry_across_reload(app: &App) -> (Keymap, Theme) {
    (app.keys.clone(), app.theme)
}

/// What running a command line does to the outer session — most just mutate
/// `app` in place and report `None`; `:q` quits, and `:e` needs to leave
/// `State::Ready` entirely and go back through the progressive loader, which
/// `execute_command`/`accept_command`/`handle_command_key` can't do on their
/// own (they only ever see `&mut App`) — so it's handed back to `run` as data.
enum CommandOutcome {
    None,
    Quit,
    /// `:e <rev>` resolved: the resolved target and the rev string as typed
    Reload(Target, String),
    /// `:quickfix`/`:qf`/`:vim-qfl` wrote the script and the resolved editor
    /// is vim-family: hand the terminal to it (needs `&mut
    /// ratatui::DefaultTerminal`, which `execute_command` never sees)
    OpenQuickfix(PathBuf),
}

/// `Enter` on the command bar: with a candidate highlighted, splice it into
/// the line (doesn't run anything yet — a second `Enter` does); with nothing
/// highlighted, run the line and close the bar.
fn accept_command(app: &mut App) -> CommandOutcome {
    let Some(bar) = app.command.as_ref() else {
        return CommandOutcome::None;
    };
    if let Some(i) = bar.selected {
        let candidate = bar.candidates[i].clone();
        let new_text = apply_completion(&bar.text, &candidate);
        if let Some(bar) = app.command.as_mut() {
            bar.text = new_text;
        }
        recompute_candidates(app);
        return CommandOutcome::None;
    }
    let line = bar.text.clone();
    app.command = None;
    match execute_command(app, &line) {
        Ok(outcome) => outcome,
        Err(msg) => {
            app.popup = Some(Popup::new("command", vec![prose(msg)]));
            CommandOutcome::None
        }
    }
}

/// Command-bar key handling while `app.command` is open — every key edits or
/// drives the bar instead of resolving through the keymap, mirroring the `/`
/// prompt's interception.
fn handle_command_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> CommandOutcome {
    if app.command.is_none() {
        return CommandOutcome::None;
    }
    match (code, mods) {
        (KeyCode::Esc, _) => app.command = None,
        (KeyCode::Backspace, _) => {
            if let Some(bar) = app.command.as_mut() {
                bar.text.pop();
            }
            recompute_candidates(app);
        }
        (KeyCode::Tab, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
            cycle_candidate(app, 1);
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => cycle_candidate(app, -1),
        (KeyCode::Enter, _) => return accept_command(app),
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
            if let Some(bar) = app.command.as_mut() {
                bar.text.push(c);
            }
            recompute_candidates(app);
        }
        _ => {}
    }
    CommandOutcome::None
}

fn parse_strategy(s: &str) -> Option<Strategy> {
    match s {
        "comprehension" => Some(Strategy::Comprehension),
        "defs-first" => Some(Strategy::DefsFirst),
        "file" => Some(Strategy::File),
        _ => None,
    }
}

/// Rebuild the `Change` list `ordo::run` needs from `app.sources` — the full
/// old/new content per path, already in memory from the initial load, so
/// `:strategy` never re-reads git or re-fetches anything. Sorted by path for
/// determinism: `Sources` is a `HashMap`, so the launch's own file order
/// (which `Strategy::File` uses as its tiebreak) isn't preserved either way;
/// alphabetical is a stable, predictable substitute.
fn changes_from_sources(sources: &Sources) -> Vec<Change> {
    let mut paths: Vec<&String> = sources.keys().collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let (old, new) = &sources[path];
            Change {
                path: path.clone(),
                old: Some(old.join("\n")),
                new: Some(new.join("\n")),
                diff: None,
            }
        })
        .collect()
}

/// `:strategy <name>` — re-runs `ordo::run` synchronously on the main thread
/// rather than the worker-thread/`State::Loading` path `load` uses. The
/// content is already fully in memory (no git, and the only re-parsing is
/// the engine's own hunk analysis, which the task explicitly allows for this
/// one command); a synchronous call keeps every other piece of session state
/// (marks, keymap, scroll positions elsewhere) untouched, where reusing the
/// loading screen would mean tearing the whole `App` down and rebuilding it.
/// Builds the replacement items/view/reviewed into locals first and only
/// commits them to `app` once every step has succeeded, so a rejected
/// re-order (nothing to review, or nothing visible under the current
/// filters) never leaves `app.sel`/`app.view` pointing past a shrunk
/// `app.items`.
fn run_strategy(app: &mut App, name: &str) -> Result<(), String> {
    let strategy = parse_strategy(name).ok_or_else(|| {
        format!("unknown strategy '{name}' (want: comprehension, defs-first, file)")
    })?;
    let input = Input {
        changes: changes_from_sources(&app.sources),
        options: Options {
            strategy,
            cross_file: true,
            full_context: false,
            only_comments: false,
            rules: app.rules.clone(),
        },
    };
    let out = ordo::run(input);
    let items = build_items(&out);
    let groups = group_reasons(&out);
    if items.is_empty() {
        return Err("that strategy leaves nothing to review".to_string());
    }
    let reviewed: Vec<bool> = items
        .iter()
        .map(|it| mark_key(&app.rev, it, &app.sources).is_some_and(|k| app.marks.contains_key(&k)))
        .collect();
    let glob = app.path_filter.as_ref().map(|(_, g)| g);
    let view = compute_view(&items, app.comments_only, app.show_all, glob);
    if view.is_empty() {
        return Err("that strategy leaves nothing visible under the current filters".to_string());
    }
    app.items = items;
    app.reviewed = reviewed;
    app.view = view;
    app.groups = groups;
    app.jumps.clear();
    app.popup = None;
    app.search = None;
    app.strategy = name.to_string();
    select(app, app.view[0]);
    Ok(())
}

/// Recomputes `app.view` under new filter state and commits it — rejecting
/// (leaving `app` untouched) a combination that would leave nothing to
/// review, so a stray `:filter`/`:only-comments`/`:all` never strands the
/// review on an empty pane. Clamps `app.sel` onto the new view when the
/// selected hunk itself got filtered out.
fn set_filters(
    app: &mut App,
    comments_only: bool,
    show_all: bool,
    path_filter: Option<(String, PathGlobs)>,
) -> Result<(), String> {
    let glob = path_filter.as_ref().map(|(_, g)| g);
    let view = compute_view(&app.items, comments_only, show_all, glob);
    if view.is_empty() {
        return Err("that filter combination leaves nothing to review".to_string());
    }
    app.comments_only = comments_only;
    app.show_all = show_all;
    app.path_filter = path_filter;
    app.view = view;
    if !app.view.contains(&app.sel) {
        select(app, app.view[0]);
    }
    Ok(())
}

fn run_goto(app: &mut App, path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("usage: :goto <path>".to_string());
    }
    let target = app
        .view
        .iter()
        .copied()
        .find(|&i| app.items[i].path == path);
    let Some(idx) = target else {
        return Err(format!(
            "no visible hunk for path '{path}' (clear filters with :filter, :all, :only-comments)"
        ));
    };
    select(app, idx);
    app.focus = Pane::Code;
    Ok(())
}

/// Runs a full command line (without its leading `:`) — the command bar's
/// `Enter` handler once nothing is selected in its completion menu. `Err`
/// carries a message shown in a popup rather than silently doing nothing, so
/// a typo or a bad argument is never mistaken for "nothing happened".
fn execute_command(app: &mut App, line: &str) -> Result<CommandOutcome, String> {
    let (name, arg) = parse_command_line(line);
    match name {
        "" => Ok(CommandOutcome::None),
        "q" => Ok(CommandOutcome::Quit),
        "help" => {
            app.popup = Some(Popup::new(
                "commands",
                build_command_help().into_iter().map(prose).collect(),
            ));
            Ok(CommandOutcome::None)
        }
        "rules" => {
            let lines: Vec<Line<'static>> = app
                .rules_report
                .iter()
                .map(|l| Line::from(l.clone()))
                .collect();
            app.popup = Some(Popup::new("rules", lines));
            Ok(CommandOutcome::None)
        }
        "audit" => {
            let hidden = hidden_breakdown(
                &app.items,
                app.comments_only,
                app.show_all,
                app.path_filter.as_ref().map(|(_, g)| g),
            );
            app.popup = Some(Popup::new(
                "audit",
                build_audit(
                    &app.items,
                    app.view.len(),
                    &hidden,
                    &app.ledger,
                    app.path_filter.as_ref().map(|(p, _)| p.as_str()),
                )
                .into_iter()
                .map(prose)
                .collect(),
            ));
            Ok(CommandOutcome::None)
        }
        "only-comments" => {
            set_filters(
                app,
                !app.comments_only,
                app.show_all,
                app.path_filter.clone(),
            )?;
            Ok(CommandOutcome::None)
        }
        "all" => {
            set_filters(
                app,
                app.comments_only,
                !app.show_all,
                app.path_filter.clone(),
            )?;
            Ok(CommandOutcome::None)
        }
        "filter" => {
            let arg = arg.trim();
            let next = if arg.is_empty() {
                None
            } else {
                let globs = build_globs(&[arg.to_string()])?;
                Some((arg.to_string(), globs))
            };
            set_filters(app, app.comments_only, app.show_all, next)?;
            Ok(CommandOutcome::None)
        }
        "theme" => {
            let want = arg.trim();
            if want.is_empty() {
                // no argument: show what there is, rather than an error about
                // the argument the reviewer is trying to discover
                app.popup = Some(Popup::new(
                    "themes",
                    theme_names()
                        .iter()
                        .map(|n| {
                            let mark = if *n == app.theme.name { "▸ " } else { "  " };
                            prose(format!("{mark}{n}"))
                        })
                        .collect(),
                ));
                return Ok(CommandOutcome::None);
            }
            let Some(t) = theme(want) else {
                return Err(format!("unknown theme '{want}' (try :theme to list them)"));
            };
            app.theme = t;
            // syntax colours are baked into the highlight cache at load time,
            // so a live theme swap has to re-highlight what is on screen
            let paths: Vec<String> = app.highlights.keys().cloned().collect();
            for path in paths {
                let Some((_, nl)) = app.sources.get(&path) else {
                    continue;
                };
                if let Some(h) = highlight_file(&path, &nl.join("\n"), &app.theme.syn) {
                    app.highlights.insert(path, h);
                }
            }
            Ok(CommandOutcome::None)
        }
        "keys" => {
            let preset = arg.trim();
            let Some(km) = keymap(preset) else {
                return Err(format!("unknown key preset '{preset}' (want: vim, vscode)"));
            };
            app.keys = km;
            Ok(CommandOutcome::None)
        }
        "strategy" => {
            run_strategy(app, arg.trim())?;
            Ok(CommandOutcome::None)
        }
        "group" => {
            app.show_groups = !app.show_groups;
            Ok(CommandOutcome::None)
        }
        "rule" => {
            app.popup = Some(Popup::new(
                "draft rule".to_string(),
                draft_rule(app, app.sel).into_iter().map(prose).collect(),
            ));
            Ok(CommandOutcome::None)
        }
        "delta" => {
            if app.deltas.iter().all(|d| *d == Delta::New) {
                return Err("no previous run of this review to compare against".to_string());
            }
            let count = |k: Delta| app.deltas.iter().filter(|d| **d == k).count();
            let (new, changed, moved) = (
                count(Delta::New),
                count(Delta::Changed),
                count(Delta::Moved),
            );
            let mut lines = vec![
                format!("{new} new"),
                format!("{changed} changed"),
                // the one no other tool reports
                format!("{moved} unchanged but reordered"),
                format!("{} gone", app.delta_gone),
            ];
            lines.push(String::new());
            lines.extend(
                app.view
                    .iter()
                    .filter(|&&i| matches!(app.deltas.get(i), Some(Delta::Moved)))
                    .map(|&i| {
                        format!(
                            "  moved: {}:L{}  {}",
                            app.items[i].path, app.items[i].new_range[0], app.items[i].rationale
                        )
                    }),
            );
            app.popup = Some(Popup::new(
                "since you last looked".to_string(),
                lines.into_iter().map(prose).collect(),
            ));
            Ok(CommandOutcome::None)
        }
        "note" => {
            let it = &app.items[app.sel];
            let Some(key) = note_key(it) else {
                return Err(
                    "this hunk declares no symbol to anchor a note to (try one that defines something)"
                        .to_string(),
                );
            };
            let text = arg.trim();
            if text.is_empty() {
                app.notes.remove(&key);
            } else {
                app.notes.insert(key, text.to_string());
            }
            if let Some(p) = app.notes_path.as_deref() {
                save_notes(p, &app.notes);
            }
            Ok(CommandOutcome::None)
        }
        "mode" => {
            let to = match arg.trim() {
                "" => match app.mode {
                    ViewMode::Ledger => ViewMode::Hunks,
                    ViewMode::Hunks => ViewMode::Ledger,
                },
                "ledger" | "symbols" | "symbol" => ViewMode::Ledger,
                "hunks" | "hunk" => ViewMode::Hunks,
                other => return Err(format!("unknown mode: {other} (ledger, hunks)")),
            };
            let led = app.symbol_ledger.clone();
            set_mode(app, to, &led);
            Ok(CommandOutcome::None)
        }
        "goto" => {
            run_goto(app, arg.trim())?;
            Ok(CommandOutcome::None)
        }
        "quickfix" | "qf" | "vim-qfl" => {
            let hunks: Vec<QfHunk> = app
                .view
                .iter()
                .map(|&i| {
                    let it = &app.items[i];
                    let warn =
                        it.rules.iter().any(|r| r.level == "warn") || !it.advisories.is_empty();
                    QfHunk {
                        filename: it.path.clone(),
                        lnum: it.new_range[0],
                        kind: qf_kind(warn, app.reviewed[i]),
                        cluster: it.cluster.map(|n| format!("c{n}")),
                        text: it.rationale.clone(),
                    }
                })
                .collect();
            let script = quickfix_script(&app.rev, &app.strategy, &hunks);
            let path = write_quickfix_script(&script)?;
            if is_vim_family(&resolve_editor()) {
                Ok(CommandOutcome::OpenQuickfix(path))
            } else {
                app.popup = Some(Popup::new(
                    "quickfix",
                    vec![
                        prose(format!("wrote {}", path.display())),
                        prose(format!(":source {}", path.display())),
                    ],
                ));
                Ok(CommandOutcome::None)
            }
        }
        "e" => {
            let rev_arg = arg.trim();
            if rev_arg.is_empty() {
                return Err("usage: :e <rev>".to_string());
            }
            let Some(target) = resolve(rev_arg) else {
                return Err(format!(
                    "'{rev_arg}' is not a git revision, a commit range or a \
                     GitButler CLI ID (see `but status`)"
                ));
            };
            Ok(CommandOutcome::Reload(target, rev_arg.to_string()))
        }
        other => Err(format!("unknown command '{other}' — :help lists them")),
    }
}

// The TUI can't be driven headless (`ratatui::init()` needs a real tty), so
// what's testable here is factored into small pure functions above — cursor
// arithmetic, scroll-follow clamping, and symbol/signature/docstring
// extraction — and exercised directly.
#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    // ---- markdown highlighting ----

    #[test]
    fn markdown_block_structure_is_highlighted() {
        let syn = Theme::terminal("dark", false).syn;
        let src = "# Title\n\n- item\n\n```rust\nfn f() {}\n```\n";
        let h = highlight_file("f.md", src, &syn).unwrap();
        // heading marker, list marker and fence delimiters all get a
        // non-default colour (the terminal theme's `Reset` is the default).
        let heading_marker = h[0].iter().find(|(t, _)| t == "#").unwrap();
        assert_ne!(heading_marker.1, Color::Reset);
        let list_marker = h[2].iter().find(|(t, _)| t.starts_with('-')).unwrap();
        assert_ne!(list_marker.1, Color::Reset);
        let fence_open = h[4].iter().find(|(t, _)| t == "```").unwrap();
        assert_ne!(fence_open.1, Color::Reset);
    }

    #[test]
    fn markdown_inline_emphasis_is_highlighted() {
        let syn = Theme::terminal("dark", false).syn;
        let src = "plain and *emphasised* text\n";
        let h = highlight_file("f.md", src, &syn).unwrap();
        let word = h[0].iter().find(|(t, _)| t == "emphasised").unwrap();
        assert_eq!(word.1, syn.keyword);
    }

    #[test]
    fn markdown_fenced_rust_uses_rust_grammar() {
        let syn = Theme::terminal("dark", false).syn;
        let src = "```rust\nfn add() {\n    let x = 1;\n}\n```\n";
        let h = highlight_file("f.md", src, &syn).unwrap();
        let fn_kw = h[1].iter().find(|(t, _)| t == "fn").unwrap();
        assert_eq!(fn_kw.1, syn.keyword);
        let let_kw = h[2].iter().find(|(t, _)| t == "let").unwrap();
        assert_eq!(let_kw.1, syn.keyword);
    }

    #[test]
    fn markdown_fence_in_unsupported_language_does_not_panic() {
        let syn = Theme::terminal("dark", false).syn;
        let src = "```console\n$ echo hi\nhi\n```\n";
        let h = highlight_file("f.md", src, &syn).unwrap();
        assert_eq!(h.len(), src.lines().count() + 1);
    }

    #[test]
    fn non_markdown_file_highlighting_is_unaffected() {
        let syn = Theme::terminal("dark", false).syn;
        let src = "fn add(a: i32) -> i32 {\n    a\n}\n";
        let h = highlight_file("f.rs", src, &syn).unwrap();
        let fn_kw = h[0].iter().find(|(t, _)| t == "fn").unwrap();
        assert_eq!(fn_kw.1, syn.keyword);
        let ty = h[0].iter().find(|(t, _)| t == "i32").unwrap();
        assert_eq!(ty.1, syn.type_);
    }

    // ---- background-load progress messages ----

    #[test]
    fn read_progress_is_one_based() {
        assert_eq!(
            read_progress("src/foo.rs", 0, 30),
            "reading src/foo.rs (1/30)"
        );
        assert_eq!(
            read_progress("src/foo.rs", 11, 30),
            "reading src/foo.rs (12/30)"
        );
        assert_eq!(
            read_progress("src/foo.rs", 29, 30),
            "reading src/foo.rs (30/30)"
        );
    }

    #[test]
    fn highlight_progress_is_one_based() {
        assert_eq!(highlight_progress(0, 27), "highlighting (1/27)");
        assert_eq!(highlight_progress(7, 27), "highlighting (8/27)");
    }

    // ---- cursor column/line arithmetic ----

    #[test]
    fn move_col_clamps_within_line() {
        let ls = lines(&["abc"]);
        let c = Cursor { line: 0, col: 0 };
        assert_eq!(move_col(c, &ls, -1), Cursor { line: 0, col: 0 });
        assert_eq!(move_col(c, &ls, 1), Cursor { line: 0, col: 1 });
        assert_eq!(
            move_col(Cursor { line: 0, col: 2 }, &ls, 5),
            Cursor { line: 0, col: 2 }
        );
    }

    #[test]
    fn move_line_clamps_and_carries_column() {
        let ls = lines(&["hello", "hi", ""]);
        let c = Cursor { line: 0, col: 4 };
        assert_eq!(move_line(c, &ls, 1), Cursor { line: 1, col: 1 }); // "hi" only has col 0/1
        assert_eq!(move_line(c, &ls, 2), Cursor { line: 2, col: 0 }); // empty line -> col 0
        assert_eq!(move_line(c, &ls, -5), Cursor { line: 0, col: 4 });
    }

    #[test]
    fn line_start_end() {
        let ls = lines(&["abcdef"]);
        let c = Cursor { line: 0, col: 3 };
        assert_eq!(line_start(c, &ls), Cursor { line: 0, col: 0 });
        assert_eq!(line_end(c, &ls), Cursor { line: 0, col: 5 });
    }

    #[test]
    fn clamp_cursor_handles_shrunk_or_empty_files() {
        let ls = lines(&["ab"]);
        assert_eq!(
            clamp_cursor(Cursor { line: 5, col: 5 }, &ls),
            Cursor { line: 0, col: 1 }
        );
        assert_eq!(
            clamp_cursor(Cursor { line: 0, col: 0 }, &[]),
            Cursor { line: 0, col: 0 }
        );
    }

    // ---- word motion ----

    #[test]
    fn word_next_skips_word_then_whitespace() {
        let ls = lines(&["foo bar  baz"]);
        let c = Cursor { line: 0, col: 0 };
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 4 }); // "bar"
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 9 }); // "baz"
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 11 }); // no next word: last char of buffer
    }

    #[test]
    fn word_next_crosses_lines() {
        let ls = lines(&["foo", "bar"]);
        let c = word_next(Cursor { line: 0, col: 0 }, &ls);
        assert_eq!(c, Cursor { line: 1, col: 0 });
    }

    #[test]
    fn word_prev_mirrors_word_next() {
        let ls = lines(&["foo bar  baz"]);
        let c = word_prev(Cursor { line: 0, col: 9 }, &ls);
        assert_eq!(c, Cursor { line: 0, col: 4 });
        let c = word_prev(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 0 });
        let c = word_prev(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 0 });
    }

    #[test]
    fn word_end_lands_on_last_char_of_word() {
        let ls = lines(&["foo bar"]);
        let c = word_end(Cursor { line: 0, col: 0 }, &ls);
        assert_eq!(c, Cursor { line: 0, col: 2 }); // end of "foo"
        let c = word_end(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 6 }); // end of "bar"
    }

    #[test]
    fn word_motion_treats_punctuation_as_its_own_run() {
        let ls = lines(&["foo(bar)"]);
        let c = word_next(Cursor { line: 0, col: 0 }, &ls);
        assert_eq!(c, Cursor { line: 0, col: 3 }); // "("
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 4 }); // "bar"
    }

    // ---- paragraph motion ----

    #[test]
    fn para_next_prev_find_blank_lines() {
        let ls = lines(&["a", "b", "", "c", "d"]);
        assert_eq!(
            para_next(Cursor { line: 0, col: 0 }, &ls),
            Cursor { line: 2, col: 0 }
        );
        assert_eq!(
            para_prev(Cursor { line: 4, col: 0 }, &ls),
            Cursor { line: 2, col: 0 }
        );
        assert_eq!(
            para_prev(Cursor { line: 0, col: 0 }, &ls),
            Cursor { line: 0, col: 0 }
        );
    }

    // ---- scroll-follow clamping ----

    #[test]
    fn follow_scroll_keeps_cursor_in_view() {
        assert_eq!(follow_scroll(5, 0, 10), 0); // already visible
        assert_eq!(follow_scroll(0, 5, 10), 0); // scrolled past top -> jump up
        assert_eq!(follow_scroll(20, 5, 10), 11); // scrolled past bottom -> jump down
        assert_eq!(follow_scroll(9, 0, 10), 0); // last visible row stays put
        assert_eq!(follow_scroll(10, 0, 10), 1); // one past the last visible row
    }

    // ---- byte/char column conversion ----

    #[test]
    fn char_byte_handles_multibyte() {
        let line = "héllo";
        assert_eq!(char_byte(line, 0), 0);
        assert_eq!(char_byte(line, 1), 1); // 'é' starts at byte 1
        assert_eq!(char_byte(line, 2), 3); // 'l' starts after the 2-byte 'é'
        assert_eq!(char_byte(line, 99), line.len());
    }

    // ---- symbol resolution + signature/docstring extraction ----

    fn parse(lang: tree_sitter::Language, src: &str) -> tree_sitter::Tree {
        let mut p = Parser::new();
        p.set_language(&lang).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn resolves_rust_call_site_to_its_definition() {
        let src = "/// adds two numbers\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    add(1, 2);\n}\n";
        let tree = parse(tree_sitter_rust::LANGUAGE.into(), src);
        let root = tree.root_node();
        // cursor on `add` inside `add(1, 2)` — row 6, "    add(1, 2);"
        let point = Point { row: 6, column: 5 };
        let mut node = root.descendant_for_point_range(point, point).unwrap();
        while !node.kind().contains("identifier") {
            node = node.parent().unwrap();
        }
        let name = node_text(node, src);
        assert_eq!(name, "add");
        let kinds = def_kinds("f.rs");
        let def = find_definition(root, kinds, &name, src).unwrap();
        assert_eq!(def.kind(), "function_item");
        assert_eq!(signature(def, src), "fn add(a: i32, b: i32) -> i32");
        assert_eq!(
            doc_for("f.rs", def, src).as_deref(),
            Some("/// adds two numbers")
        );
    }

    #[test]
    fn reports_missing_rust_definition_plainly() {
        let src = "fn main() {\n    unknown_fn();\n}\n";
        let tree = parse(tree_sitter_rust::LANGUAGE.into(), src);
        let root = tree.root_node();
        let point = Point { row: 1, column: 5 };
        let mut node = root.descendant_for_point_range(point, point).unwrap();
        while !node.kind().contains("identifier") {
            node = node.parent().unwrap();
        }
        let name = node_text(node, src);
        assert_eq!(name, "unknown_fn");
        assert!(find_definition(root, def_kinds("f.rs"), &name, src).is_none());
    }

    #[test]
    fn extracts_python_docstring_and_signature() {
        let src = "def greet(name):\n    \"\"\"Say hello.\"\"\"\n    print(name)\n";
        let tree = parse(tree_sitter_python::LANGUAGE.into(), src);
        let root = tree.root_node();
        let def = find_definition(root, def_kinds("f.py"), "greet", src).unwrap();
        assert_eq!(def.kind(), "function_definition");
        assert_eq!(signature(def, src), "def greet(name):");
        assert_eq!(
            doc_for("f.py", def, src).as_deref(),
            Some("\"\"\"Say hello.\"\"\"")
        );
    }

    #[test]
    fn c_function_definition_name_comes_from_declarator() {
        // C has no `name` field on function_definition — the identifier is
        // nested in `declarator`, past the parameter_list.
        let src = "// squares x\nint square(int x) {\n    return x * x;\n}\n";
        let tree = parse(tree_sitter_c::LANGUAGE.into(), src);
        let root = tree.root_node();
        let def = find_definition(root, def_kinds("f.c"), "square", src).unwrap();
        assert_eq!(def.kind(), "function_definition");
        assert_eq!(signature(def, src), "int square(int x)");
        assert_eq!(doc_for("f.c", def, src).as_deref(), Some("// squares x"));
    }

    // ---- search: symbol occurrences vs. text search ----

    #[test]
    fn byte_to_char_col_is_the_inverse_of_char_byte() {
        let line = "héllo";
        for col in 0..line.chars().count() {
            assert_eq!(byte_to_char_col(line, char_byte(line, col)), col);
        }
    }

    // A real case from a test corpus: a local `may_refine` bound once and used
    // twice, alongside `may_refine_camber_span` (which contains `may_refine`
    // as a substring) appearing 4 times. Symbol-occurrence search must find
    // exactly the 3 identifier nodes named `may_refine` and none of the 4
    // longer-named ones; text search, by contrast, matches the substring
    // everywhere it appears — including inside the longer name.
    #[test]
    fn symbol_occurrences_ignore_substring_matches() {
        let src = "def refine(may_refine_camber_span):\n\
                   \x20   may_refine = may_refine_camber_span > 0\n\
                   \x20   if may_refine:\n\
                   \x20       return may_refine_camber_span\n\
                   \x20   return may_refine or may_refine_camber_span\n";
        let lines: Vec<String> = src.lines().map(str::to_string).collect();
        let tree = parse(tree_sitter_python::LANGUAGE.into(), src);
        let root = tree.root_node();

        let matches = symbol_matches(root, "may_refine", src, &lines);
        assert_eq!(matches.len(), 3); // 1 binding + 2 uses
        for &(line, s, e) in &matches {
            assert_eq!(&lines[line][s..e], "may_refine");
        }

        let long_matches = symbol_matches(root, "may_refine_camber_span", src, &lines);
        assert_eq!(long_matches.len(), 4);
    }

    #[test]
    fn text_search_matches_substrings_unlike_symbol_search() {
        let src = "def refine(may_refine_camber_span):\n\
                   \x20   may_refine = may_refine_camber_span > 0\n\
                   \x20   if may_refine:\n\
                   \x20       return may_refine_camber_span\n\
                   \x20   return may_refine or may_refine_camber_span\n";
        let lines: Vec<String> = src.lines().map(str::to_string).collect();
        // every occurrence of the literal substring, standalone or embedded —
        // 3 standalone `may_refine` + 4 embedded in `may_refine_camber_span`
        let matches = text_matches(&lines, "may_refine");
        assert_eq!(matches.len(), 7);
    }

    #[test]
    fn text_matches_empty_pattern_finds_nothing() {
        let lines = lines(&["abc"]);
        assert!(text_matches(&lines, "").is_empty());
    }

    // ---- next/previous match: seeking and wrap-around ----

    #[test]
    fn cycle_index_wraps_both_directions() {
        assert_eq!(cycle_index(0, 3, 1), 1);
        assert_eq!(cycle_index(2, 3, 1), 0); // wraps forward
        assert_eq!(cycle_index(0, 3, -1), 2); // wraps backward
        assert_eq!(cycle_index(0, 0, 1), 0); // empty: no panic, degenerate 0
    }

    #[test]
    fn seek_forward_wraps_when_nothing_ahead() {
        let matches = vec![(0, 0, 3), (2, 0, 3)];
        let cursor = Cursor { line: 5, col: 0 };
        assert_eq!(seek_forward(&matches, cursor, true), Some(0));
        // strict (non-inclusive) seek skips a match starting exactly at cursor
        let cursor = Cursor { line: 0, col: 0 };
        assert_eq!(seek_forward(&matches, cursor, false), Some(1));
        assert_eq!(seek_forward(&matches, cursor, true), Some(0));
    }

    #[test]
    fn seek_backward_wraps_when_nothing_behind() {
        let matches = vec![(0, 0, 3), (2, 0, 3)];
        let cursor = Cursor { line: 0, col: 0 };
        assert_eq!(seek_backward(&matches, cursor, false), Some(1)); // wraps to last
        let cursor = Cursor { line: 2, col: 0 };
        assert_eq!(seek_backward(&matches, cursor, true), Some(1));
        assert_eq!(seek_backward(&matches, cursor, false), Some(0));
    }

    #[test]
    fn search_seek_on_empty_matches_returns_none() {
        let cursor = Cursor { line: 0, col: 0 };
        assert_eq!(seek_forward(&[], cursor, true), None);
        assert_eq!(seek_backward(&[], cursor, false), None);
    }

    // ---- cross-commit history: identity, windowing, labeling ----

    fn sym(name: &str, kind: &str, scope: Option<&str>) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: kind.to_string(),
            scope: scope.map(str::to_string),
        }
    }

    // The project owner's rule, verbatim: "ordo tree sitter type + scope for
    // the symbol must match, otherwise it's a different symbol". A module-level
    // `run` must NOT match a method `run` on class `A`, nor a same-scope `run`
    // of a different tree-sitter kind — name alone is never enough.
    #[test]
    fn symbol_eq_requires_matching_name_kind_and_scope() {
        let module_level = sym("run", "function_definition", None);
        let method_a = sym("run", "function_definition", Some("A"));
        assert!(!symbol_eq(&module_level, &method_a));

        let different_kind = sym("run", "async_function_definition", None);
        assert!(!symbol_eq(&module_level, &different_kind));

        let same = sym("run", "function_definition", None);
        assert!(symbol_eq(&module_level, &same));
    }

    #[test]
    fn qualified_name_joins_scope_and_name() {
        assert_eq!(
            qualified_name(&sym("run", "function_definition", None)),
            "run"
        );
        assert_eq!(
            qualified_name(&sym("run", "function_definition", Some("A"))),
            "A.run"
        );
    }

    #[test]
    fn bound_earlier_drops_current_and_orders_oldest_of_window_first() {
        let shas: Vec<String> = ["current", "a", "b", "c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let got = bound_earlier(shas, "current", 2);
        assert_eq!(got, vec!["b".to_string(), "a".to_string()]); // nearest 2, oldest first
    }

    #[test]
    fn bound_earlier_bounds_a_long_lived_files_history() {
        let shas: Vec<String> = (0..20).map(|i| format!("c{i}")).collect(); // c0 nearest
        let got = bound_earlier(shas, "none-of-these", 10);
        assert_eq!(got.len(), 10);
        assert_eq!(got.first().unwrap(), "c9"); // oldest of the nearest-10 window
        assert_eq!(got.last().unwrap(), "c0"); // nearest to the reviewed rev
    }

    #[test]
    fn bound_later_orders_nearest_to_rev_first_and_bounds_the_window() {
        // rev-list's own order: HEAD first, nearest descendant of rev last
        let shas: Vec<String> = ["head", "b", "a"].iter().map(|s| s.to_string()).collect();
        let got = bound_later(shas, 10);
        assert_eq!(
            got,
            vec!["a".to_string(), "b".to_string(), "head".to_string()]
        );

        let many: Vec<String> = (0..20).map(|i| format!("c{i}")).collect(); // c19 nearest to rev
        let got = bound_later(many, 10);
        assert_eq!(got.len(), 10);
        assert_eq!(got.first().unwrap(), "c19");
    }

    fn test_hunk(rationale: &str, enclosing: Option<&str>, symbols: Vec<Symbol>) -> HunkOut {
        HunkOut {
            id: "h1".to_string(),
            old_range: [0, 0],
            new_range: [1, 1],
            category: ordo::model::Category::Definition,
            enclosing: enclosing.map(str::to_string),
            enclosing_kind: None,
            rules: vec![],
            defines: vec![],
            uses: vec![],
            group: "g".to_string(),
            order_index: 0,
            rationale: rationale.to_string(),
            noise: false,
            comment: false,
            details: vec![],
            notes: vec![],
            advisories: vec![],
            symbols,
        }
    }

    #[test]
    fn label_for_picks_the_fragment_naming_the_symbol() {
        let target = sym("run", "function_definition", None);
        let h = test_hunk("adds helper; changes signature of run", None, vec![]);
        assert_eq!(label_for(&h, &target), "changes signature of run");
    }

    #[test]
    fn label_for_falls_back_to_the_whole_rationale_for_a_body_edit() {
        let target = sym("run", "function_definition", Some("A"));
        let h = test_hunk("edits A.run", Some("A.run"), vec![]);
        assert_eq!(label_for(&h, &target), "edits A.run");
    }

    // ---- open-in-editor: command building ----

    #[test]
    fn split_command_separates_program_and_flags() {
        assert_eq!(split_command("code --wait"), vec!["code", "--wait"]);
        assert_eq!(split_command("vim"), vec!["vim"]);
        assert_eq!(
            split_command("emacsclient  -nw  -a ''"),
            vec!["emacsclient", "-nw", "-a", "''"]
        );
    }

    #[test]
    fn editor_args_covers_each_line_argument_shape() {
        assert_eq!(
            editor_args("vim", "path", 120),
            vec!["+120".to_string(), "path".to_string()]
        );
        assert_eq!(
            editor_args("nvim", "path", 120),
            vec!["+120".to_string(), "path".to_string()]
        );
        assert_eq!(editor_args("hx", "path", 120), vec!["path:120".to_string()]);
        assert_eq!(
            editor_args("code", "path", 120),
            vec!["-g".to_string(), "path:120".to_string()]
        );
        // unknown editor: file only — never a guessed syntax it might read as
        // another filename
        assert_eq!(editor_args("subl", "path", 120), vec!["path".to_string()]);
    }

    #[test]
    fn build_command_keeps_editor_flags_ahead_of_the_location() {
        let spec = split_command("code --wait");
        let (program, args) = build_command(&spec, "path", 120).unwrap();
        assert_eq!(program, "code");
        assert_eq!(
            args,
            vec![
                "--wait".to_string(),
                "-g".to_string(),
                "path:120".to_string()
            ]
        );
    }

    #[test]
    fn build_command_matches_on_basename_not_full_path() {
        let spec = split_command("/usr/local/bin/hx");
        let (program, args) = build_command(&spec, "path", 120).unwrap();
        assert_eq!(program, "/usr/local/bin/hx");
        assert_eq!(args, vec!["path:120".to_string()]);
    }

    #[test]
    fn build_command_is_none_for_an_empty_spec() {
        assert!(build_command(&[], "path", 120).is_none());
    }

    // ---- key/chord rendering ----

    #[test]
    fn key_label_renders_plain_and_modified_keys() {
        assert_eq!(key_label(ch('j')), "j");
        assert_eq!(key_label(ctrl('w')), "C-w");
        assert_eq!(key_label(plain(KeyCode::F(12))), "F12");
        assert_eq!(key_label((KeyCode::F(3), KeyModifiers::SHIFT)), "S-F3");
        assert_eq!(key_label(plain(KeyCode::Esc)), "Esc");
    }

    #[test]
    fn chord_label_renders_known_chords() {
        // plain-char chords render tight, like vim's own "gg"/"ge" spelling
        assert_eq!(chord_label(Some(ch('g')), ch('g')), "gg");
        assert_eq!(chord_label(Some(ch('g')), ch('e')), "ge");
        // a modified prefix or key renders spaced, like "C-w C-w"
        assert_eq!(chord_label(Some(ctrl('w')), ctrl('w')), "C-w C-w");
        assert_eq!(chord_label(Some(ctrl('w')), ch('h')), "C-w h");
        assert_eq!(chord_label(None, ch('q')), "q");
    }

    // ---- generated keybinding help ----

    #[test]
    fn help_text_is_generated_from_the_bind_table() {
        let km = keymap("vim").unwrap();
        let help = build_help(&km);
        // `Hover` is bound to `K` in vim's own bind table (see `keymap`) — if
        // that binding ever changes, this row (built from the table, not
        // hand-copied) changes with it, and this assertion breaks.
        let (_, desc) = action_help(Action::Hover);
        let row = help
            .iter()
            .find(|l| l.contains(desc))
            .expect("hover row present");
        assert!(row.contains('K'), "expected the hover row to list K: {row}");
    }

    #[test]
    fn help_collapses_multiple_keys_bound_to_the_same_action() {
        let km = keymap("vim").unwrap();
        let help = build_help(&km);
        // `Next` is bound to both `j` and `Down` — one row, both keys.
        let (_, desc) = action_help(Action::Next);
        let row = help
            .iter()
            .find(|l| l.contains(desc))
            .expect("next row present");
        assert!(
            row.contains('j') && row.contains("Down"),
            "expected both keys on one row: {row}"
        );
    }

    #[test]
    fn help_covers_both_presets_without_panicking() {
        for name in ["vim", "vscode"] {
            let km = keymap(name).unwrap();
            let help = build_help(&km);
            assert!(!help.is_empty());
        }
    }

    #[test]
    fn excerpt_numbers_lines_and_uses_highlight_segments_when_present() {
        let lines: Vec<String> = vec!["fn a() {}".into(), "let x = 1;".into(), "done".into()];
        // no grammar: falls back to raw text, still gutter-numbered
        let plainly = excerpt(&lines, None, 2, 3, &Theme::terminal("dark", false));
        assert_eq!(plainly.len(), 2);
        let first: String = plainly[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(first, "    2 let x = 1;");

        // with highlights: the segments are used, each carrying its own colour
        let hl: Vec<LineSpans> = vec![
            vec![],
            vec![
                ("let ".to_string(), Color::Magenta),
                ("x = 1;".to_string(), Color::Reset),
            ],
            vec![],
        ];
        let lit = excerpt(&lines, Some(&hl), 2, 2, &Theme::terminal("dark", false));
        assert_eq!(lit.len(), 1);
        assert_eq!(lit[0].spans.len(), 3, "gutter + two coloured segments");
        assert_eq!(lit[0].spans[1].style.fg, Some(Color::Magenta));
    }

    #[test]
    fn popup_width_measures_the_widest_line_across_its_spans() {
        let lines = vec![
            Line::from("short"),
            Line::from(vec![Span::raw("12345"), Span::raw("67890")]),
            Line::from("mid"),
        ];
        assert_eq!(popup_width(&lines), 10, "spans on one line sum");
        assert_eq!(popup_width(&[]), 0);
    }

    #[test]
    fn excerpt_stops_at_the_end_of_the_file() {
        let lines: Vec<String> = vec!["only".into()];
        assert_eq!(
            excerpt(&lines, None, 1, 9, &Theme::terminal("dark", false)).len(),
            1
        );
        assert!(excerpt(&lines, None, 5, 9, &Theme::terminal("dark", false)).is_empty());
    }

    #[test]
    fn vim_takes_arrows_after_a_window_chord() {
        // vim accepts arrows wherever it accepts hjkl; a reviewer who reaches
        // for C-w Right should land where C-w l lands
        let km = keymap("vim").unwrap();
        let w = Some(ctrl('w'));
        for (key, want) in [
            (plain(KeyCode::Left), Pane::List),
            (plain(KeyCode::Right), Pane::Code),
            (plain(KeyCode::Up), Pane::Code),
            (plain(KeyCode::Down), Pane::Why),
        ] {
            assert!(
                matches!(km.resolve(w, key), Resolve::Act(Action::Focus(p)) if p == want),
                "{}",
                chord_label(w, key)
            );
        }
        // and the chord still has to be opened first
        assert!(matches!(km.resolve(None, ctrl('w')), Resolve::Pending));
    }

    #[test]
    fn no_preset_binds_the_same_chord_twice() {
        // `Keymap::resolve` takes the first match, so a duplicate (prefix, key)
        // is silently shadowed rather than rejected — the second binding just
        // never fires. These tables have been edited by many separate changes,
        // so the invariant is worth asserting rather than assuming.
        for name in ["vim", "vscode"] {
            let km = keymap(name).unwrap();
            let mut seen: Vec<(Option<Key>, Key)> = vec![];
            for (prefix, key, _) in &km.binds {
                let pair = (*prefix, *key);
                assert!(
                    !seen.contains(&pair),
                    "{name}: {} is bound twice",
                    chord_label(*prefix, *key)
                );
                seen.push(pair);
            }
        }
    }

    #[test]
    fn every_bound_action_has_a_help_entry() {
        // The `?` help is generated from the bind table, so an Action without a
        // description would render a blank row instead of documenting the key.
        for name in ["vim", "vscode"] {
            let km = keymap(name).unwrap();
            for (prefix, key, action) in &km.binds {
                let (_, desc) = action_help(*action);
                assert!(
                    !desc.trim().is_empty(),
                    "{name}: {} has no help description",
                    chord_label(*prefix, *key)
                );
            }
        }
    }

    // ---- :quickfix ----

    fn qf_hunk(
        filename: &str,
        lnum: usize,
        kind: Option<char>,
        cluster: Option<&str>,
        text: &str,
    ) -> QfHunk {
        QfHunk {
            filename: filename.to_string(),
            lnum,
            kind,
            cluster: cluster.map(str::to_string),
            text: text.to_string(),
        }
    }

    #[test]
    fn quickfix_script_has_one_item_line_per_hunk_in_order() {
        let hunks = vec![
            qf_hunk("a.rs", 10, None, None, "first"),
            qf_hunk("b.rs", 20, Some('W'), Some("cluster 1"), "second"),
        ];
        let script = quickfix_script("HEAD", "comprehension", &hunks);
        assert!(script.contains("'nr': '$',"));
        let first = script.find("'filename': 'a.rs'").unwrap();
        let second = script.find("'filename': 'b.rs'").unwrap();
        assert!(first < second, "items must appear in the given order");
        assert!(script.contains("'lnum': 10"));
        assert!(script.contains("'lnum': 20"));
    }

    #[test]
    fn quickfix_script_doubles_apostrophes_and_keeps_unicode_verbatim() {
        let hunks = vec![qf_hunk("a.rs", 1, None, None, "don't lose → this ⚠")];
        let script = quickfix_script("HEAD", "comprehension", &hunks);
        assert!(script.contains("don''t lose → this ⚠"));
    }

    #[test]
    fn quickfix_script_never_sets_vims_module_key() {
        // vim renders `module` instead of the filename in the quickfix window,
        // so setting it would hide the path a reviewer navigates by
        let script = quickfix_script(
            "HEAD",
            "comprehension",
            &[
                qf_hunk("src/lib.rs", 12, None, Some("c2"), "wires it through"),
                qf_hunk("src/lang.rs", 75, None, None, "adds xonsh"),
            ],
        );
        assert!(!script.contains("'module'"), "{script}");
        assert!(script.contains("'filename': 'src/lib.rs'"), "{script}");
    }

    #[test]
    fn quickfix_script_prefixes_the_cluster_onto_the_text() {
        let script = quickfix_script(
            "HEAD",
            "comprehension",
            &[qf_hunk(
                "src/lib.rs",
                12,
                None,
                Some("c2"),
                "wires it through",
            )],
        );
        assert!(
            script.contains("'text': '[c2] wires it through'"),
            "{script}"
        );
    }

    #[test]
    fn quickfix_script_leaves_an_unclustered_text_alone() {
        let hunks = vec![qf_hunk("a.rs", 1, None, None, "plain")];
        let script = quickfix_script("HEAD", "comprehension", &hunks);
        assert!(script.contains("'text': 'plain'"), "{script}");
    }

    #[test]
    fn quickfix_script_titles_by_distinct_cluster_count() {
        let hunks = vec![
            qf_hunk("a.rs", 1, None, Some("c1"), "one"),
            qf_hunk("b.rs", 2, None, Some("c2"), "two"),
            qf_hunk("c.rs", 3, None, Some("c2"), "three"),
        ];
        let script = quickfix_script("HEAD", "comprehension", &hunks);
        assert!(script.contains("3 hunks, 2 clusters"), "{script}");
    }

    #[test]
    fn quickfix_script_on_an_empty_export_still_produces_a_valid_call() {
        let script = quickfix_script("HEAD", "comprehension", &[]);
        assert!(script.contains("'nr': '$',"));
        assert!(script.contains("'items': ["));
        assert!(script.trim_end().ends_with("]})"));
        // no item line was emitted
        assert!(!script.contains("'filename'"));
    }

    #[test]
    fn qf_kind_warn_beats_reviewed_and_neither_is_empty() {
        assert_eq!(qf_kind(true, false), Some('W'));
        assert_eq!(qf_kind(false, true), Some('I'));
        assert_eq!(qf_kind(true, true), Some('W'));
        assert_eq!(qf_kind(false, false), None);
    }

    #[test]
    fn is_vim_family_matches_vim_and_neovim_by_basename_only() {
        for prog in ["vim", "nvim", "/usr/bin/nvim", "vi", "gvim", "mvim"] {
            assert!(
                is_vim_family(&[prog.to_string()]),
                "{prog} should be vim-family"
            );
        }
        for prog in ["code", "emacs", "hx", "subl"] {
            assert!(
                !is_vim_family(&[prog.to_string()]),
                "{prog} should not be vim-family"
            );
        }
        assert!(!is_vim_family(&[]));
    }

    // ---- command mode: line parsing ----

    #[test]
    fn parse_command_line_splits_name_and_argument() {
        assert_eq!(
            parse_command_line("strategy defs-first"),
            ("strategy", "defs-first")
        );
        assert_eq!(parse_command_line("filter   src/*   "), ("filter", "src/*"));
        assert_eq!(parse_command_line("q"), ("q", ""));
        assert_eq!(parse_command_line("  q  "), ("q", ""));
        assert_eq!(parse_command_line(""), ("", ""));
    }

    // ---- command mode: the completion matcher ----

    #[test]
    fn complete_prefers_a_prefix_match_over_a_substring_one() {
        let candidates = lines(&["comprehension", "defs-first", "file"]);
        // "f" prefixes "file" but is also a substring of "defs-first" — the
        // prefix match must win outright, not just sort first
        assert_eq!(complete("f", &candidates), vec!["file".to_string()]);
    }

    #[test]
    fn complete_falls_back_to_substring_when_no_prefix_matches() {
        let candidates = lines(&["comprehension", "defs-first", "file"]);
        // "first" isn't a prefix of anything, but is a substring of "defs-first"
        assert_eq!(
            complete("first", &candidates),
            vec!["defs-first".to_string()]
        );
    }

    #[test]
    fn complete_is_case_insensitive() {
        let candidates = lines(&["vim", "vscode"]);
        assert_eq!(complete("VS", &candidates), vec!["vscode".to_string()]);
    }

    #[test]
    fn complete_empty_input_lists_everything_unfiltered() {
        let candidates = lines(&["vim", "vscode"]);
        assert_eq!(complete("", &candidates), candidates);
    }

    #[test]
    fn complete_no_match_yields_an_empty_menu() {
        let candidates = lines(&["vim", "vscode"]);
        assert!(complete("zzz", &candidates).is_empty());
    }

    // ---- command mode: argument completion per command ----

    #[test]
    fn command_completions_lists_command_names_before_any_space() {
        let got = command_completions("str", &[], &[], &[]);
        assert_eq!(got, vec!["strategy".to_string()]);
    }

    #[test]
    fn command_completions_lists_keys_presets() {
        let got = command_completions("keys ", &[], &[], &[]);
        assert_eq!(got, vec!["vim".to_string(), "vscode".to_string()]);
    }

    #[test]
    fn command_completions_lists_strategy_names() {
        let got = command_completions("strategy ", &[], &[], &[]);
        assert_eq!(
            got,
            vec![
                "comprehension".to_string(),
                "defs-first".to_string(),
                "file".to_string()
            ]
        );
    }

    #[test]
    fn command_completions_lists_goto_paths_and_filter_dirs_from_their_own_pools() {
        let goto_paths = lines(&["src/a.rs", "src/b.rs"]);
        let filter_dirs = lines(&["src", "tests"]);
        assert_eq!(
            command_completions("goto ", &goto_paths, &filter_dirs, &[]),
            goto_paths
        );
        assert_eq!(
            command_completions("filter ", &goto_paths, &filter_dirs, &[]),
            filter_dirs
        );
    }

    #[test]
    fn command_completions_narrows_the_argument_by_its_own_partial_word() {
        let goto_paths = lines(&["src/a.rs", "src/b.rs"]);
        assert_eq!(
            command_completions("goto src/b", &goto_paths, &[], &[]),
            vec!["src/b.rs".to_string()]
        );
    }

    #[test]
    fn command_completions_is_empty_for_a_no_argument_command() {
        assert!(command_completions("q ", &[], &[], &[]).is_empty());
        assert!(command_completions("only-comments ", &[], &[], &[]).is_empty());
    }

    #[test]
    fn apply_completion_replaces_the_word_being_completed_and_adds_a_trailing_space() {
        assert_eq!(apply_completion("str", "strategy"), "strategy ");
        assert_eq!(
            apply_completion("strategy defs", "defs-first"),
            "strategy defs-first "
        );
    }

    #[test]
    fn dir_prefix_and_distinct_sorted_derive_stable_glob_candidates() {
        assert_eq!(dir_prefix("src/bin/ordo.rs"), Some("src/bin"));
        assert_eq!(dir_prefix("Cargo.toml"), None);
        let got = distinct_sorted(["src/b.rs", "src/a.rs", "src/a.rs"].into_iter());
        assert_eq!(got, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
    }

    // ---- command mode: `:help` generated from the command table ----

    #[test]
    fn completing_a_command_name_shows_its_help_sentence_beside_it() {
        let names = command_names();
        let (head, help) = command_menu_row("strategy", true, &names, 120);
        assert!(head.starts_with("strategy <"), "{head:?}");
        let strategy = COMMANDS.iter().find(|c| c.name == "strategy").unwrap();
        assert_eq!(
            help.as_deref(),
            Some(format!("  — {}", strategy.help).as_str())
        );
        // every name pads to the same column, so the sentences line up
        let (h1, _) = command_menu_row("q", true, &names, 120);
        assert_eq!(h1.chars().count(), head.chars().count());
    }

    #[test]
    fn a_narrow_menu_cuts_the_sentence_and_a_very_narrow_one_drops_it() {
        let names = command_names();
        let (_, help) = command_menu_row("strategy", true, &names, 60);
        let help = help.unwrap();
        assert!(help.ends_with('…'), "{help:?}");
        assert!(help.chars().count() <= 60);
        let (_, none) = command_menu_row("strategy", true, &names, 20);
        assert!(none.is_none());
    }

    #[test]
    fn argument_candidates_carry_no_sentence() {
        let pool = vec!["vim".to_string(), "vscode".to_string()];
        assert_eq!(
            command_menu_row("vim", false, &pool, 120),
            ("vim".to_string(), None)
        );
    }

    #[test]
    fn command_help_is_generated_from_the_command_table() {
        let help = build_command_help();
        // if `strategy`'s entry in `COMMANDS` ever changes, this row (built
        // from the table, not hand-copied) changes with it, and this breaks
        let strategy = COMMANDS.iter().find(|c| c.name == "strategy").unwrap();
        let row = help
            .iter()
            .find(|l| l.contains(strategy.help))
            .expect("strategy row present");
        assert!(
            row.contains(":strategy"),
            "expected the command name in the row: {row}"
        );
    }

    #[test]
    fn every_command_has_a_non_empty_help_line() {
        for c in COMMANDS {
            assert!(!c.help.trim().is_empty(), ":{} has no help text", c.name);
        }
    }

    // ---- command mode: filter view + index clamping ----

    #[test]
    fn hidden_breakdown_partitions_the_hidden_set_exactly() {
        let mut noisy = test_item("src/a.rs");
        noisy.noise = true;
        let mut both = test_item("tests/b.rs"); // noise AND outside the glob
        both.noise = true;
        let outside = test_item("tests/c.rs");
        let shown = test_item("src/d.rs");
        let items = vec![noisy, both, outside, shown];
        let globs = build_globs(&["src/*".to_string()]).unwrap();

        let h = hidden_breakdown(&items, false, false, Some(&globs));
        // an item hidden twice is charged once, to the first reason
        assert_eq!(
            h,
            Hidden {
                comment: 0,
                noise: 2,
                glob: 1,
                unaccounted: 0
            }
        );
        assert_eq!(
            compute_view(&items, false, false, Some(&globs)).len() + h.noise + h.glob,
            items.len()
        );
    }

    #[test]
    fn hidden_breakdown_charges_only_comments_before_noise() {
        let mut a = test_item("a.rs");
        a.comment = true;
        let mut b = test_item("b.rs");
        b.noise = true; // hidden by only-comments first, not by noise
        let items = vec![a, b];

        let h = hidden_breakdown(&items, true, false, None);
        assert_eq!(
            h,
            Hidden {
                comment: 1,
                noise: 0,
                glob: 0,
                unaccounted: 0
            }
        );
    }

    #[test]
    fn build_audit_reports_every_reason_and_flags_an_unaccounted_remainder() {
        let items = vec![test_item("a.rs"), test_item("b.rs"), test_item("c.rs")];
        let ledger = Ledger {
            files_seen: 9,
            files_generated: 2,
            files_declared: 1,
            files_globbed: 3,
            files_unreadable: 1,
            hunks_import: 4,
            hunks_non_comment: 0,
        };
        let clean = Hidden {
            comment: 0,
            noise: 1,
            glob: 0,
            unaccounted: 0,
        };
        let text = build_audit(&items, 2, &clean, &ledger, None).join("\n");
        assert!(text.contains("2 of 3 hunks shown"), "{text}");
        assert!(text.contains("   4  of which import hunks"), "{text}");
        assert!(text.contains("files: 9 changed"), "{text}");
        assert!(
            text.contains("never fetched: excluded by a launch-time glob"),
            "{text}"
        );
        assert!(
            text.contains("every hidden hunk is accounted for"),
            "{text}"
        );

        let leak = Hidden {
            comment: 0,
            noise: 0,
            glob: 0,
            unaccounted: 1,
        };
        let text = build_audit(&items, 2, &leak, &ledger, Some("src/*")).join("\n");
        assert!(text.contains("1 hidden hunk unaccounted for"), "{text}");
        assert!(text.contains("outside the path filter 'src/*'"), "{text}");
    }

    #[test]
    fn compute_view_applies_only_comments_show_all_and_glob_independently() {
        let mut a = test_item("src/a.rs");
        a.comment = true;
        let mut b = test_item("src/b.rs");
        b.noise = true;
        let c = test_item("tests/c.rs");
        let items = vec![a, b, c];

        assert_eq!(compute_view(&items, false, true, None), vec![0, 1, 2]);
        assert_eq!(compute_view(&items, true, true, None), vec![0]); // only-comments
        assert_eq!(compute_view(&items, false, false, None), vec![0, 2]); // hide noise

        let globs = build_globs(&["src/*".to_string()]).unwrap();
        assert_eq!(compute_view(&items, false, true, Some(&globs)), vec![0, 1]);
    }

    #[test]
    fn set_filters_clamps_selection_off_a_hunk_that_falls_out_of_view() {
        let mut app = test_app(0);
        app.items = vec![test_item("a.rs"), test_item("b.rs"), test_item("c.rs")];
        app.view = vec![0, 1, 2];
        app.reviewed = vec![false, false, false];
        app.sel = 1; // b.rs — about to be filtered out
        app.popup = Some(Popup {
            title: "x".to_string(),
            lines: vec![],
            scroll: 0,
            hscroll: 0,
        });

        let globs = build_globs(&["a.rs".to_string()]).unwrap();
        set_filters(&mut app, false, true, Some(("a.rs".to_string(), globs))).unwrap();

        assert_eq!(app.view, vec![0]);
        assert_eq!(app.sel, 0, "selection must move off the now-hidden item");
        assert!(app.popup.is_none(), "select() clears a stale popup");
    }

    #[test]
    fn set_filters_rejects_a_combination_that_would_empty_the_view() {
        let mut app = test_app(0);
        app.items = vec![test_item("a.rs")];
        app.view = vec![0];
        app.reviewed = vec![false];

        let globs = build_globs(&["nope/*".to_string()]).unwrap();
        let err = set_filters(&mut app, false, true, Some(("nope/*".to_string(), globs)));
        assert!(err.is_err());
        // rejected: state is untouched, still pointing at the one real item
        assert_eq!(app.view, vec![0]);
        assert!(app.path_filter.is_none());
    }

    #[test]
    fn view_pos_and_first_last_visible_read_the_view_not_the_raw_item_list() {
        assert_eq!(view_pos(&[3, 5, 8], 5), 1);
        assert_eq!(view_pos(&[3, 5, 8], 99), 0); // not found: degrades to 0
        let mut app = test_app(0);
        app.view = vec![3, 5, 8];
        assert_eq!(first_visible(&app), 3);
        assert_eq!(last_visible(&app), 8);
    }

    // ---- horizontal scroll: cursor-follow ----

    #[test]
    fn follow_hscroll_keeps_cursor_in_view() {
        assert_eq!(follow_hscroll(5, 0, 10), 0); // already visible: no movement
        assert_eq!(follow_hscroll(15, 0, 10), 6); // right of the window: scrolls right
        assert_eq!(follow_hscroll(2, 6, 10), 2); // left of the window: scrolls back
    }

    // ---- horizontal scroll: span slicing ----

    #[test]
    fn slice_range_crops_by_char_column_not_byte() {
        // "héllo world" — é is 2 bytes, so a byte-based slice would misalign
        // every column after it.
        let spans = vec![Span::styled("héllo world".to_string(), Style::default())];
        let sliced = slice_range(spans, 1, 3);
        let text: String = sliced.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "éll");
    }

    #[test]
    fn slice_range_drops_content_outside_the_window() {
        let spans = vec![Span::styled("hello".to_string(), Style::default())];
        assert!(slice_range(spans.clone(), 10, 5).is_empty());
        let sliced = slice_range(spans, 0, 2);
        let text: String = sliced.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "he");
    }

    // ---- horizontal scroll: the gutter stays fixed ----

    fn test_item(path: &str) -> Item {
        Item {
            bucket: "g0".to_string(),
            ledger: None,
            path: path.to_string(),
            old_range: [0, 0],
            new_range: [0, 0],
            mark: String::new(),
            cat: "other".to_string(),
            rationale: String::new(),
            details: vec![],
            notes: vec![],
            edges: vec![],
            advisories: vec![],
            noise: false,
            comment: false,
            symbols: vec![],
            enclosing: None,
            group: String::new(),
            rules: vec![],
            refined: ordo::refine::Refined::default(),
            cluster: None,
        }
    }

    fn edge(label: &str, target: Option<usize>) -> EdgeRef {
        EdgeRef {
            label: label.to_string(),
            target,
            // `←` is the direction that means "this hunk uses what the target
            // defines", which is what makes the target a dependency
            dependency: label.starts_with('←'),
        }
    }

    #[test]
    fn code_view_horizontal_scroll_keeps_gutter_fixed_and_handles_multibyte() {
        let it = test_item("f.rs");
        // a multi-byte line, long enough to be clipped on the right at a
        // narrow width
        let line = "let héllo_world = 1234567890abcdef;".to_string();
        let mut sources: Sources = HashMap::new();
        sources.insert("f.rs".to_string(), (vec![], vec![line]));
        let highlights: Highlights = HashMap::new();
        // width = gutter (6) + 10 cols of code
        let (unscrolled, right_clip_0, _) = code_view(
            &it,
            &sources,
            &highlights,
            16,
            0,
            None,
            &[],
            None,
            &theme("dark").unwrap(),
            0,
            usize::MAX,
        );
        let (scrolled, right_clip_5, _) = code_view(
            &it,
            &sources,
            &highlights,
            16,
            5,
            None,
            &[],
            None,
            &theme("dark").unwrap(),
            0,
            usize::MAX,
        );
        assert_eq!(unscrolled.len(), 1);
        assert_eq!(scrolled.len(), 1);
        // the sign-bar and line-number gutter (the row's first two spans)
        // never move, regardless of horizontal scroll
        let gutter = |line: &Line<'static>| -> Vec<String> {
            line.spans
                .iter()
                .take(2)
                .map(|s| s.content.to_string())
                .collect()
        };
        assert_eq!(gutter(&unscrolled[0]), gutter(&scrolled[0]));
        // but the code past the gutter does shift with hscroll
        let rest = |line: &Line<'static>| -> String {
            line.spans[2..]
                .iter()
                .map(|s| s.content.to_string())
                .collect()
        };
        assert_ne!(rest(&unscrolled[0]), rest(&scrolled[0]));
        // both directions were clipped at this width, so both report it
        assert!(right_clip_0);
        assert!(right_clip_5);
    }

    #[test]
    fn code_view_tints_only_the_refined_span_of_a_paired_line() {
        let old = "fn f(a: A) {}".to_string();
        let new = "fn f(a: A, b: B) {}".to_string();
        let mut it = test_item("f.rs");
        it.old_range = [1, 1];
        it.new_range = [1, 1];
        let mut sources: Sources = HashMap::new();
        sources.insert("f.rs".to_string(), (vec![old.clone()], vec![new.clone()]));
        let mut items = vec![it];
        refine_items(&mut items, &sources);
        let it = &items[0];
        assert_eq!(
            it.refined.added[0],
            Some(vec![(9, 15)]),
            "expected only `, b: B` to be refined"
        );

        let theme = theme("dark").unwrap();
        let (rows, _, _) = code_view(
            it,
            &sources,
            &HashMap::new(),
            60,
            0,
            None,
            &[],
            None,
            &theme,
            0,
            usize::MAX,
        );
        // the added row is the one carrying the add tint (the removed row
        // comes first, on the del tint)
        let added = rows.last().expect("an added row");
        // walk the row char by char: the strong tint covers `, b: B` and
        // nothing else
        let mut strong = String::new();
        for span in &added.spans {
            if span.style.bg == Some(theme.add_strong_bg) {
                strong.push_str(&span.content);
            }
        }
        assert_eq!(strong, ", b: B");
    }

    #[test]
    fn code_view_tints_a_whole_unpaired_line() {
        // nothing in common with the removed line, so no span is singled out
        let mut it = test_item("f.rs");
        it.old_range = [1, 1];
        it.new_range = [1, 1];
        let mut sources: Sources = HashMap::new();
        sources.insert(
            "f.rs".to_string(),
            (
                vec!["use std::io;".to_string()],
                vec!["fn totally(different: X) {}".to_string()],
            ),
        );
        let mut items = vec![it];
        refine_items(&mut items, &sources);
        assert_eq!(items[0].refined.added[0], None);

        let theme = theme("dark").unwrap();
        let (rows, _, _) = code_view(
            &items[0],
            &sources,
            &HashMap::new(),
            60,
            0,
            None,
            &[],
            None,
            &theme,
            0,
            usize::MAX,
        );
        let added = rows.last().unwrap();
        assert!(
            added
                .spans
                .iter()
                .all(|s| s.style.bg != Some(theme.add_strong_bg)),
            "an unpaired line must not be partially tinted"
        );
    }

    #[test]
    fn code_view_reports_no_clipping_when_the_line_fits() {
        let it = test_item("f.rs");
        let mut sources: Sources = HashMap::new();
        sources.insert("f.rs".to_string(), (vec![], vec!["short".to_string()]));
        let highlights: Highlights = HashMap::new();
        let (_, right_clip, _) = code_view(
            &it,
            &sources,
            &highlights,
            40,
            0,
            None,
            &[],
            None,
            &theme("dark").unwrap(),
            0,
            usize::MAX,
        );
        assert!(!right_clip);
    }

    /// The viewport slice must be exactly the window it replaces: whatever
    /// `code_view` builds for `(start, rows)` has to equal that range of the
    /// full view, and `total` has to stay the full view's length whatever
    /// window is asked for. The removed block shifts every row after it, so
    /// this is checked with the deletion mid-file and again at EOF.
    #[test]
    fn code_view_window_matches_the_same_slice_of_the_whole_view() {
        let theme = theme("dark").unwrap();
        let old: Vec<String> = (1..=4).map(|i| format!("gone {i}")).collect();
        let new: Vec<String> = (1..=30).map(|i| format!("fn line_{i}() {{}}")).collect();

        for (label, new_range) in [("mid-file", [10, 12]), ("at EOF", [31, 33])] {
            let mut it = test_item("f.rs");
            it.old_range = [1, 4];
            it.new_range = new_range;
            let mut sources: Sources = HashMap::new();
            sources.insert("f.rs".to_string(), (old.clone(), new.clone()));
            let highlights: Highlights = HashMap::new();

            let call = |start: usize, rows: usize| {
                code_view(
                    &it,
                    &sources,
                    &highlights,
                    60,
                    0,
                    None,
                    &[],
                    None,
                    &theme,
                    start,
                    rows,
                )
            };
            let (full, clip_full, total) = call(0, usize::MAX);
            assert_eq!(total, new.len() + old.len(), "{label}: total row count");
            assert_eq!(full.len(), total, "{label}: unwindowed view is complete");

            for start in 0..total {
                let (win, clip, t) = call(start, 7);
                assert_eq!(t, total, "{label}: total is window-independent");
                assert_eq!(clip, clip_full, "{label}: clipping is window-independent");
                let want = &full[start..(start + 7).min(total)];
                assert_eq!(win.len(), want.len(), "{label}: window length at {start}");
                for (a, b) in win.iter().zip(want) {
                    assert_eq!(spans_of(a), spans_of(b), "{label}: row {start} content");
                }
            }
        }
    }

    fn spans_of(l: &Line<'static>) -> Vec<String> {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    // ---- def→use edges: target resolution ----

    fn id_hunk(id: &str) -> HunkOut {
        let mut h = test_hunk("does a thing", None, vec![]);
        h.id = id.to_string();
        h
    }

    fn test_file_out(path: &str, hunks: Vec<HunkOut>) -> ordo::model::FileOut {
        ordo::model::FileOut {
            path: path.to_string(),
            hunks,
            degraded: false,
            unsupported: false,
            dropped: vec![],
        }
    }

    #[test]
    fn build_items_resolves_edge_targets_within_the_review_and_flags_ones_outside_it() {
        let out = Output {
            schema: 1,
            order: vec![
                ordo::model::OrderItem {
                    path: "a.rs".to_string(),
                    hunk: "h1".to_string(),
                },
                ordo::model::OrderItem {
                    path: "a.rs".to_string(),
                    hunk: "h2".to_string(),
                },
            ],
            files: vec![test_file_out("a.rs", vec![id_hunk("h1"), id_hunk("h2")])],
            groups: vec![],
            edges: vec![
                // resolves: h2 is item index 1
                ordo::model::Edge {
                    from: "h1".to_string(),
                    to: "h2".to_string(),
                    why: "uses it".to_string(),
                },
                // doesn't resolve: "ghost" was never built into an item (e.g.
                // filtered out, or a cross-file edge to an unreviewed file)
                ordo::model::Edge {
                    from: "h1".to_string(),
                    to: "ghost".to_string(),
                    why: "calls it".to_string(),
                },
            ],
            clusters: vec![],
            problems: vec![],
            notes: vec![],
            ledger: vec![],
        };
        let items = build_items(&out);
        assert_eq!(items.len(), 2);
        assert!(items[0].edges.iter().any(|e| e.target == Some(1)));
        assert!(items[0].edges.iter().any(|e| e.target.is_none()));
    }

    // ---- position stack ----

    #[test]
    fn stack_pop_on_an_empty_stack_is_a_no_op() {
        let mut stack: Vec<(usize, Cursor)> = vec![];
        assert_eq!(stack_pop_valid(&mut stack, 10), None);
        assert!(stack.is_empty());
    }

    #[test]
    fn stack_push_then_pop_returns_you_to_the_pushed_position() {
        let mut stack: Vec<(usize, Cursor)> = vec![];
        let pos = (2, Cursor { line: 5, col: 1 });
        stack_push(&mut stack, pos);
        assert_eq!(stack_pop_valid(&mut stack, 10), Some(pos));
        assert!(stack.is_empty());
    }

    #[test]
    fn stack_push_is_bounded_dropping_the_oldest_entry_first() {
        let mut stack: Vec<(usize, Cursor)> = vec![];
        for i in 0..JUMP_STACK_CAP + 10 {
            stack_push(&mut stack, (i, Cursor { line: i, col: 0 }));
        }
        assert_eq!(stack.len(), JUMP_STACK_CAP);
        // the oldest 10 pushes were dropped to keep the cap
        assert_eq!(stack.first().unwrap().0, 10);
        assert_eq!(stack.last().unwrap().0, JUMP_STACK_CAP + 9);
    }

    #[test]
    fn stack_pop_valid_skips_an_entry_whose_index_no_longer_resolves() {
        // "99" is stale (out of range for a 5-item review) and sits on top —
        // it must be skipped, not returned, and the stack must not panic.
        let mut stack = vec![
            (0, Cursor { line: 1, col: 1 }),
            (99, Cursor { line: 0, col: 0 }),
        ];
        assert_eq!(
            stack_pop_valid(&mut stack, 5),
            Some((0, Cursor { line: 1, col: 1 }))
        );
        assert!(stack.is_empty());
    }

    // ---- why pane: dep-line resolution and cursor ----

    #[test]
    fn why_rows_marks_edge_lines_and_carries_their_target() {
        let mut it = test_item("a.rs");
        it.edges = vec![
            edge("→ a.rs:L10   uses it", Some(3)),
            edge("→ b.rs:L1   calls it", None),
        ];
        // target 3 must be in `view` to resolve — same as being part of the
        // review at all; a 4-item view (0..=3) covers it here
        let rows = why_rows(
            &it,
            &[0, 1, 2, 3],
            &Theme::terminal("dark", false),
            None,
            &[],
            None,
            None,
        );
        let edges: Vec<&WhyKind> = rows
            .iter()
            .map(|r| &r.kind)
            .filter(|k| matches!(k, WhyKind::Edge(_)))
            .collect();
        assert!(matches!(edges[0], WhyKind::Edge(Some(3))));
        assert!(matches!(edges[1], WhyKind::Edge(None)));
    }

    #[test]
    fn why_rows_treats_a_filtered_out_target_as_not_part_of_the_review() {
        let mut it = test_item("a.rs");
        it.edges = vec![edge("→ a.rs:L10   uses it", Some(3))];
        // target 3 exists (it's a valid item index) but isn't in `view`
        let rows = why_rows(
            &it,
            &[0, 1, 2],
            &Theme::terminal("dark", false),
            None,
            &[],
            None,
            None,
        );
        let edges: Vec<&WhyKind> = rows
            .iter()
            .map(|r| &r.kind)
            .filter(|k| matches!(k, WhyKind::Edge(_)))
            .collect();
        assert!(matches!(edges[0], WhyKind::Edge(None)));
    }

    fn test_app(why_len: usize) -> App {
        App {
            items: vec![test_item("a.rs")],
            reviewed: vec![false],
            view: vec![0],
            comments_only: false,
            show_all: true,
            path_filter: None,
            command: None,
            sel: 0,
            scroll: 0,
            hscroll: 0,
            why_scroll: 0,
            why_sel: 0,
            code_len: 0,
            why_len,
            code_height: 10,
            why_height: 5,
            code_width: 10,
            focus: Pane::Why,
            keys: keymap("vim").unwrap(),
            pending: None,
            sources: HashMap::new(),
            highlights: HashMap::new(),
            cursor: Cursor { line: 0, col: 0 },
            popup: None,
            trees: HashMap::new(),
            prompt: None,
            search: None,
            review_sha: None,
            uncommitted: false,
            history_cache: HashMap::new(),
            jumps: vec![],
            rev: "HEAD".to_string(),
            marks_path: None,
            marks: HashMap::new(),
            theme: theme("dark").unwrap(),
            show_groups: false,
            groups: HashMap::new(),
            group_reasons: HashMap::new(),
            mode: ViewMode::Hunks,
            symbol_ledger: vec![],
            notes: HashMap::new(),
            notes_path: None,
            deltas: vec![],
            delta_gone: 0,
            collapsed: HashSet::new(),
            ledger: Ledger::default(),
            rules: vec![],
            strategy: "comprehension".to_string(),
            rules_report: vec![],
            max_col: HashMap::new(),
        }
    }

    #[test]
    fn the_list_defaults_to_symbols_and_mode_switches_it_back() {
        let mut app = test_app(1);
        app.items = vec![grouped_item("a.rs", "g0"), grouped_item("b.rs", "g1")];
        app.items[0].ledger = Some(0);
        let led = vec![ordo::model::LedgerEntry {
            name: "fetch".to_string(),
            kind: Some("function_definition".to_string()),
            scope: None,
            path: "a.rs".to_string(),
            at: "h0".to_string(),
            change: ordo::model::SymbolChange::Signature,
            from: None,
            used_by: vec!["h1".to_string()],
        }];

        set_mode(&mut app, ViewMode::Ledger, &led);
        // the symbol with a ledger entry buckets under it; the one without
        // falls into the shared bucket rather than vanishing
        assert_eq!(app.items[0].bucket, "L0");
        assert_eq!(app.items[1].bucket, "L-");
        // headers *are* the ledger in this mode, so they are forced on
        assert!(app.show_groups);
        assert_eq!(
            app.groups.get("L0").map(String::as_str),
            Some("fetch — signature, used by 1 hunk")
        );
        assert_eq!(
            app.groups.get("L-").map(String::as_str),
            Some("no symbol changed")
        );

        set_mode(&mut app, ViewMode::Hunks, &led);
        assert_eq!(app.items[0].bucket, "g0");
        assert_eq!(app.items[1].bucket, "g1");
    }

    #[test]
    fn switching_mode_drops_fold_state_keyed_to_the_old_one() {
        let mut app = test_app(1);
        app.items = vec![grouped_item("a.rs", "g0")];
        app.collapsed.insert("g0".to_string());
        set_mode(&mut app, ViewMode::Ledger, &[]);
        // "g0" means nothing in ledger mode; keeping it would fold a bucket
        // that no longer exists
        assert!(app.collapsed.is_empty());
    }

    fn item_with_symbol(path: &str, name: &str) -> Item {
        let mut it = test_item(path);
        it.symbols = vec![sym(name, "function_definition", None)];
        it
    }

    /// Two items where 1 uses what 0 defines: `1 ← 0`.
    fn dependent_pair() -> App {
        let mut app = test_app(1);
        app.items = vec![test_item("a.py"), test_item("b.py")];
        app.items[1].edges = vec![edge("← a.py:L1   def→use: f", Some(0))];
        app.items[0].edges = vec![edge("→ b.py:L1   def→use: f", Some(1))];
        app.view = vec![0, 1];
        app.reviewed = vec![false, false];
        app
    }

    fn snaps_of(app: &App, sources: &Sources) -> HashMap<String, Snap> {
        let order: Vec<usize> = (0..app.items.len()).collect();
        compare_runs(&app.items, &order, sources, &HashMap::new()).0
    }

    /// 0 defines what 1 uses; 1 defines what 2 uses. Rejecting 0 strands both.
    fn dependency_chain() -> App {
        let mut app = test_app(1);
        app.items = vec![test_item("a.py"), test_item("b.py"), test_item("c.py")];
        app.items[1].edges = vec![edge("← a.py:L1   def→use: f", Some(0))];
        app.items[2].edges = vec![edge("← b.py:L1   def→use: g", Some(1))];
        app.view = vec![0, 1, 2];
        app.reviewed = vec![false, false, false];
        app
    }

    #[test]
    fn a_draft_rule_states_the_hunk_it_came_from() {
        let mut app = test_app(1);
        app.items = vec![item_with_symbol("a.py", "fetch")];
        app.items[0].notes = vec!["7 params".to_string()];
        let d = draft_rule(&app, 0).join("\n");
        assert!(d.contains("[[rule]]"), "{d}");
        assert!(d.contains("lang = \"python\""), "{d}");
        assert!(d.contains("kind = [\"function_definition\"]"), "{d}");
        // one below what this hunk measured, so the rule fires on it
        assert!(d.contains("max-params = 6"), "{d}");
        assert!(d.contains("warn ="), "{d}");
    }

    #[test]
    fn a_draft_turns_each_structural_note_into_its_limit() {
        let mut app = test_app(1);
        app.items = vec![item_with_symbol("a.py", "f")];
        app.items[0].notes = vec![
            "large definition (120 lines)".to_string(),
            "deeply nested (depth 4)".to_string(),
        ];
        let d = draft_rule(&app, 0).join("\n");
        assert!(d.contains("max-lines = 119"), "{d}");
        assert!(d.contains("max-nesting = 3"), "{d}");
    }

    #[test]
    fn a_draft_from_a_hunk_with_no_symbol_says_it_is_broad() {
        let mut app = test_app(1);
        app.items = vec![test_item("a.py")];
        let d = draft_rule(&app, 0).join("\n");
        assert!(!d.contains("kind = ["), "{d}");
        assert!(d.contains("no symbol"), "{d}");
    }

    #[test]
    fn a_cascade_is_transitive_not_just_the_direct_dependents() {
        // pushing back on a leaf when the root is the problem sends the author
        // round the loop twice
        let app = dependency_chain();
        assert_eq!(cascade(&app, 0), vec![1, 2]);
        assert_eq!(cascade(&app, 1), vec![2]);
        assert!(cascade(&app, 2).is_empty());
    }

    #[test]
    fn a_cascade_terminates_on_a_dependency_cycle() {
        // mutual recursion is a real def→use cycle
        let mut app = dependency_chain();
        app.items[0].edges = vec![edge("← c.py:L1   def→use: h", Some(2))];
        let hit = cascade(&app, 0);
        assert_eq!(hit, vec![1, 2], "{hit:?}");
    }

    #[test]
    fn a_cascade_ignores_hunks_filtered_out_of_the_view() {
        let mut app = dependency_chain();
        app.view = vec![0, 1];
        assert_eq!(cascade(&app, 0), vec![1]);
    }

    #[test]
    fn a_leaf_strands_nothing_and_says_nothing() {
        let app = dependency_chain();
        assert!(cascade_line(&app, 2).is_none());
        assert!(cascade_line(&app, 0).is_some());
    }

    #[test]
    fn a_hunk_that_only_moved_in_the_reading_order_is_reported_as_such() {
        // the case no other tool reports: byte-identical, but it reads
        // somewhere else now because what it depends on changed
        let app = dependent_pair();
        let sources = Sources::new();
        let mut prev = snaps_of(&app, &sources);
        // same content, different position last time
        for v in prev.values_mut() {
            v.p += 5;
        }
        let order: Vec<usize> = (0..app.items.len()).collect();
        let (_, deltas) = compare_runs(&app.items, &order, &sources, &prev);
        assert!(deltas.iter().all(|d| *d == Delta::Moved), "{deltas:?}");
    }

    #[test]
    fn a_run_identical_to_the_last_one_reports_nothing() {
        let app = dependent_pair();
        let sources = Sources::new();
        let prev = snaps_of(&app, &sources);
        let order: Vec<usize> = (0..app.items.len()).collect();
        let (_, deltas) = compare_runs(&app.items, &order, &sources, &prev);
        assert!(deltas.iter().all(|d| *d == Delta::Same), "{deltas:?}");
    }

    #[test]
    fn a_hunk_with_no_previous_snapshot_is_new() {
        let app = dependent_pair();
        let sources = Sources::new();
        let order: Vec<usize> = (0..app.items.len()).collect();
        let (_, deltas) = compare_runs(&app.items, &order, &sources, &HashMap::new());
        assert!(deltas.iter().all(|d| *d == Delta::New), "{deltas:?}");
    }

    #[test]
    fn a_first_run_says_nothing_rather_than_calling_everything_new() {
        let mut app = dependent_pair();
        app.deltas = vec![Delta::New, Delta::New];
        assert!(delta_line(&app, 0).is_none());
        // but once there is a real comparison, New is worth saying
        app.deltas = vec![Delta::New, Delta::Same];
        assert!(delta_line(&app, 0).is_some());
    }

    #[test]
    fn a_changed_dependency_set_counts_as_moved() {
        let app = dependent_pair();
        let sources = Sources::new();
        let mut prev = snaps_of(&app, &sources);
        for v in prev.values_mut() {
            v.d = vec!["something-else".to_string()];
        }
        let order: Vec<usize> = (0..app.items.len()).collect();
        let (_, deltas) = compare_runs(&app.items, &order, &sources, &prev);
        assert!(deltas.contains(&Delta::Moved), "{deltas:?}");
    }

    #[test]
    fn approving_a_use_before_its_definition_is_reported() {
        let mut app = dependent_pair();
        app.reviewed[1] = true; // the caller, not the callee
        let out = out_of_order_labels(&app, 1);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].starts_with("a.py:"), "{out:?}");
    }

    #[test]
    fn reviewing_the_definition_first_says_nothing() {
        let mut app = dependent_pair();
        app.reviewed[0] = true;
        app.reviewed[1] = true;
        assert!(out_of_order_labels(&app, 1).is_empty());
    }

    #[test]
    fn an_unreviewed_hunk_is_not_out_of_order() {
        // the warning is about the order things were approved in, not about
        // work still to do
        let app = dependent_pair();
        assert!(out_of_order_labels(&app, 1).is_empty());
    }

    #[test]
    fn edge_coverage_needs_both_ends_reviewed() {
        let mut app = dependent_pair();
        assert_eq!(coverage(&app), (0, 2, 0, 1));
        app.reviewed[1] = true;
        // one hunk done, but the link between them is still unchecked
        assert_eq!(coverage(&app), (1, 2, 0, 1));
        app.reviewed[0] = true;
        assert_eq!(coverage(&app), (2, 2, 1, 1));
    }

    #[test]
    fn an_edge_leaving_the_view_is_not_counted_against_it() {
        // filtering the review must not make coverage look better than it is
        let mut app = dependent_pair();
        app.view = vec![1];
        let (_, _, _, edges) = coverage(&app);
        assert_eq!(edges, 0);
    }

    #[test]
    fn a_note_key_ignores_everything_a_rebase_can_move() {
        // same symbol, different file, different lines, different content —
        // a line anchor would be lost, the note must not be
        let mut a = item_with_symbol("api.py", "fetch");
        a.new_range = [10, 12];
        let mut b = item_with_symbol("moved/elsewhere.py", "fetch");
        b.new_range = [900, 902];
        b.rationale = "totally different".to_string();
        assert_eq!(note_key(&a), note_key(&b));
    }

    #[test]
    fn a_note_key_separates_two_symbols_that_share_a_name() {
        // name alone is not identity: kind and scope are part of it
        let mut a = item_with_symbol("a.py", "run");
        a.symbols = vec![sym("run", "function_definition", None)];
        let mut b = item_with_symbol("a.py", "run");
        b.symbols = vec![sym("run", "function_definition", Some("Worker"))];
        assert_ne!(note_key(&a), note_key(&b));
    }

    #[test]
    fn a_hunk_with_no_symbol_cannot_be_anchored_to() {
        // anchoring to the enclosing name would silently drift
        let it = test_item("a.py");
        assert!(it.symbols.is_empty());
        assert!(note_key(&it).is_none());
    }

    #[test]
    fn a_rename_maps_the_old_identity_onto_the_new_one() {
        // what `load` uses to carry a note across `renames parse_cfg → load_cfg`
        let new = item_with_symbol("cfg.py", "load_cfg");
        let old = item_with_symbol("cfg.py", "parse_cfg");
        assert_eq!(note_key_named(&new, "parse_cfg"), note_key(&old));
        assert_ne!(note_key_named(&new, "parse_cfg"), note_key(&new));
    }

    #[test]
    fn notes_round_trip_through_the_cache_file() {
        let dir = std::env::temp_dir().join(format!("ordo-notes-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("n.json");
        let mut notes = HashMap::new();
        notes.insert(42u64, "check the retry path".to_string());
        save_notes(&path, &notes);
        assert_eq!(load_notes(&path), notes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_note_file_is_not_an_error() {
        let missing = std::env::temp_dir().join("ordo-notes-does-not-exist.json");
        assert!(load_notes(&missing).is_empty());
    }

    #[test]
    fn why_cursor_move_clamps_at_both_ends() {
        let mut app = test_app(3); // 3 logical why-pane lines: indices 0..=2
        why_cursor_move(&mut app, -1);
        assert_eq!(app.why_sel, 0, "can't move above the first line");
        why_cursor_move(&mut app, 1);
        assert_eq!(app.why_sel, 1);
        why_cursor_move(&mut app, 10);
        assert_eq!(app.why_sel, 2, "clamps at the last line");
        why_cursor_move(&mut app, 10);
        assert_eq!(app.why_sel, 2, "stays clamped past the last line");
    }

    // ---- reviewed-mark persistence ----

    // A known FNV-1a 64-bit test vector, hard-coded so a future refactor that
    // swaps in a different hash (or a different offset/prime) fails loudly
    // instead of silently invalidating every mark ever written.
    #[test]
    fn fnv1a_is_stable_for_a_known_input() {
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a(b"hello"), 0xa430d84680aabd0b);
    }

    fn item_with(
        path: &str,
        old: [usize; 2],
        new: [usize; 2],
        symbols: Vec<Symbol>,
        enclosing: Option<&str>,
    ) -> Item {
        let mut it = test_item(path);
        it.old_range = old;
        it.new_range = new;
        it.symbols = symbols;
        it.enclosing = enclosing.map(str::to_string);
        it
    }

    fn sources_for(path: &str, old: &[&str], new: &[&str]) -> Sources {
        let mut s: Sources = HashMap::new();
        s.insert(path.to_string(), (lines(old), lines(new)));
        s
    }

    #[test]
    fn mark_key_changes_when_hunk_content_changes() {
        let it_a = item_with(
            "f.rs",
            [1, 1],
            [1, 1],
            vec![sym("run", "function_item", None)],
            None,
        );
        let src_a = sources_for("f.rs", &["fn run() {}"], &["fn run() { 1 }"]);
        let src_b = sources_for("f.rs", &["fn run() {}"], &["fn run() { 2 }"]);
        let ka = mark_key("HEAD", &it_a, &src_a).unwrap();
        let kb = mark_key("HEAD", &it_a, &src_b).unwrap();
        assert_ne!(
            ka, kb,
            "a body edit must drop the mark, never carry it over silently"
        );
    }

    #[test]
    fn mark_key_covers_both_old_and_new_sides() {
        // same new content, different old content (e.g. a reformat that
        // happens to converge) — still a different key
        let it = item_with("f.rs", [1, 1], [1, 1], vec![], Some("run"));
        let src_a = sources_for("f.rs", &["fn run() { 1 }"], &["fn run() {\n    1\n}"]);
        let src_b = sources_for("f.rs", &["fn run() { 2 }"], &["fn run() {\n    1\n}"]);
        assert_ne!(
            mark_key("HEAD", &it, &src_a).unwrap(),
            mark_key("HEAD", &it, &src_b).unwrap()
        );
    }

    #[test]
    fn mark_key_is_unchanged_by_reordering_symbols() {
        let syms_a = vec![
            sym("a", "function_item", None),
            sym("b", "function_item", None),
        ];
        let syms_b = vec![
            sym("b", "function_item", None),
            sym("a", "function_item", None),
        ];
        let it_a = item_with("f.rs", [1, 1], [1, 2], syms_a, None);
        let it_b = item_with("f.rs", [1, 1], [1, 2], syms_b, None);
        let src = sources_for("f.rs", &["old"], &["fn a() {}", "fn b() {}"]);
        assert_eq!(
            mark_key("HEAD", &it_a, &src).unwrap(),
            mark_key("HEAD", &it_b, &src).unwrap()
        );
    }

    #[test]
    fn mark_key_falls_back_to_enclosing_when_no_symbols() {
        // a body-edit hunk (no `symbols`) still gets a key, via `enclosing`
        let it = item_with("f.rs", [1, 1], [1, 1], vec![], Some("A.run"));
        let src = sources_for("f.rs", &["old"], &["new"]);
        assert!(mark_key("HEAD", &it, &src).is_some());
    }

    #[test]
    fn mark_key_none_without_source_content() {
        let it = item_with("f.rs", [1, 1], [1, 1], vec![], None);
        let empty: Sources = HashMap::new();
        assert!(mark_key("HEAD", &it, &empty).is_none());
    }

    #[test]
    fn mark_key_differs_across_rev_and_path() {
        let it = item_with(
            "f.rs",
            [1, 1],
            [1, 1],
            vec![sym("run", "function_item", None)],
            None,
        );
        let src = sources_for("f.rs", &["old"], &["new"]);
        let k1 = mark_key("HEAD", &it, &src).unwrap();
        let k2 = mark_key("abc123", &it, &src).unwrap();
        assert_ne!(k1, k2, "different revs must not collide");

        let it2 = item_with(
            "g.rs",
            [1, 1],
            [1, 1],
            vec![sym("run", "function_item", None)],
            None,
        );
        let mut src2 = src.clone();
        src2.insert("g.rs".to_string(), src2["f.rs"].clone());
        let k3 = mark_key("HEAD", &it2, &src2).unwrap();
        assert_ne!(
            k1, k3,
            "different paths must not collide even with identical content/symbol"
        );
    }

    #[test]
    fn prune_marks_drops_old_entries_and_keeps_recent() {
        let now = 1_000_000_000u64;
        let mut marks: HashMap<u64, u64> = HashMap::new();
        marks.insert(1, now); // just written
        marks.insert(2, now - MARK_TTL_SECS + 10); // just inside the window
        marks.insert(3, now - MARK_TTL_SECS - 10); // just outside — dropped
        marks.insert(4, 0); // ancient — dropped
        prune_marks(&mut marks, now);
        assert_eq!(marks.len(), 2);
        assert!(marks.contains_key(&1));
        assert!(marks.contains_key(&2));
        assert!(!marks.contains_key(&3));
        assert!(!marks.contains_key(&4));
    }

    #[test]
    fn load_marks_degrades_to_empty_on_a_missing_or_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("ordo-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let missing = dir.join("missing.json");
        assert!(load_marks(&missing).is_empty());

        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, b"not json").unwrap();
        assert!(load_marks(&corrupt).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_marks_then_load_marks_round_trips() {
        let dir = std::env::temp_dir().join(format!("ordo-test-roundtrip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("marks.json");
        let mut marks: HashMap<u64, u64> = HashMap::new();
        marks.insert(0xdeadbeef, 123);
        marks.insert(0x1, 456);
        save_marks(&path, &marks);
        let got = load_marks(&path);
        assert_eq!(got, marks);

        // the file on disk reveals nothing about the code under review — no
        // path, symbol name, or source text, only hex keys and timestamps
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        for (k, v) in obj {
            assert!(
                u64::from_str_radix(k, 16).is_ok(),
                "key must be plain hex: {k}"
            );
            assert!(v.is_u64(), "value must be a plain timestamp: {v}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- negative globs ----

    fn filt(pats: &[&str]) -> Filter {
        let globs = build_globs(&pats.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        Filter {
            globs,
            skip_generated: false,
            negatives_emptied: std::cell::Cell::new(false),
            tally: std::cell::Cell::new(Ledger::default()),
        }
    }

    #[test]
    fn positives_only_keep_only_matching_paths() {
        let f = filt(&["src/*"]);
        assert!(f.keep("src/a.rs"));
        assert!(!f.keep("tests/a.rs"));
    }

    #[test]
    fn negatives_only_keep_everything_except_those() {
        let f = filt(&["!tests/*"]);
        assert!(f.keep("src/a.rs"));
        assert!(f.keep("README.md"));
        assert!(!f.keep("tests/a.rs"));
    }

    #[test]
    fn positive_and_negative_combine_as_and() {
        let f = filt(&["src/*", "!src/generated/*"]);
        assert!(f.keep("src/a.rs"));
        assert!(!f.keep("src/generated/x.rs"));
        assert!(!f.keep("tests/a.rs"), "outside the positive set entirely");
    }

    #[test]
    fn a_negative_excludes_a_path_a_positive_also_matches_regardless_of_order() {
        // deliberately unlike .gitignore: order on the command line never
        // matters, and a later positive can never re-include what a negative
        // excluded
        let f = filt(&["!src/a.rs", "src/*"]);
        assert!(!f.keep("src/a.rs"));
        let f2 = filt(&["src/*", "!src/a.rs"]);
        assert!(!f2.keep("src/a.rs"));
    }

    #[test]
    fn escaped_bang_is_a_literal_positive_pattern() {
        let f = filt(&["\\!weird"]);
        assert!(f.keep("!weird"));
        assert!(!f.keep("weird"));
    }

    #[test]
    fn apply_note_distinguishes_negatives_emptying_it_from_no_positive_match() {
        let f = filt(&["!src/*"]);
        let kept = f.apply(vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
        assert!(kept.is_empty());
        assert_eq!(
            f.note(),
            " (every matching path was excluded by a negative glob)"
        );

        let f2 = filt(&["nomatch/*"]);
        let kept2 = f2.apply(vec!["src/a.rs".to_string()]);
        assert!(kept2.is_empty());
        assert_eq!(f2.note(), " matching the given globs");
    }

    #[test]
    fn filter_command_glob_type_supports_negatives_too() {
        // `:filter` reuses `build_globs`, so a single negative pattern like
        // `!tests/*` narrows to "everything except tests" the same as the CLI
        let globs = build_globs(&["!tests/*".to_string()]).unwrap();
        let mut a = test_item("src/a.rs");
        a.comment = false;
        let items = vec![test_item("src/a.rs"), test_item("tests/b.rs")];
        assert_eq!(compute_view(&items, false, true, Some(&globs)), vec![0]);
    }

    // ---- theme selection ----

    #[test]
    fn theme_selects_dark_and_light_by_name() {
        assert!(theme("dark").is_some());
        assert!(theme("light").is_some());
    }

    #[test]
    fn theme_rejects_an_unknown_name() {
        assert!(theme("nonsense").is_none());
    }

    #[test]
    fn light_theme_is_not_the_dark_values_inverted() {
        let dark = theme("dark").unwrap();
        let light = theme("light").unwrap();
        // a real, distinct palette — not a placeholder equal to dark, and not
        // literally 255-x of dark's channels either
        assert!(!colors_eq(dark.add_bg, light.add_bg));
        let Color::Rgb(dr, dg, db) = dark.add_bg else {
            panic!("dark add_bg not Rgb")
        };
        let Color::Rgb(lr, lg, lb) = light.add_bg else {
            panic!("light add_bg not Rgb")
        };
        assert!(
            !(lr == 255 - dr && lg == 255 - dg && lb == 255 - db),
            "not a bitwise inversion"
        );
    }

    fn colors_eq(a: Color, b: Color) -> bool {
        matches!((a, b), (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) if ar == br && ag == bg && ab == bb)
    }

    // ---- :group header rows ----

    fn grouped_item(path: &str, group: &str) -> Item {
        let mut it = test_item(path);
        it.group = group.to_string();
        // hunk mode: the list buckets by group id
        it.bucket = group.to_string();
        it
    }

    #[test]
    fn display_rows_inserts_one_header_per_contiguous_group_run() {
        let items = vec![
            grouped_item("a.rs", "g0"),
            grouped_item("a.rs", "g0"),
            grouped_item("b.rs", "g1"),
        ];
        let mut groups = HashMap::new();
        groups.insert("g0".to_string(), "same definition: run".to_string());
        groups.insert("g1".to_string(), "same scope: top-level".to_string());
        let view = vec![0, 1, 2];

        let rows = display_rows(&view, &items, &groups, true, &HashSet::new());
        let kinds: Vec<&str> = rows
            .iter()
            .map(|r| match r {
                DisplayRow::Header(_) => "header",
                DisplayRow::Item(_) => "item",
            })
            .collect();
        assert_eq!(kinds, vec!["header", "item", "item", "header", "item"]);
        let DisplayRow::Header(reason) = &rows[0] else {
            panic!("expected a header")
        };
        // the header carries its fold marker and how many hunks it covers
        assert_eq!(reason, "▾ same definition: run (2)");
    }

    // ---- reviewing rules: the client half ----

    #[test]
    fn rules_come_from_the_user_then_the_repository() {
        let srcs = rule_sources("/repo");
        assert_eq!(srcs.len(), 2);
        assert!(srcs[0].ends_with("ordo/rules.toml"), "{:?}", srcs[0]);
        assert_eq!(srcs[1], PathBuf::from("/repo/.ordo/rules.toml"));
    }

    #[test]
    fn a_rules_file_parses_into_rules() {
        let (rules, problems) = parse_rules(
            "[[rule]]\nname = \"security-first\"\npath = \"src/security/**\"\nnote = \"sensitive\"\npriority = 100\n\n             [[rule]]\nname = \"vendored\"\npath = \"vendor/**\"\nnoise = true\n",
            Path::new("."),
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "security-first");
        assert_eq!(rules[0].when.path.as_deref(), Some("src/security/**"));
        assert_eq!(rules[0].note.as_deref(), Some("sensitive"));
        assert_eq!(rules[0].priority, 100);
        assert!(rules[1].noise);
    }

    #[test]
    fn a_rule_without_a_name_is_a_problem_not_a_silent_default() {
        // the old line-based parser couldn't tell "missing" from "empty" and
        // papered over it with an auto name (`rule-1`); real TOML makes `name`
        // a required field, so a rule without one fails to convert and is
        // reported, rather than kept under a name nobody wrote
        let (rules, problems) =
            parse_rules("[[rule]]\npath = \"a/**\"\nnote = \"n\"\n", Path::new("."));
        assert!(rules.is_empty(), "{rules:?}");
        assert!(problems[0].contains("missing field `name`"), "{problems:?}");
    }

    #[test]
    fn a_bad_rule_is_reported_and_dropped_not_partially_applied() {
        // the old parser evaluated each `key = value` line independently, so
        // a rule with a bad line still got kept with whatever lines *did*
        // parse, plus a problem per bad line. A typed table deserializes
        // atomically: an unknown key fails the whole rule, one problem,
        // nothing partially applied
        let (rules, problems) =
            parse_rules("[[rule]]\nname = \"a\"\nnonsense = \"x\"\n", Path::new("."));
        assert!(rules.is_empty(), "{rules:?}");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("nonsense"), "{problems:?}");
    }

    #[test]
    fn one_broken_rule_does_not_sink_the_others() {
        let (rules, problems) = parse_rules(
            "[[rule]]\nname = \"bad\"\npriority = \"soon\"\n\n[[rule]]\nname = \"good\"\npath = \"x/**\"\n",
            Path::new("."),
        );
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert_eq!(rules[0].name, "good");
        assert_eq!(rules[0].when.path.as_deref(), Some("x/**"));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("bad"), "{problems:?}");
    }

    #[test]
    fn an_unknown_key_names_itself_in_the_problem() {
        let (rules, problems) = parse_rules("name = \"loose\"\n", Path::new("."));
        assert!(rules.is_empty());
        assert!(problems[0].contains("name"), "{problems:?}");
    }

    #[test]
    fn a_query_can_live_in_its_own_file() {
        let dir = std::env::temp_dir().join(format!("ordo-rules-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let q = "(call function: (identifier) @fn)";
        std::fs::write(dir.join("q.scm"), q).unwrap();
        let (rules, problems) =
            parse_rules("[[rule]]\nname = \"q\"\nquery-file = \"q.scm\"\n", &dir);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rules[0].when.query.as_deref(), Some(q));

        // and a missing one is reported rather than silently never matching
        let (_, problems) =
            parse_rules("[[rule]]\nname = \"q\"\nquery-file = \"nope.scm\"\n", &dir);
        assert!(problems[0].contains("nope.scm"), "{problems:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_multi_line_query_and_a_kind_list_both_read() {
        let (rules, problems) = parse_rules(
            "[[rule]]\nname = \"loop-shapes\"\nkind = [\"for_statement\", \"while_statement\"]\nquery = '''\n(call\n  function: (identifier) @f)\n'''\n",
            Path::new("."),
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            rules[0].when.kind.as_deref(),
            Some(&["for_statement".to_string(), "while_statement".to_string()][..])
        );
        assert_eq!(
            rules[0].when.query.as_deref(),
            Some("(call\n  function: (identifier) @f)\n")
        );
    }

    #[test]
    fn kind_as_a_bare_string_also_reads_as_a_one_entry_list() {
        let (rules, problems) = parse_rules(
            "[[rule]]\nname = \"one-kind\"\nkind = \"for_statement\"\n",
            Path::new("."),
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            rules[0].when.kind.as_deref(),
            Some(&["for_statement".to_string()][..])
        );
    }

    #[test]
    fn kebab_case_keys_reach_the_matching_when_fields() {
        let (rules, problems) = parse_rules(
            "[[rule]]\nname = \"limits\"\npath-not = \"vendor/**\"\nmax-params = 4\ncontainer-without = \"Drop\"\nmember-uninitialized = true\n",
            Path::new("."),
        );
        assert!(problems.is_empty(), "{problems:?}");
        let w = &rules[0].when;
        assert_eq!(w.path_not.as_deref(), Some("vendor/**"));
        assert_eq!(w.max_params, Some(4));
        assert_eq!(w.container_without.as_deref(), Some("Drop"));
        assert_eq!(w.member_uninitialized, Some(true));
    }

    #[test]
    fn a_rules_flag_file_is_layered_last_and_a_missing_one_is_a_problem() {
        let dir = std::env::temp_dir().join(format!("ordo-rules-flag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let extra = dir.join("extra.toml");
        std::fs::write(
            &extra,
            "[[rule]]\nname = \"from-flag\"\nkind = \"type_definition\"\nnote = \"n\"\n",
        )
        .unwrap();
        let (rules, problems) = load_rules("", &[extra.to_string_lossy().into_owned()]);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(rules.iter().any(|r| r.name == "from-flag"));
        // the implicit user/repo files may be absent; a file named on the
        // command line was asked for, so its absence is reported
        let (_, problems) = load_rules("", &[dir.join("nope.toml").to_string_lossy().into_owned()]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("nope.toml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_shipped_ruleset_is_a_bundled_preset() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("rulesets");
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|x| x == "toml") {
                let stem = p.file_stem().unwrap().to_str().unwrap();
                assert!(
                    preset(stem).is_some(),
                    "rulesets/{stem}.toml is not in PRESETS"
                );
            }
        }
        for (name, text) in PRESETS {
            let d = parse_rules_doc(text, Path::new("."));
            assert!(d.problems.is_empty(), "{name}: {:?}", d.problems);
        }
    }

    fn rules_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ordo-rules-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_included_preset_layers_first_and_a_same_named_rule_replaces_its_entry() {
        let dir = rules_dir("include");
        let mine = dir.join("rules.toml");
        std::fs::write(&mine, concat!(
            "include = [\"go-uber-guide\"]\n",
            "[[rule]]\nname = \"no-panic\"\nlang = \"go\"\nuses = \"panic\"\nnote = \"ours: panic is fine in main\"\n",
            "[[rule]]\nname = \"no-cgo\"\nlang = \"go\"\nimports = \"C\"\nwarn = \"cgo\"\n",
        )).unwrap();
        let r = report_from(vec![], &[mine.to_string_lossy().into_owned()]);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let preset_len = parse_rules_doc(preset("go-uber-guide").unwrap(), Path::new("."))
            .rules
            .len();
        assert_eq!(
            r.rules.len(),
            preset_len + 1,
            "one replaced in place, one added"
        );
        let np = r.rules.iter().find(|x| x.name == "no-panic").unwrap();
        assert_eq!(np.note.as_deref(), Some("ours: panic is fine in main"));
        assert_eq!(r.replaced.len(), 1);
        assert!(r.replaced[0].starts_with("no-panic"), "{:?}", r.replaced);
        assert_eq!(
            r.origins
                .iter()
                .find(|(o, _)| o == "go-uber-guide")
                .map(|(_, n)| *n),
            Some(preset_len - 1)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disables_apply_after_every_layer_so_an_earlier_file_can_silence_a_later_include() {
        let dir = rules_dir("disable");
        let user = dir.join("user.toml");
        let repo = dir.join("repo.toml");
        std::fs::write(&user, "disable = [\"no-init\", \"*-size\"]\n").unwrap();
        std::fs::write(&repo, "include = [\"go-uber-guide\"]\n").unwrap();
        let r = report_from(
            vec![],
            &[
                user.to_string_lossy().into_owned(),
                repo.to_string_lossy().into_owned(),
            ],
        );
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(r.rules.iter().all(|x| x.name != "no-init"));
        assert!(
            r.disabled.iter().any(|d| d.starts_with("no-init")),
            "{:?}",
            r.disabled
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_include_and_a_duplicate_name_are_problems() {
        let dir = rules_dir("problems");
        let f = dir.join("rules.toml");
        std::fs::write(
            &f,
            concat!(
                "include = [\"./nope.toml\"]\n",
                "[[rule]]\nname = \"twice\"\nnote = \"a\"\n",
                "[[rule]]\nname = \"twice\"\nnote = \"b\"\n",
            ),
        )
        .unwrap();
        let r = report_from(vec![], &[f.to_string_lossy().into_owned()]);
        assert_eq!(r.rules.len(), 1);
        assert!(
            r.problems.iter().any(|p| p.contains("nope.toml")),
            "{:?}",
            r.problems
        );
        assert!(
            r.problems.iter().any(|p| p.contains("defined twice")),
            "{:?}",
            r.problems
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_include_cycle_is_reported_not_looped() {
        let dir = rules_dir("cycle");
        let f = dir.join("rules.toml");
        std::fs::write(&f, "include = [\"./rules.toml\"]\n").unwrap();
        let r = report_from(vec![], &[f.to_string_lossy().into_owned()]);
        assert!(
            r.problems.iter().any(|p| p.contains("cycle")),
            "{:?}",
            r.problems
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rules_flag_names_a_preset_or_a_file() {
        let r = report_from(vec![], &["c-power-of-ten".to_string()]);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(r.rules.iter().any(|x| x.name == "no-recursion"));
        assert_eq!(
            r.origins,
            vec![("c-power-of-ten".to_string(), r.rules.len())]
        );
        assert!(r.lines()[0].ends_with("rules active"));
    }

    #[test]
    fn ordos_own_rules_file_loads_with_zero_problems() {
        let text = std::fs::read_to_string(".ordo/rules.toml").expect("repo has .ordo/rules.toml");
        let (rules, problems) = parse_rules(&text, Path::new(".ordo"));
        assert!(problems.is_empty(), "{problems:?}");
        assert!(!rules.is_empty());
    }

    // ---- --init-config ----

    #[test]
    fn the_generated_config_is_one_the_program_accepts() {
        // uncommenting the whole file must parse with no complaints: a
        // generated config that ordo itself rejects is worse than none
        for (preset, theme_name) in [("vim", "dark"), ("vscode", "catppuccin-mocha")] {
            let text = init_config(preset, theme_name);
            let live: String = text
                .lines()
                .map(|l| match l.trim_start().strip_prefix("# ") {
                    // a `#` line that looks like a setting is a commented-out
                    // default; anything else is prose
                    Some(rest) if rest.contains(" = ") => rest.to_string(),
                    _ => l.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let cfg = parse_key_config(&live);
            assert!(
                cfg.problems.is_empty(),
                "{preset}/{theme_name}: {:?}",
                cfg.problems
            );
            assert_eq!(cfg.preset.as_deref(), Some(preset));
            assert_eq!(cfg.theme.as_deref(), Some(theme_name));
        }
    }

    #[test]
    fn the_generated_config_changes_nothing_when_fully_uncommented() {
        // ...and the values it writes are the ones already in effect, so a
        // reviewer who uncomments everything sees no difference
        let text = init_config("vim", "catppuccin-mocha");
        // keep the section headers: uncommenting the settings without them
        // would file every line under the top level
        let live: String = text
            .lines()
            .filter_map(|l| match l.trim_start().strip_prefix("# ") {
                Some(rest) if rest.contains(" = ") => Some(rest.to_string()),
                _ if l.starts_with('[') => Some(l.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let cfg = parse_key_config(&live);
        assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);

        let base = keymap("vim").unwrap();
        let after = apply_key_config(keymap("vim").unwrap(), &cfg);
        assert_eq!(
            after.binds.len(),
            base.binds.len(),
            "no binding gained or lost"
        );
        for b in &base.binds {
            assert!(after.binds.contains(b), "{:?} was dropped", key_label(b.1));
        }

        let t0 = theme("catppuccin-mocha").unwrap();
        let t1 = apply_theme_colors(t0, &cfg.colors);
        for role in THEME_ROLES {
            assert!(
                colors_eq(theme_role_color(&t0, role), theme_role_color(&t1, role)),
                "role `{role}` changed"
            );
        }
    }

    #[test]
    fn every_generated_binding_names_a_real_action() {
        let text = init_config("vscode", "nord");
        for line in text.lines().filter_map(|l| l.strip_prefix("# \"")) {
            let Some((_, rest)) = line.split_once("\" = \"") else {
                continue;
            };
            let Some((action, _)) = rest.split_once('"') else {
                continue;
            };
            assert!(
                action_by_name(action).is_some(),
                "`{action}` is not an action"
            );
        }
    }

    // ---- theming ----

    #[test]
    fn every_theme_name_resolves_and_truecolor_themes_leave_nothing_to_the_terminal() {
        for name in theme_names() {
            let t = theme(&name).expect(&name);
            assert_eq!(t.name, name, "a theme must know its own name");
            if name == "dark" || name == "light" {
                // the promise of a terminal theme: text follows the terminal
                assert_eq!(t.fg, Color::Reset, "{name}");
                continue;
            }
            // a truecolor theme names everything; a stray Reset would show up
            // as one element mysteriously following the terminal instead
            let roles: Vec<(&str, Color)> = vec![
                ("fg", t.fg),
                ("dim", t.dim),
                ("border", t.border),
                ("border_focus", t.border_focus),
                ("accent", t.accent),
                ("category", t.category),
                ("mark", t.mark),
                ("reviewed", t.reviewed),
                ("warn", t.warn),
                ("add_fg", t.add_fg),
                ("del_fg", t.del_fg),
                ("add_bg", t.add_bg),
                ("del_bg", t.del_bg),
                ("add_strong_bg", t.add_strong_bg),
                ("del_strong_bg", t.del_strong_bg),
                ("select_bg", t.select_bg),
                ("match_bg", t.match_bg),
                ("match_cur_bg", t.match_cur_bg),
                ("syn.comment", t.syn.comment),
                ("syn.keyword", t.syn.keyword),
                ("syn.string", t.syn.string),
                ("syn.number", t.syn.number),
                ("syn.function", t.syn.function),
                ("syn.type", t.syn.type_),
                ("syn.property", t.syn.property),
                ("syn.operator", t.syn.operator),
                ("syn.variable", t.syn.variable),
                ("syn.builtin", t.syn.builtin),
                ("syn.param", t.syn.param),
                ("syn.attribute", t.syn.attribute),
            ];
            for (role, c) in roles {
                assert!(
                    matches!(c, Color::Rgb(..)),
                    "{name}: {role} is not truecolor"
                );
            }
        }
    }

    #[test]
    fn a_diff_tint_is_distinguishable_from_the_selection_tint() {
        // the three tints a row can carry must not collapse into each other,
        // or an added line and a selected line look the same
        for name in theme_names() {
            let t = theme(&name).unwrap();
            assert!(!colors_eq(t.add_bg, t.del_bg), "{name}: add/del");
            assert!(!colors_eq(t.add_bg, t.select_bg), "{name}: add/select");
            assert!(!colors_eq(t.del_bg, t.select_bg), "{name}: del/select");
            assert!(
                !colors_eq(t.add_bg, t.add_strong_bg),
                "{name}: add/add-strong"
            );
            assert!(
                !colors_eq(t.del_bg, t.del_strong_bg),
                "{name}: del/del-strong"
            );
            assert!(
                !colors_eq(t.match_bg, t.match_cur_bg),
                "{name}: match/current"
            );
        }
    }

    #[test]
    fn parse_hex_takes_the_form_palettes_publish_and_nothing_else() {
        assert_eq!(parse_hex("#89b4fa"), Some(Color::Rgb(0x89, 0xb4, 0xfa)));
        assert_eq!(parse_hex("89b4fa"), Some(Color::Rgb(0x89, 0xb4, 0xfa)));
        assert_eq!(parse_hex("  #000000 "), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_hex("#89b4f"), None, "five digits");
        assert_eq!(parse_hex("#89b4fag"), None, "not hex");
        assert_eq!(parse_hex("blue"), None, "colour names are not accepted");
    }

    #[test]
    fn every_documented_theme_role_actually_changes_the_theme() {
        // THEME_ROLES is what the docs promise a config can set; a name listed
        // there but missing from `apply_theme_colors` would silently do nothing
        let base = theme("catppuccin-mocha").unwrap();
        let sentinel = Color::Rgb(1, 2, 3);
        for role in THEME_ROLES {
            let got = apply_theme_colors(base, &[(role.to_string(), sentinel)]);
            let changed = [
                got.fg,
                got.dim,
                got.border,
                got.border_focus,
                got.accent,
                got.category,
                got.mark,
                got.reviewed,
                got.warn,
                got.add_fg,
                got.del_fg,
                got.add_bg,
                got.del_bg,
                got.add_strong_bg,
                got.del_strong_bg,
                got.select_bg,
                got.match_bg,
                got.match_cur_bg,
                got.syn.comment,
                got.syn.keyword,
                got.syn.string,
                got.syn.number,
                got.syn.function,
                got.syn.type_,
                got.syn.property,
                got.syn.operator,
                got.syn.variable,
                got.syn.builtin,
                got.syn.param,
                got.syn.attribute,
            ]
            .iter()
            .filter(|c| colors_eq(**c, sentinel))
            .count();
            assert_eq!(changed, 1, "role `{role}` set {changed} fields, expected 1");
            // and the read side must name the same field as the write side —
            // without this, `--init-config` can print one role's colour under
            // another role's name
            assert!(
                colors_eq(theme_role_color(&got, role), sentinel),
                "role `{role}` reads back a different field than it writes"
            );
        }
    }

    #[test]
    fn a_config_can_name_a_theme_and_override_its_roles() {
        let cfg = parse_key_config(
            "[theme]\nname = \"nord\"\nborder-focus = \"#ff0000\"\nsyntax-keyword = \"00ff00\"\n",
        );
        assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
        assert_eq!(cfg.theme.as_deref(), Some("nord"));

        let t = apply_theme_colors(theme(&cfg.theme.clone().unwrap()).unwrap(), &cfg.colors);
        assert!(colors_eq(t.border_focus, Color::Rgb(0xff, 0, 0)));
        assert!(colors_eq(t.syn.keyword, Color::Rgb(0, 0xff, 0)));
        // everything not named keeps the palette's own value
        assert!(colors_eq(t.syn.string, theme("nord").unwrap().syn.string));
    }

    #[test]
    fn a_bad_theme_line_is_reported_by_number_and_skipped() {
        let cfg = parse_key_config(
            "[theme]\nnmae = \"nord\"\nborder-focus = \"redish\"\nmark = \"#ffcc00\"\n",
        );
        assert_eq!(cfg.problems.len(), 2, "{:?}", cfg.problems);
        assert!(cfg.problems[0].contains("line 2"), "{:?}", cfg.problems);
        assert!(cfg.problems[1].contains("line 3"), "{:?}", cfg.problems);
        assert_eq!(cfg.colors.len(), 1, "the good line still lands");
    }

    #[test]
    fn a_hash_opens_a_comment_only_outside_quotes() {
        // every palette value starts with `#`; cutting at the first one
        // regardless would eat the whole theme section
        assert_eq!(
            strip_comment("mark = \"#ffcc00\"  # the ⚠ colour"),
            "mark = \"#ffcc00\"  "
        );
        assert_eq!(strip_comment("# whole line"), "");
        assert_eq!(strip_comment("preset = \"vim\""), "preset = \"vim\"");

        let cfg = parse_key_config("[theme]\nmark = \"#ffcc00\"  # trailing\n");
        assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
        assert_eq!(cfg.colors[0].0, "mark");
        assert!(colors_eq(cfg.colors[0].1, Color::Rgb(0xff, 0xcc, 0)));
    }

    #[test]
    fn an_unknown_section_is_reported_rather_than_silently_ignored() {
        let cfg = parse_key_config("[colours]\nfg = \"#ffffff\"\n");
        assert!(
            cfg.problems[0].contains("unknown section"),
            "{:?}",
            cfg.problems
        );
    }

    #[test]
    fn theme_completes_from_the_theme_list() {
        let got = command_completions("theme catp", &[], &[], &[]);
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(got.iter().all(|n| n.starts_with("catppuccin-")), "{got:?}");
    }

    #[test]
    fn the_theme_command_swaps_the_palette_and_rejects_an_unknown_name() {
        let mut app = test_app(0);
        assert!(execute_command(&mut app, "theme nord").is_ok());
        assert_eq!(app.theme.name, "nord");
        match execute_command(&mut app, "theme nonesuch") {
            Err(msg) => assert!(msg.contains("unknown theme"), "{msg}"),
            Ok(_) => panic!("an unknown theme must be refused"),
        }
        // and the refusal leaves the previous theme in place
        assert_eq!(app.theme.name, "nord");
    }

    // ---- configurable keybinds ----

    #[test]
    fn every_action_has_a_config_name_and_every_name_resolves() {
        // the table is the only way to name an action in a config file: an
        // action missing from it simply cannot be bound
        for km in ["vim", "vscode"] {
            for (_, _, action) in keymap(km).unwrap().binds {
                assert!(
                    ACTION_NAMES.iter().any(|(_, a)| *a == action),
                    "{action:?} is bound in {km} but has no config name"
                );
            }
        }
        for (name, action) in ACTION_NAMES {
            assert_eq!(action_by_name(name), Some(*action));
        }
    }

    #[test]
    fn parse_key_is_the_inverse_of_key_label() {
        for km in ["vim", "vscode"] {
            for (prefix, key, _) in keymap(km).unwrap().binds {
                for k in prefix.into_iter().chain([key]) {
                    assert_eq!(parse_key(&key_label(k)), Some(k), "{}", key_label(k));
                }
            }
        }
    }

    #[test]
    fn a_config_can_add_replace_and_remove_bindings() {
        let cfg = parse_key_config(
            "preset = \"vscode\"\n\n[binds]\n\"C-n\" = \"next\"\n\"g d\" = \"jump-to-edge\"\n\"x\" = \"none\"\n",
        );
        assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
        assert_eq!(cfg.preset.as_deref(), Some("vscode"));

        let km = apply_key_config(keymap("vim").unwrap(), &cfg);
        let has = |p: Option<Key>, k: Key, a: Action| km.binds.contains(&(p, k, a));
        assert!(has(None, ctrl('n'), Action::Next), "added binding");
        assert!(
            has(Some(ch('g')), ch('d'), Action::JumpToEdge),
            "chord binding"
        );
        assert!(
            !km.binds
                .iter()
                .any(|(p, k, _)| p.is_none() && *k == ch('x')),
            "`none` removes the binding"
        );
    }

    #[test]
    fn a_config_binding_replaces_the_presets_own() {
        let cfg = parse_key_config("[binds]\n\"j\" = \"prev\"\n");
        let km = apply_key_config(keymap("vim").unwrap(), &cfg);
        let bound: Vec<Action> = km
            .binds
            .iter()
            .filter(|(p, k, _)| p.is_none() && *k == ch('j'))
            .map(|(_, _, a)| *a)
            .collect();
        assert_eq!(bound, vec![Action::Prev], "one binding, the config's");
    }

    #[test]
    fn a_bad_config_line_is_reported_by_number_and_skipped() {
        let cfg = parse_key_config(
            "[binds]\n\"C-n\" = \"nonsense\"\n\"!!\" = \"next\"\nnot a pair\n\"C-y\" = \"help\"\n",
        );
        assert_eq!(cfg.problems.len(), 3, "{:?}", cfg.problems);
        assert!(cfg.problems[0].contains("line 2"), "{:?}", cfg.problems);
        assert!(cfg.problems[1].contains("line 3"), "{:?}", cfg.problems);
        // the good line still lands
        assert_eq!(cfg.binds.len(), 1);
        assert_eq!(cfg.binds[0].2, Some(Action::Help));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let cfg = parse_key_config("# a comment\n\npreset = \"vim\"  # trailing\n");
        assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
        assert_eq!(cfg.preset.as_deref(), Some("vim"));
    }

    #[test]
    fn a_folded_group_shows_its_header_and_hides_its_hunks() {
        let items = vec![
            grouped_item("a.rs", "g0"),
            grouped_item("a.rs", "g0"),
            grouped_item("b.rs", "g1"),
        ];
        let mut groups = HashMap::new();
        groups.insert("g0".to_string(), "same definition: run".to_string());
        groups.insert("g1".to_string(), "same scope: top-level".to_string());
        let view = vec![0, 1, 2];
        let collapsed: HashSet<String> = ["g0".to_string()].into_iter().collect();

        let rows = display_rows(&view, &items, &groups, true, &collapsed);
        let kinds: Vec<&str> = rows
            .iter()
            .map(|r| match r {
                DisplayRow::Header(_) => "header",
                DisplayRow::Item(_) => "item",
            })
            .collect();
        assert_eq!(kinds, vec!["header", "header", "item"]);
        let DisplayRow::Header(h) = &rows[0] else {
            panic!("expected a header")
        };
        assert_eq!(
            h, "▸ same definition: run (2)",
            "a folded header says what it hides"
        );
    }

    #[test]
    fn a_folded_groups_rows_are_skipped_when_placing_the_selection() {
        let items = vec![
            grouped_item("a.rs", "g0"),
            grouped_item("a.rs", "g0"),
            grouped_item("b.rs", "g1"),
        ];
        let view = vec![0, 1, 2];
        let collapsed: HashSet<String> = ["g0".to_string()].into_iter().collect();
        // rows are: [g0 header][g1 header][item 2] — the third view entry is
        // the item at row 2
        assert_eq!(display_row_of(&view, &items, true, &collapsed, 2), 2);
    }

    #[test]
    fn folding_moves_the_selection_out_of_the_group_it_folds() {
        let mut app = test_app(0);
        app.items = vec![
            grouped_item("a.rs", "g0"),
            grouped_item("a.rs", "g0"),
            grouped_item("b.rs", "g1"),
        ];
        app.view = vec![0, 1, 2];
        app.reviewed = vec![false; 3];
        app.sel = 1;
        app.show_groups = true;

        fold(&mut app, Fold::Close);
        assert!(app.collapsed.contains("g0"));
        assert_eq!(app.sel, 2, "selection must land on a row that is drawn");
        assert_eq!(folded_view(&app), vec![2]);

        fold(&mut app, Fold::OpenAll);
        assert!(app.collapsed.is_empty());
        assert_eq!(folded_view(&app), vec![0, 1, 2]);
    }

    #[test]
    fn folding_turns_group_headers_on_because_that_is_what_was_meant() {
        let mut app = test_app(0);
        app.items = vec![grouped_item("a.rs", "g0")];
        app.view = vec![0];
        app.reviewed = vec![false];
        app.show_groups = false;
        fold(&mut app, Fold::Toggle);
        assert!(app.show_groups);
    }

    #[test]
    fn display_rows_is_flat_view_when_groups_are_off() {
        let items = vec![grouped_item("a.rs", "g0"), grouped_item("b.rs", "g1")];
        let groups = HashMap::new();
        let view = vec![0, 1];
        let rows = display_rows(&view, &items, &groups, false, &HashSet::new());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| matches!(r, DisplayRow::Item(_))));
    }

    #[test]
    fn display_row_of_skips_headers_so_selection_never_lands_on_one() {
        let items = vec![
            grouped_item("a.rs", "g0"),
            grouped_item("a.rs", "g0"),
            grouped_item("b.rs", "g1"),
        ];
        let groups = HashMap::new(); // reason lookup irrelevant to row placement
        let view = vec![0, 1, 2];
        let rows = display_rows(&view, &items, &groups, true, &HashSet::new());

        for pos in 0..view.len() {
            let row = display_row_of(&view, &items, true, &HashSet::new(), pos);
            assert!(
                matches!(rows[row], DisplayRow::Item(_)),
                "selection at view pos {pos} landed on row {row}, which is a header"
            );
        }
        // and the header count lines up: 2 groups among 3 items -> 2 headers,
        // so row indices for view positions [0,1,2] are [1,2,4]
        assert_eq!(
            (0..view.len())
                .map(|p| display_row_of(&view, &items, true, &HashSet::new(), p))
                .collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
    }

    #[test]
    fn display_row_of_is_identity_when_groups_are_off() {
        let items = vec![grouped_item("a.rs", "g0"), grouped_item("b.rs", "g1")];
        assert_eq!(
            display_row_of(&[0, 1], &items, false, &HashSet::new(), 0),
            0
        );
        assert_eq!(
            display_row_of(&[0, 1], &items, false, &HashSet::new(), 1),
            1
        );
    }

    // ---- `:e` — reload carry-forward ----

    #[test]
    fn carry_across_reload_keeps_keymap_and_theme() {
        let mut app = test_app(0);
        app.keys = keymap("vscode").unwrap();
        app.theme = theme("light").unwrap();
        let (keys, carried) = carry_across_reload(&app);
        assert_eq!(keys.name, "vscode");
        assert_eq!(carried.name, "light");
        assert!(colors_eq(carried.add_bg, theme("light").unwrap().add_bg));
    }

    #[test]
    fn e_command_rejects_an_empty_argument() {
        let mut app = test_app(0);
        match execute_command(&mut app, "e") {
            Err(msg) => assert!(msg.contains("usage")),
            Ok(_) => panic!("expected an error for a missing rev argument"),
        }
    }

    #[test]
    fn e_command_rejects_an_unresolvable_revision() {
        let mut app = test_app(0);
        match execute_command(&mut app, "e not-a-real-rev-xyzzy-12345") {
            Err(msg) => assert!(msg.contains("not-a-real-rev-xyzzy-12345")),
            Ok(_) => panic!("expected an error for an unresolvable rev"),
        }
    }

    #[test]
    fn e_command_resolves_head_to_a_reload_outcome() {
        // relies on the test binary running inside the ordo git checkout,
        // same assumption `resolve`'s own callers make
        let mut app = test_app(0);
        match execute_command(&mut app, "e HEAD") {
            Ok(CommandOutcome::Reload(_, rev)) => assert_eq!(rev, "HEAD"),
            other => panic!("expected a Reload outcome, got {}", other.is_ok()),
        }
    }

    #[test]
    fn arg_candidates_e_offers_the_given_rev_pool() {
        let revs = lines(&["zz", "HEAD", "main"]);
        assert_eq!(arg_candidates("e", &[], &[], &revs), revs);
        assert_eq!(
            command_completions("e H", &[], &[], &revs),
            vec!["HEAD".to_string()]
        );
    }

    // ---- `<base>..zz` / `<base>...zz` parsing (relies on the test binary
    // running inside the ordo git checkout, same assumption `resolve`'s own
    // callers make) ----

    #[test]
    fn base_dotdot_zz_resolves_to_a_worktree_range_at_base() {
        let base = git(&["rev-parse", "--verify", "-q", "main^{commit}"]);
        let base = base.trim().to_string();
        match resolve("main..zz") {
            Some(Target::WorktreeRange(b)) => assert_eq!(b, base),
            _ => panic!("expected a WorktreeRange at main"),
        }
    }

    #[test]
    fn base_dotdotdot_zz_resolves_to_the_merge_base_with_head() {
        let merge_base = git(&["merge-base", "main", "HEAD"]);
        let merge_base = merge_base.trim().to_string();
        match resolve("main...zz") {
            Some(Target::WorktreeRange(b)) => assert_eq!(b, merge_base),
            _ => panic!("expected a WorktreeRange at the merge base"),
        }
    }

    #[test]
    fn zz_on_the_left_is_rejected() {
        assert!(resolve_range("zz..main").is_none());
        assert!(resolve_range("zz...main").is_none());
        // resolve() as a whole also refuses it, same as any unresolvable rev
        assert!(resolve("zz..main").is_none());
    }

    #[test]
    fn plain_zz_is_unchanged() {
        assert!(matches!(resolve("zz"), Some(Target::Uncommitted)));
    }

    #[test]
    fn ordinary_commit_range_is_unchanged() {
        match resolve("main..HEAD") {
            Some(Target::Range(_, tip)) => {
                let head = git(&["rev-parse", "--verify", "-q", "HEAD"]);
                assert_eq!(tip, head.trim());
            }
            _ => panic!("expected a plain commit Range"),
        }
    }
}
