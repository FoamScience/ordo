//! ordo — a reviewer that is a pure client of the ordo engine, served to a
//! browser. It owns git (shells out for a commit's blobs), calls `ordo::run`,
//! and serves the change in comprehension order: the full file with the
//! changed hunk in context, plus rationale, advisories and def→use edges. The
//! engine stays git-free; gated behind the `web` feature so
//! `--no-default-features` builds the engine alone.
use crate::config::config_commit_edit;
use crate::config::config_path;
use crate::config::config_toggle;
use crate::config::config_write;
use crate::config::write_init_config;
use crate::config::ConfigUi;
use crate::keys::apply_key_config;
use crate::keys::keymap;
use crate::keys::parse_key_config;
use crate::keys::Fold;
use crate::keys::KeyConfig;
use crate::keys::Keymap;
use crate::keys::ViewMode;
use crate::theme::apply_theme_colors;
use crate::theme::theme;
use crate::theme::theme_names;
use crate::theme::Theme;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::commands::step_wave;
use crate::comments::comments_file_path;
use crate::comments::load_comments;
use crate::comments::LineComment;
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
use crate::history::churn_query;
use crate::history::file_churn;
use crate::history::hunk_churn;
use crate::history::Churn;
use crate::hover::hover;
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
use crate::reload::{place_of, restore, Place};
use crate::rules::load_rules_report;
use crate::rules::RuleSet;
use crate::watch::{Drift, Fingerprint, Stale, WatchMode};
use ordo::model::{Change, Input, Output, Symbol};
use tree_sitter::Tree;
mod commands;
mod comments;
mod config;
mod consumers;
mod editor;
mod facts;
mod findings;
mod git;
mod handoff;
mod highlight;
mod history;
mod hover;
mod keys;
mod marks;
mod reload;
mod rules;
mod theme;
mod watch;
mod waves;
mod web;

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

const USAGE: &str = "\
ordo — review a change in reading order: definitions before their uses.

usage: ordo [<rev>] [<glob>...] [options]
       ordo wave [-m <message>] | --list | --clear
       ordo help [<topic>]

  Serves the review to a browser on this machine and opens a tab on it.
  `ordo web ...` is the same command.

  <rev>   what to review (default HEAD)
            HEAD~2, a sha, a tag         one commit
            main..feat, main...feat      a range (... diffs from the merge base)
            zz                           uncommitted work (`@` too, on GitButler repos)
            main...zz                    the branch so far, uncommitted included
            a branch or commit ID        from `but status`, on GitButler repos
            int..at, int...at            a range to a GitButler branch's tip (either end)
            wave/2..wave/3               one agent turn, recorded by `ordo wave`
  <glob>  keep matching paths only; `!` excludes, `*` crosses `/`. Quote them.

examples:
  ordo                                   the last commit
  ordo zz                                uncommitted work
  ordo main...feat 'src/*' '!tests/*'    a branch, sources without tests
  ordo --watch=auto main...zz            follow an agent while it edits
  ordo --sarif out.sarif main...feat     analyzer findings on their hunks

options:
  --port <n>              the localhost port (default: any free one)
  --no-open               print the URL, do not open a browser
  --watch[=hint|auto]     flag the review stale when files change; auto reloads
  --watch-debounce <ms>   quiet time before a change counts (default 1500)
  --sarif <file>          attach SARIF 2.1.0 findings to hunks (repeatable)
  --coverage <file>       count changed lines lcov never ran (repeatable)
  --rules <file>          add a rules file (repeatable)
  --no-catalog            skip the built-in construct catalog
  --all                   keep generated and lock files
  --only-comments         only comment and docstring hunks
  --keys <vim|vscode>     key preset (default vim, or $ORDO_KEYS)
  --theme <name>          palette (default dark, or $ORDO_THEME)
  --init-config [--force] write ordo.toml with the current keys and theme
  --json                  print the engine input for <rev> and exit
  -V, --version
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
    /// whether a docs-only hunk sorts after the code (ordo.toml `docs_last`)
    docs_last: bool,
    filter: Filter,
    only_comments: bool,
    theme: Theme,
    extra_rules: Vec<String>,
    sarif: Vec<String>,
    coverage: Vec<String>,
    /// `--no-catalog`: run only the caller's own rules this once
    no_catalog: bool,
    /// `--watch[=hint|auto]`, else ordo.toml `watch`
    watch: WatchMode,
    debounce: Duration,
    /// `--json`: print the engine input and exit, for scripts that want ordo's
    /// revision and path handling without the reviewer
    json: bool,
    web: web::WebArgs,
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
        "web",
        "this reviewer: revisions, filters, keys, commands, themes, safety",
        include_str!("../../../docs/web.md"),
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
        out.push_str(&format!("  {name:<13}{blurb}\n"));
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
    // `--json` prints what was gathered, before rules, overlays or the server
    // exist; a flag for any of those would be quietly dropped
    if raw.json {
        let unused = [
            ("--only-comments", raw.only_comments),
            ("--no-catalog", raw.no_catalog),
            ("--rules", !raw.extra_rules.is_empty()),
            ("--sarif", !raw.sarif.is_empty()),
            ("--coverage", !raw.coverage.is_empty()),
            ("--watch", raw.watch.is_some()),
        ];
        if let Some((flag, _)) = unused.iter().find(|(_, set)| *set) {
            eprintln!("ordo: {flag} has no effect with --json\n\n{USAGE}");
            return Err(2);
        }
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
        watch: raw
            .watch
            .or_else(|| {
                cfg.as_ref()
                    .and_then(|c| c.watch.as_deref())
                    .and_then(WatchMode::parse)
            })
            .unwrap_or_default(),
        debounce: raw
            .debounce_ms
            .and_then(|v| v.parse().ok())
            .map_or(watch::DEBOUNCE, Duration::from_millis),
        json: raw.json,
        web: web::WebArgs {
            port: raw
                .port
                .as_deref()
                .and_then(|p| p.parse().ok())
                .unwrap_or(0),
            open: !raw.no_open,
        },
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
    watch: Option<WatchMode>,
    /// as written: checked once the flags are all read
    debounce_ms: Option<String>,
    json: bool,
    port: Option<String>,
    no_open: bool,
}

/// A flag whose value is the next argument.
#[derive(Clone, Copy)]
enum Pending {
    Preset,
    Theme,
    Rules,
    Sarif,
    Coverage,
    Debounce,
    Port,
}

/// The flag loop: what each argument said, with nothing resolved yet.
fn flag_loop(argv: Vec<String>) -> Result<RawArgs, i32> {
    let mut raw = RawArgs {
        rev: None,
        globs: vec![],
        skip_generated: true,
        no_catalog: false,
        only_comments: false,
        preset_given: std::env::var("ORDO_KEYS").is_ok(),
        preset: std::env::var("ORDO_KEYS").unwrap_or_else(|_| "vim".to_string()),
        theme_given: std::env::var("ORDO_THEME").is_ok(),
        theme_name: std::env::var("ORDO_THEME").unwrap_or_else(|_| "dark".to_string()),
        extra_rules: vec![],
        sarif: vec![],
        coverage: vec![],
        want_init: false,
        force: false,
        watch: None,
        debounce_ms: None,
        json: false,
        port: None,
        no_open: false,
    };
    // `ordo help [<topic>]` short-circuits everything else: it takes an
    // argument the flag loop below would otherwise read as a revision.
    if let Some(first) = argv.first() {
        if first == "help" || first == "--help" || first == "-h" {
            return Err(help_topic(argv.get(1).map(String::as_str)));
        }
        if first == "wave" {
            return Err(waves::cli(&argv[1..]));
        }
    }
    // `ordo web ...` is the same command
    let alias = argv.first().is_some_and(|a| a == "web");
    let mut pending: Option<Pending> = None;
    for a in argv.into_iter().skip(usize::from(alias)) {
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
            "--watch-debounce" => pending = Some(Pending::Debounce),
            "--port" => pending = Some(Pending::Port),
            "--no-open" => raw.no_open = true,
            s if s.starts_with("--watch-debounce=") => raw.take_value(
                Pending::Debounce,
                s["--watch-debounce=".len()..].to_string(),
            ),
            "--watch" => raw.watch = Some(WatchMode::Hint),
            s if s.starts_with("--watch=") => {
                let mode = &s["--watch=".len()..];
                let Some(m) = WatchMode::parse(mode) else {
                    eprintln!("ordo: --watch wants hint, auto or off, not '{mode}'\n\n{USAGE}");
                    return Err(2);
                };
                raw.watch = Some(m);
            }
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
            "--json" => raw.json = true,
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
        Some(Pending::Debounce) => {
            eprintln!("ordo: --watch-debounce needs milliseconds\n\n{USAGE}");
            Err(2)
        }
        Some(Pending::Port) => {
            eprintln!("ordo: --port needs a port number\n\n{USAGE}");
            Err(2)
        }
        _ if raw.port.as_ref().is_some_and(|v| v.parse::<u16>().is_err()) => {
            eprintln!("ordo: --port wants a number from 0 to 65535\n\n{USAGE}");
            Err(2)
        }
        _ if raw
            .debounce_ms
            .as_ref()
            .is_some_and(|v| v.parse::<u64>().is_err()) =>
        {
            eprintln!("ordo: --watch-debounce wants whole milliseconds\n\n{USAGE}");
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
            Pending::Debounce => self.debounce_ms = Some(value),
            Pending::Port => self.port = Some(value),
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
                eprintln!("ordo: ordo.toml: {p}");
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
// per item — pulled out of `gather_range`/`gather_uncommitted` so the counter
// arithmetic and wording are unit-testable without a worker thread
fn read_progress(path: &str, i: usize, total: usize) -> String {
    format!("reading {path} ({}/{total})", i + 1)
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
        watch,
        debounce,
        json,
        web,
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
    if json {
        println!("{}", input_json(&gather_input(target, &filter, &|_| {})));
        return Ok(());
    }
    // the single commit the hover's history section reviews from — a
    // range's tip, or HEAD standing in for the uncommitted area (`zz`)
    let review_sha = review_commit_sha(&target);
    let uncommitted = matches!(target, Target::Uncommitted | Target::WorktreeRange(_));
    // rules are the client's to collect: this user's, then this repository's
    let repo_root = git(&["rev-parse", "--show-toplevel"]);
    let report = load_rules_report(repo_root.trim(), &extra_rules);
    for p in &report.problems {
        eprintln!("ordo: {p}");
    }
    // the catalog is on unless this run turned it off, either way round: the
    // flag for once, `catalog = false` in a rules file for always
    let catalog = !no_catalog && report.catalog;
    let rules_report = report.lines(catalog);
    let rules = report.rule_set(catalog);
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
        target,
        carry: None,
        watch,
        debounce,
        drift: Drift::default(),
        watcher: None,
        waves_seen: None,
        waves_checked: std::time::Instant::now(),
    };
    session.count_waves();
    web::serve(session, web)
}

/// What the worker thread reports back over the channel while `gather` and
/// `ordo::run` do their work.
enum LoadMsg {
    /// one line of progress, replacing whatever was shown before
    Progress(String),
    /// `build_items` came back empty — what to say instead of a review
    Empty(String),
    /// boxed, which keeps `LoadMsg` itself small regardless of how much a load
    /// result carries
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
    /// `reviewed[i]` set from a persisted mark whose key matches item `i`
    /// (see `mark_key`) — loading is here, on the worker, alongside the
    /// rest of the load work
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
    /// the engine's one-line verdict on the whole change
    summary: Option<ordo::model::Summary>,
    comments: Vec<LineComment>,
    comments_path: Option<PathBuf>,
    deltas: Vec<Delta>,
    delta_gone: usize,
    /// the working tree this review was read from, when it can drift (see
    /// `watch`); taken before reading, so an edit landing mid-load shows up
    fingerprint: Option<Fingerprint>,
    /// which wave touched each line, kept so a re-order can tag its new items
    /// without asking git again, and what each wave's agent turn was
    waves: waves::Waves,
}

/// group id -> the engine's `Group::reason`, for `:group`'s header rows.
fn group_reasons(out: &Output) -> HashMap<String, String> {
    out.groups
        .iter()
        .map(|g| (g.id.clone(), g.reason.clone()))
        .collect()
}

/// Everything `gather`/`ordo::run` need, run on a worker thread.
/// `ordo::run` itself can `eprintln!` when a change is degraded to
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
    /// the reviewer's own rules (user + repo) plus the catalog switch,
    /// collected by `main`
    rules: RuleSet,
    /// paths given with `--sarif`; their findings are placed onto the hunks
    sarif: Vec<String>,
    /// paths given with `--coverage`; lcov tracefiles
    coverage: Vec<String>,
}

/// What `--json` prints: the gathered changes in the engine's input shape.
fn input_json(input: &Input) -> serde_json::Value {
    serde_json::json!({ "changes": input.changes })
}

fn gather_input(target: Target, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    match target {
        Target::Commit(sha) => gather(&sha, filter, progress),
        Target::Range(base, tip) => gather_range(&base, &tip, filter, progress),
        Target::Uncommitted => gather_uncommitted(filter, progress),
        Target::WorktreeRange(base) => gather_worktree_range(&base, filter, progress),
    }
}

fn load(spec: LoadSpec, tx: mpsc::Sender<LoadMsg>) {
    let LoadSpec {
        target,
        docs_last,
        filter,
        only_comments,
        rev,
        rules,
        sarif,
        coverage,
    } = spec;
    let progress = |msg: String| {
        let _ = tx.send(LoadMsg::Progress(msg));
    };
    // resolved before `target` is consumed below; per-file churn counts from it
    let churn_sha = review_commit_sha(&target);
    let fingerprint = matches!(target, Target::Uncommitted | Target::WorktreeRange(_))
        .then(|| watch::fingerprint("."));
    let wave_span = match &target {
        Target::Range(base, tip) => Some((base.clone(), Some(tip.clone()))),
        Target::WorktreeRange(base) => Some((base.clone(), None)),
        Target::Commit(_) | Target::Uncommitted => None,
    };
    let input = gather_input(target, &filter, &progress);
    // `only_comments` becomes the starting state of the live `:only-comments`
    // filter (see `compute_view`) rather than an engine option — the engine
    // always sees every hunk, so toggling it back off has something to show.
    let sources = split_sources(&input);
    progress("ordering…".to_string());
    let mut input = input;
    let root = git(&["rev-parse", "--show-toplevel"]);
    if !root.trim().is_empty() {
        progress("looking for other repositories that import this one…".to_string());
        input.consumers = consumers::gather(
            std::path::Path::new(root.trim()),
            &input.changes,
            &rules.consumers,
        );
    }
    input.options.rules = rules.rules;
    input.options.catalog = rules.catalog;
    input.options.disable = rules.disables;
    input.options.levels = rules.levels;
    input.options.docs_last = docs_last;
    let changes = input.changes.clone();
    let out = ordo::run(input);
    let mut ledger = filter.tally.get();
    count_engine_drops(&out, &mut ledger);
    let mut items = build_items(&out);
    place_overlays(&mut items, &sarif, &coverage, &mut ledger);
    refine_items(&mut items, &sources);
    let waves = wave_span
        .map(|(base, tip)| {
            let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
            waves::Waves::read(".", &base, tip.as_deref(), &paths)
        })
        .unwrap_or_default();
    waves::tag(&mut items, &waves.lines);
    let groups = group_reasons(&out);
    let view = compute_view(&items, only_comments, true, None, None);
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
    let churn_by_file = match &churn_sha {
        Some(sha) => file_churn(sha, &churn_paths, &progress),
        None => HashMap::new(),
    };
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
            // the carried notes are in memory either way; the next note written retries
            let _ = save_notes(p, &notes);
        }
    }
    let comments_path = notes_path.as_deref().and_then(comments_file_path);
    let mut comments = comments_path
        .as_deref()
        .map(load_comments)
        .unwrap_or_default();
    for c in &mut comments {
        if let Some((_, new)) = sources.get(&c.path) {
            c.relocate(new);
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
        reviewed,
        marks_path,
        marks,
        groups,
        ledger,
        symbol_ledger: out.ledger.clone(),
        summary: out.summary.clone(),
        notes,
        notes_path,
        comments,
        comments_path,
        deltas,
        delta_gone,
        fingerprint,
        waves,
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
    /// the wave that last changed this hunk's lines (see `waves::hunk_wave`);
    /// `None` outside a review of waves
    wave: Option<usize>,
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

/// A floating popup: the symbol hover's kind/signature/doc and, when the
/// definition resolves, a cross-commit history section — or what a command
/// had to say.
struct Popup {
    title: String,
    lines: Vec<String>,
}

impl Popup {
    fn new(title: impl Into<String>, lines: Vec<String>) -> Popup {
        Popup {
            title: title.into(),
            lines,
        }
    }
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
    /// `:only-wave` — show only the hunks one wave changed
    only_wave: Option<usize>,
    sel: usize,
    keys: Keymap,
    /// whether a docs-only hunk sorts after the code it describes; a `:config`
    /// setting, so a re-order has to be told about it
    docs_last: bool,
    /// the engine's input for this review — see `LoadResult::changes`. The
    /// text is also held line-split in `sources`, which the panes read; this
    /// copy exists so a re-run is the same run, and both are bounded by the
    /// size of the change under review
    changes: Vec<Change>,
    sources: Sources,
    cursor: Cursor,
    popup: Option<Popup>,
    trees: HashMap<String, ParsedFile>,
    /// the commit the history section is relative to; None when it couldn't
    /// be resolved (history is then reported unavailable rather than guessed)
    review_sha: Option<String>,
    /// the reviewed commit's subject line, for the top bar; `None` for a
    /// range or the working tree, which have no one subject
    subject: Option<String>,
    /// true when reviewing the uncommitted area (`zz`) or a `base..zz` /
    /// `base...zz` worktree range — there the worktree file IS the new
    /// content, so `ge`'s line number is always exact; for a commit or
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
    /// `[levels]` from every rules file, forwarded to the engine
    levels: BTreeMap<String, ordo::model::Level>,
    /// bundled rulesets named by `include` — what `:config` shows as on
    includes: Vec<String>,
    /// `:config`'s open state; `None` when it is closed
    config: Option<ConfigUi>,
    /// commits per file in `FILE_CHURN_WINDOW`, computed once at load. The
    /// cheap half of the signal: always shown, and `H` refines the selected
    /// hunk to its exact lines.
    file_churn: HashMap<String, usize>,
    /// the reviewed rev string, as typed — part of a persisted mark's key
    /// (see `mark_key`)
    rev: String,
    /// where `ToggleReviewed` persists a mark; `None` when the repo root or
    /// cache dir couldn't be resolved, so toggles just aren't written
    marks_path: Option<PathBuf>,
    /// in-memory mirror of the mark file, kept in sync on every toggle
    marks: HashMap<u64, u64>,
    /// the palette `:config` shows and edits — `--theme`/`$ORDO_THEME`,
    /// carried across a reload since it's a display preference, not
    /// something indexed to the reviewed revision
    theme: Theme,
    /// `:group` — show a non-selectable group-reason header before each run
    /// of the reading-order list that shares a group id
    show_groups: bool,
    /// bucket key -> the symbol its group's first row shows, empty for
    /// top-level code. What the keys *are* depends on `mode`: group ids in
    /// hunk mode, `L<n>` ledger keys in ledger mode (see `set_mode`).
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
    /// line and range comments, every file of the repository (see `comments`)
    comments: Vec<LineComment>,
    comments_path: Option<PathBuf>,
    /// the code-pane line a `v` selection started on, 0-based; the range runs
    /// from it to the cursor
    selection: Option<usize>,
    /// `--watch`: whether to look for drift, and whether to reload on it
    watch: WatchMode,
    /// the working tree no longer matches this review (see `watch`)
    stale: Option<Stale>,
    /// one line for the reader: what a reload or a command changed
    notice: Option<String>,
    /// `r` or `:watch`: read the review again, keeping the place
    reload_requested: bool,
    /// the engine's change ledger, kept so `:mode` can rebuild the headers.
    /// Named apart from `ledger`, which is the unrelated `:audit` ledger.
    symbol_ledger: Vec<ordo::model::LedgerEntry>,
    /// see `LoadResult::waves`
    waves: waves::Waves,
    /// see `LoadResult::summary`
    summary: Option<ordo::model::Summary>,
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
    /// otherwise the engine's own default
    strategy: String,
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
    out.order
        .iter()
        .filter_map(|o| {
            let (path, h) = by_id.get(o.hunk.as_str())?;
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
                bucket: bucket_key(ViewMode::Ledger, ledger, &h.group, h.category),
                ledger,
                old_range: h.old_range,
                new_range: h.new_range,
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
                wave: None,
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
    let own = by_id.get(h.id.as_str()).map(|(p, _)| *p);
    // the why pane's needs / needed-by rows already say the direction, so a
    // label is the name and where it lives — the path only when it is elsewhere
    let loc = |id: &str| {
        by_id
            .get(id)
            .map(|(p, h)| {
                if own == Some(*p) {
                    format!("L{}", h.new_range[0])
                } else {
                    format!("{p}:L{}", h.new_range[0])
                }
            })
            .unwrap_or_else(|| id.to_string())
    };
    out.edges
        .iter()
        .filter(|e| e.from == h.id || e.to == h.id)
        .map(|e| {
            let defines_it = e.from == h.id;
            let target_id = if defines_it { &e.to } else { &e.from };
            let name = e.why.strip_prefix("def→use: ").unwrap_or(&e.why);
            EdgeRef {
                label: format!("{name}  {}", loc(target_id)),
                target: item_index.get(target_id.as_str()).copied(),
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
    wave: Option<usize>,
) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, it)| !comments_only || it.comment)
        .filter(|(_, it)| show_all || !it.noise)
        .filter(|(_, it)| glob.is_none_or(|g| g.is_match(&it.path)))
        .filter(|(_, it)| wave.is_none_or(|w| it.wave == Some(w)))
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
    /// changed in another wave than `:only-wave`'s
    wave: usize,
    unaccounted: usize,
}

fn hidden_breakdown(
    items: &[Item],
    comments_only: bool,
    show_all: bool,
    glob: Option<&PathGlobs>,
    wave: Option<usize>,
) -> Hidden {
    let mut h = Hidden::default();
    for it in items {
        if comments_only && !it.comment {
            h.comment += 1;
        } else if !show_all && it.noise {
            h.noise += 1;
        } else if glob.is_some_and(|g| !g.is_match(&it.path)) {
            h.glob += 1;
        } else if wave.is_some_and(|w| it.wave != Some(w)) {
            h.wave += 1;
        }
    }
    let shown = compute_view(items, comments_only, show_all, glob, wave).len();
    h.unaccounted = items.len() - shown - h.comment - h.noise - h.glob - h.wave;
    h
}

// ---------------------------------------------------------- group headers

/// One row of the reading-order list's display. Only `Item` and `Folded` rows
/// stand for hunks; the rest are structure the selection never lands on.
enum DisplayRow {
    /// a run of one file's hunks begins: its path, and how many hunks run
    File(String, usize),
    /// a blank row before a top-level group that follows a named one, which
    /// would otherwise read as that group's continuation
    Gap,
    /// a hunk (an index into `App.items`), and whether it opens its group —
    /// the row that shows the group's symbol
    Item(usize, bool),
    /// a folded group: the hunks it hides, in order
    Folded(Vec<usize>),
}

/// The reading-order list's rows, in render order. `view` itself never grows
/// a structural entry — it stays exactly the indices `compute_view` produced,
/// so navigation (`step`/`select`/`first_visible`/`last_visible`, all of which
/// walk `view`) can never land `sel` on one; `display_row_of` finds the
/// selection's row in this output rather than re-deriving it. `groups` maps a
/// bucket to the symbol its first row shows. With `:group` off every row shows
/// its own symbol and nothing folds.
fn display_rows(
    view: &[usize],
    items: &[Item],
    groups: &HashMap<String, String>,
    show_groups: bool,
    collapsed: &HashSet<String>,
) -> Vec<DisplayRow> {
    let named = |gid: &str| groups.get(gid).is_some_and(|s| !s.is_empty());
    let mut rows = Vec::with_capacity(view.len());
    let mut last: Option<(&str, &str)> = None;
    let mut after_named = false;
    for (pos, &i) in view.iter().enumerate() {
        let it = &items[i];
        let gid = it.bucket.as_str();
        if last.is_none_or(|(path, _)| path != it.path) {
            let n = view[pos..]
                .iter()
                .take_while(|&&j| items[j].path == it.path)
                .count();
            rows.push(DisplayRow::File(it.path.clone(), n));
            after_named = false;
        }
        let opens = last != Some((it.path.as_str(), gid));
        last = Some((it.path.as_str(), gid));
        if !show_groups {
            rows.push(DisplayRow::Item(i, true));
            continue;
        }
        if opens {
            if after_named && !named(gid) {
                rows.push(DisplayRow::Gap);
            }
            after_named = named(gid);
        }
        if !collapsed.contains(gid) {
            rows.push(DisplayRow::Item(i, opens));
        } else if opens {
            rows.push(DisplayRow::Folded(vec![i]));
        } else if let Some(DisplayRow::Folded(hidden)) = rows.last_mut() {
            hidden.push(i);
        }
    }
    rows
}

/// The stops `j`/`k` make under a fold state: `view`, with each folded run
/// standing as one stop — the selection when it is inside, else the run's
/// first hunk — so the cursor can stand on a fold and open it again. Runs are
/// cut where `display_rows` cuts them, at a group or a file boundary.
fn folded_view(app: &App) -> Vec<usize> {
    if !app.show_groups || app.collapsed.is_empty() {
        return app.view.clone();
    }
    let mut out = Vec::with_capacity(app.view.len());
    let mut last: Option<(&str, &str)> = None;
    for &i in &app.view {
        let it = &app.items[i];
        let key = (it.path.as_str(), it.bucket.as_str());
        let opens = last != Some(key);
        last = Some(key);
        if opens || !app.collapsed.contains(&it.bucket) {
            out.push(i);
        } else if i == app.sel {
            *out.last_mut().expect("the run's first hunk was pushed") = i;
        }
    }
    out
}

/// The key a hunk groups under in a given mode. In ledger mode a hunk with no
/// symbol change anchored to it falls into one shared bucket rather than
/// vanishing — a formatting hunk is still part of the review. Imports keep the
/// engine's own per-file group in either mode, so they fold as one row.
fn bucket_key(
    mode: ViewMode,
    ledger: Option<usize>,
    group: &str,
    cat: ordo::model::Category,
) -> String {
    match (mode, ledger) {
        _ if cat == ordo::model::Category::Import => group.to_string(),
        (ViewMode::Hunks, _) => group.to_string(),
        (ViewMode::Ledger, Some(i)) => format!("L{i}"),
        (ViewMode::Ledger, None) => "L-".to_string(),
    }
}

/// What a group's first row names: the symbol it changes, or nothing for
/// top-level code. A removed symbol is not named — there is no definition
/// left to read, and the row's rationale already says what went — so a hunk
/// anchoring a removal is named by what it defines instead, if anything.
fn group_symbol(mode: ViewMode, it: &Item, ledger: &[ordo::model::LedgerEntry]) -> String {
    if it.cat == ordo::model::Category::Import {
        return "imports".to_string();
    }
    match mode {
        ViewMode::Ledger => it.ledger.and_then(|l| ledger.get(l)).and_then(|e| {
            if e.change != ordo::model::SymbolChange::Removed {
                Some(e.name.clone())
            } else {
                it.symbols.first().map(|s| s.name.clone())
            }
        }),
        ViewMode::Hunks => it.enclosing.clone(),
    }
    .unwrap_or_default()
}

/// Switch what the list is a list of. Re-keys every item and rebuilds the
/// symbol each group shows; groups are forced on in ledger mode, where they
/// *are* the ledger. Fold state belonged to the old keys, so it starts over:
/// imports folded, everything else open.
fn set_mode(app: &mut App, mode: ViewMode, ledger: &[ordo::model::LedgerEntry]) {
    app.mode = mode;
    app.groups.clear();
    for it in &mut app.items {
        it.bucket = bucket_key(mode, it.ledger, &it.group, it.cat);
        app.groups
            .insert(it.bucket.clone(), group_symbol(mode, it, ledger));
    }
    if mode == ViewMode::Ledger {
        app.show_groups = true;
    }
    app.collapsed = app
        .items
        .iter()
        .filter(|it| it.cat == ordo::model::Category::Import)
        .map(|it| it.bucket.clone())
        .collect();
}

/// Fold state change for the group holding the selected hunk (or every group).
/// The selection stays where it is: on a fold, the fold's row carries it, and
/// the same key opens it again.
fn fold(app: &mut App, how: Fold) {
    // folding is a statement about groups: turning them on is what the
    // reviewer meant, not an error to report
    app.show_groups = true;
    let gid = app.items[app.sel].bucket.clone();
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
            app.collapsed = app.items.iter().map(|it| it.bucket.clone()).collect();
        }
        Fold::OpenAll => app.collapsed.clear(),
    }
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

/// One pass of `--watch`: take in whatever the watcher has seen, and say
/// whether to read the review again now — asked for with `r`, or `auto` with
/// the tree settled and the reviewer idle (see `auto_reload_now`). A moved
/// base is shown and never followed.
fn watch_tick(session: &mut Session, app: &mut App, idle: Duration) -> bool {
    if std::mem::take(&mut app.reload_requested) {
        return true;
    }
    if let Some(seen) = session.waves_seen {
        if app.watch != WatchMode::Off && session.waves_checked.elapsed() >= watch::POLL {
            session.waves_checked = std::time::Instant::now();
            let n = waves::list(".").len();
            if n > seen {
                app.stale = Some(Stale::NewWave(n - 1));
            }
        }
    }
    if app.watch == WatchMode::Off || !session.uncommitted {
        // dropping the receiver ends the thread at its next fingerprint
        session.watcher = None;
        return auto_reload_now(app, idle);
    }
    let rx = session.watcher.get_or_insert_with(watch::spawn_watcher);
    let now = std::time::Instant::now();
    while let Ok(fp) = rx.try_recv() {
        app.stale = session
            .drift
            .observe(fp, now, session.debounce, watch::descends);
    }
    auto_reload_now(app, idle)
}

/// `--watch=auto`'s one rule: the tree settled into something worth reading,
/// and the reviewer is neither looking at a popup nor inside the config.
fn auto_reload_now(app: &App, idle: Duration) -> bool {
    app.watch == WatchMode::Auto
        && matches!(
            app.stale,
            Some(Stale::Drift(_) | Stale::Committed(_) | Stale::NewWave(_))
        )
        && idle >= watch::IDLE
        && app.popup.is_none()
        && app.config.is_none()
}

/// The status-bar line after a reload: what reading the change again moved,
/// from the same snapshot comparison `:delta` shows.
fn reload_summary(app: &App) -> String {
    let count = |d: Delta| app.deltas.iter().filter(|&&x| x == d).count();
    let parts: Vec<String> = [
        (count(Delta::Changed), "changed"),
        (count(Delta::Moved), "reordered"),
        (count(Delta::New), "new"),
        (app.delta_gone, "gone"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if parts.is_empty() {
        "reloaded · nothing changed".to_string()
    } else {
        format!("reloaded · {}", parts.join(" · "))
    }
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
    /// what is being reviewed, so a reload can read it again
    target: Target,
    /// where the reviewer was, handed from the review a reload replaces to
    /// the one it builds
    carry: Option<Place>,
    watch: WatchMode,
    debounce: Duration,
    /// the loaded review's fingerprint against what the watcher sees
    drift: Drift,
    /// fingerprints from the watcher thread, once watching has begun
    watcher: Option<mpsc::Receiver<Fingerprint>>,
    /// a review up to `wave/last` follows the chain: how many waves it read,
    /// and when the count was last looked at
    waves_seen: Option<usize>,
    waves_checked: std::time::Instant,
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
            rules: self.rules.clone(),
            sarif: self.sarif.clone(),
            coverage: self.coverage.clone(),
        };
        thread::spawn(move || load(spec, tx));
        rx
    }

    /// `r` or `--watch`: read `target` again into a fresh `App`. The
    /// reviewer's place — selection by hunk identity, filters, the mode —
    /// rides along as a `Place` and is restored once the new review is built
    /// (see `reload`); the keymap and theme are carried as they are. Reviewed
    /// marks come back from the on-disk cache keyed by the new rev, not from
    /// `app.marks`. An open popup is not carried: it was about the old review.
    fn reload(&mut self, app: &App, target: Target, new_rev: String) -> mpsc::Receiver<LoadMsg> {
        self.carry = Some(place_of(app));
        self.watch = app.watch;
        self.target = target.clone();
        self.review_sha = review_commit_sha(&target);
        self.uncommitted = matches!(target, Target::Uncommitted | Target::WorktreeRange(_));
        self.rev = new_rev;
        self.count_waves();
        self.keys = Some(app.keys.clone());
        self.theme = app.theme;
        self.spawn_load(target)
    }

    /// The same settings, reviewing another revision — a second tab of the
    /// web reviewer. `None` when `rev` names nothing reviewable.
    fn for_rev(&self, rev: &str) -> Option<Session> {
        let target = resolve(rev)?;
        let mut s = Session {
            rev: rev.to_string(),
            review_sha: review_commit_sha(&target),
            uncommitted: matches!(target, Target::Uncommitted | Target::WorktreeRange(_)),
            keys: self.keys.clone(),
            theme: self.theme,
            docs_last: self.docs_last,
            only_comments: self.only_comments,
            base_filter: self.base_filter.clone(),
            rules: self.rules.clone(),
            rules_report: self.rules_report.clone(),
            sarif: self.sarif.clone(),
            coverage: self.coverage.clone(),
            target,
            carry: None,
            watch: self.watch,
            debounce: self.debounce,
            drift: Drift::default(),
            watcher: None,
            waves_seen: None,
            waves_checked: std::time::Instant::now(),
        };
        s.count_waves();
        Some(s)
    }

    /// A review up to `wave/last` grows with the chain; note how long it is
    /// now, so the next wave can say so.
    fn count_waves(&mut self) {
        self.waves_seen = self
            .rev
            .contains("wave/last")
            .then(|| waves::list(".").len());
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
            reviewed,
            marks_path,
            marks,
            groups,
            ledger,
            symbol_ledger,
            summary,
            notes,
            notes_path,
            comments,
            comments_path,
            deltas,
            delta_gone,
            fingerprint,
            waves,
        } = r;
        self.drift = fingerprint.map(Drift::new).unwrap_or_default();
        let sel0 = view[0];
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
            only_wave: None,
            sel: sel0,
            keys: self
                .keys
                .take()
                .expect("keys is set again before every reload back into Loading"),
            sources,
            cursor,
            popup: None,
            trees: HashMap::new(),
            review_sha: self.review_sha.clone(),
            subject: match &self.target {
                Target::Commit(sha) => {
                    Some(git(&["log", "-1", "--format=%s", sha]).trim().to_string())
                        .filter(|s| !s.is_empty())
                }
                _ => None,
            },
            uncommitted: self.uncommitted,
            history_cache: HashMap::new(),
            churn_cache: HashMap::new(),
            file_churn,
            rev: self.rev.clone(),
            marks_path,
            marks,
            theme: self.theme,
            show_groups: false,
            group_reasons: groups.clone(),
            groups,
            mode: ViewMode::default(),
            symbol_ledger,
            summary,
            waves,
            notes,
            notes_path,
            comments,
            comments_path,
            selection: None,
            watch: self.watch,
            stale: None,
            notice: None,
            reload_requested: false,
            deltas,
            delta_gone,
            collapsed: HashSet::new(),
            ledger,
            rules: self.rules.rules.clone(),
            catalog: self.rules.catalog,
            disables: self.rules.disables.clone(),
            levels: self.rules.levels.clone(),
            includes: self.rules.includes.clone(),
            config: None,
            strategy: "comprehension".to_string(),
            rules_report: self.rules_report.clone(),
        };
        // the list is a list of *symbols* by default; `:mode` switches it
        // back to hunks
        let led = fresh.symbol_ledger.clone();
        set_mode(&mut fresh, ViewMode::Ledger, &led);
        if let Some(place) = self.carry.take() {
            restore(&mut fresh, place);
            fresh.notice = Some(reload_summary(&fresh));
        } else if self.watch != WatchMode::Off && !self.uncommitted && self.waves_seen.is_none() {
            fresh.notice = Some(format!("nothing to watch: {} is committed", self.rev));
        }
        fresh
    }
}

/// Writes a toggle to the mark cache immediately — not only on quit — so a
/// panic or a killed terminal never loses the session's marks. A no-op when
/// the item's content isn't loaded or `marks_path` never resolved (see
/// `mark_key`/`marks_file_path`); the checkmark still holds for this session
/// either way, and a write that fails says so.
fn persist_mark(app: &mut App, i: usize) -> Result<(), String> {
    let Some(key) = mark_key(&app.rev, &app.items[i], &app.sources) else {
        return Ok(());
    };
    // another reviewer of this repository — another tab, a second server —
    // may have written since this one loaded: merge into what is on disk
    // rather than writing this session's copy over it
    let want = app.reviewed[i];
    sync_marks(app);
    app.reviewed[i] = want;
    if want {
        app.marks.insert(key, now_unix());
    } else {
        app.marks.remove(&key);
    }
    let Some(path) = app.marks_path.clone() else {
        return Ok(());
    };
    prune_marks(&mut app.marks, now_unix());
    save_marks(&path, &app.marks)
        .map_err(|e| format!("the mark holds for this session, but was not written: {e}"))
}

/// Read the marks file again and mark every hunk it has a key for as it
/// says; a hunk with no key keeps this session's checkmark, and a file that
/// cannot be read changes nothing. True when a hunk's state changed.
fn sync_marks(app: &mut App) -> bool {
    let Some(marks) = app.marks_path.as_deref().and_then(crate::marks::read_marks) else {
        return false;
    };
    app.marks = marks;
    let before = app.reviewed.clone();
    for (i, it) in app.items.iter().enumerate() {
        if let Some(k) = mark_key(&app.rev, it, &app.sources) {
            app.reviewed[i] = app.marks.contains_key(&k);
        }
    }
    app.reviewed != before
}

/// `]f` / `[f` — the next or previous hunk on screen carrying a finding, in
/// one cycle ordered by top level (most severe first), then view order. From
/// a hunk outside the cycle, the nearest most-severe one in that direction.
fn finding_move(app: &mut App, forward: bool) {
    let view = folded_view(app);
    // `Level` declares its variants least severe first
    let mut cycle: Vec<(u8, usize)> = view
        .iter()
        .enumerate()
        .filter_map(|(pos, &i)| {
            let top = app.items[i].findings.iter().map(|f| f.level as u8).max()?;
            Some((top, pos))
        })
        .collect();
    if cycle.is_empty() {
        app.notice = Some("no findings in view".to_string());
        return;
    }
    cycle.sort_by_key(|&(top, pos)| (std::cmp::Reverse(top), pos));
    let n = cycle.len();
    let here = view_pos(&view, app.sel);
    let to = match cycle.iter().position(|&(_, pos)| pos == here) {
        Some(k) if forward => (k + 1) % n,
        Some(k) => (k + n - 1) % n,
        None => {
            let worst = cycle
                .iter()
                .take_while(|(top, _)| *top == cycle[0].0)
                .count();
            let found = if forward {
                cycle[..worst].iter().position(|&(_, pos)| pos > here)
            } else {
                cycle[..worst].iter().rposition(|&(_, pos)| pos < here)
            };
            found.unwrap_or(if forward { 0 } else { worst - 1 })
        }
    };
    select(app, view[cycle[to].1]);
}

/// The list-pane position of `sel` within `view` — 0 when it isn't found
/// (shouldn't happen: `set_filters`/`select` keep `sel` in `view`, but a
/// missing position degrading to the top is safer than panicking).
fn view_pos(view: &[usize], sel: usize) -> usize {
    view.iter().position(|&i| i == sel).unwrap_or(0)
}

fn select(app: &mut App, to: usize) {
    app.sel = to;
    app.selection = None;
    app.cursor = cursor_for(&app.items[to], &app.sources);
    app.popup = None; // a new hunk invalidates whatever the popup was showing
}

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
