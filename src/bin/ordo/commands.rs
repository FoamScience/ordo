// -------------------------------------------------------------- command mode
use crate::build_globs;
use crate::build_items;
use crate::code_view::Theme;
use crate::comments::save_comments;
use crate::comments::LineComment;
use crate::compute_view;
use crate::config::ConfigUi;
use crate::draw::draft_rule;
use crate::editor::is_vim_family;
use crate::editor::qf_kind;
use crate::editor::quickfix_script;
use crate::editor::resolve_editor;
use crate::editor::write_quickfix_script;
use crate::editor::QfHunk;
use crate::git::command_failures;
use crate::git::git;
use crate::git::resolve;
use crate::git::Target;
use crate::group_reasons;
use crate::handoff::review_prompt;
use crate::handoff::send;
use crate::handoff::yank;
use crate::hidden_breakdown;
use crate::keys::Keymap;
use crate::keys::Pane;
use crate::keys::ViewMode;
use crate::marks::mark_key;
use crate::marks::note_key;
use crate::marks::save_notes;
use crate::marks::Delta;
use crate::plural;
use crate::prose;
use crate::search::cycle_index;
use crate::select;
use crate::set_mode;
use crate::watch::WatchMode;
use crate::App;
use crate::CommandBar;
use crate::Hidden;
use crate::Item;
use crate::Ledger;
use crate::PathGlobs;
use crate::Popup;
use ordo::model::Input;
use ordo::model::Options;
use ordo::model::Strategy;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::text::Line;
use std::path::PathBuf;

/// One `:` command — name, a short argument hint for `:help` (empty when it
/// takes none), and the help line itself.
pub(super) struct Cmd {
    pub(super) name: &'static str,
    pub(super) args: &'static str,
    pub(super) help: &'static str,
}

/// The command table — the single source of truth for command names,
/// `:help`'s listing, and command-name completion. Adding a command here is
/// the only step that makes it discoverable; `execute_command`'s `match`
/// still has to know what to *do* with it, but a name present here with no
/// matching arm there falls into that match's `unknown command` case rather
/// than silently doing nothing.
pub(super) const COMMANDS: &[Cmd] = &[
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
        name: "config",
        args: "",
        help: "every setting, generated from the tables the program reads",
    },
    Cmd {
        name: "filter",
        args: "<glob>",
        help: "narrow the review to paths matching <glob>; no argument clears it",
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
        name: "comment",
        args: "[text]",
        help: "comment on the code-pane line, or the `v` selection; no text deletes what is there",
    },
    Cmd {
        name: "comments",
        args: "",
        help: "list every line comment in this review",
    },
    Cmd {
        name: "watch",
        args: "[on|off|auto]",
        help: "follow the working tree: mark the review stale (on), or reload by itself (auto)",
    },
    Cmd {
        name: "wave",
        args: "[message]",
        help: "record the working tree as the next wave; `ordo wave/2..wave/3` reviews one",
    },
    Cmd {
        name: "yank",
        args: "[all]",
        help: "copy the review as an agent prompt: your notes; `all` adds ordo's own notes and findings",
    },
    Cmd {
        name: "send",
        args: "[all]",
        help: "pipe the review prompt to the command in $ORDO_SEND (the herdr plugin sets it)",
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
pub(super) fn build_audit(
    items: &[Item],
    view_len: usize,
    hidden: &Hidden,
    ledger: &Ledger,
    path_filter: Option<&str>,
) -> Vec<String> {
    let mut out = vec![format!("{view_len} of {} hunks shown", items.len())];
    // a command that failed is the one explanation this report cannot derive
    // from its own counts, so it goes first
    let failures = command_failures();
    if !failures.is_empty() {
        out.push(String::new());
        out.push("commands that failed".to_string());
        for f in &failures {
            out.push(format!("  {f}"));
        }
    }
    out.push(String::new());
    out.push("hidden in the view".to_string());
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
    if ledger.coverage_files > 0 {
        out.push(row(
            ledger.coverage_unmatched,
            "files in the coverage tracefiles this review never looked at",
        ));
    }
    if ledger.findings_seen > 0 {
        out.push(row(
            ledger.findings_unplaced,
            "analyzer findings on lines this change did not touch",
        ));
    }
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

pub(super) fn command_names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

/// `:help`'s body, generated straight from `COMMANDS` — same discipline as
/// `build_help` for keybindings, so the listing can't name a command that
/// doesn't exist or omit one that does.
pub(super) fn build_command_help() -> Vec<String> {
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
pub(super) fn parse_command_line(line: &str) -> (&str, &str) {
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
pub(super) fn complete(input: &str, candidates: &[String]) -> Vec<String> {
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
pub(super) fn arg_candidates(
    cmd: &str,
    goto_paths: &[String],
    filter_dirs: &[String],
    rev_candidates: &[String],
) -> Vec<String> {
    match cmd {
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
pub(super) fn command_completions(
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
pub(super) fn apply_completion(text: &str, candidate: &str) -> String {
    match text.find(char::is_whitespace) {
        None => format!("{candidate} "),
        Some(pos) => format!("{}{candidate} ", &text[..=pos]),
    }
}

pub(super) fn distinct_sorted<'a>(vals: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut v: Vec<String> = vals.map(str::to_string).collect();
    v.sort();
    v.dedup();
    v
}

pub(super) fn dir_prefix(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

pub(super) fn open_command_bar(app: &mut App, text: String) {
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
pub(super) fn carry_across_reload(app: &App) -> (Keymap, Theme) {
    (app.keys.clone(), app.theme)
}

/// What running a command line does to the outer session — most just mutate
/// `app` in place and report `None`; `:q` quits, and `:e` needs to leave
/// `State::Ready` entirely and go back through the progressive loader, which
/// `execute_command`/`accept_command`/`handle_command_key` can't do on their
/// own (they only ever see `&mut App`) — so it's handed back to `run` as data.
pub(super) enum CommandOutcome {
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
pub(super) fn handle_command_key(
    app: &mut App,
    code: KeyCode,
    mods: KeyModifiers,
) -> CommandOutcome {
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
pub(super) fn run_strategy(app: &mut App, name: &str) -> Result<(), String> {
    let strategy = parse_strategy(name).ok_or_else(|| {
        format!("unknown strategy '{name}' (want: comprehension, defs-first, file)")
    })?;
    let input = Input {
        changes: app.changes.clone(),
        // everything but the strategy and the live rule state is what the
        // load ran with, which is the engine's own defaults: the client sets
        // no other option, and re-stating them here is how they would drift
        options: Options {
            strategy,
            rules: app.rules.clone(),
            catalog: app.catalog,
            docs_last: app.docs_last,
            disable: app.disables.clone(),
            ..Options::default()
        },
        consumers: vec![],
    };
    let out = ordo::run(input);
    let mut items = build_items(&out);
    crate::waves::tag(&mut items, &app.wave_lines);
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
pub(super) fn set_filters(
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
pub(super) fn execute_command(app: &mut App, line: &str) -> Result<CommandOutcome, String> {
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
        "config" => {
            app.config = Some(ConfigUi::open(app));
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
            app.popup = Some(Popup::new("audit", audit_lines(app)));
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
            let lines = delta_lines(app)?;
            app.popup = Some(Popup::new(
                "since you last looked".to_string(),
                lines.into_iter().map(prose).collect(),
            ));
            Ok(CommandOutcome::None)
        }
        "note" => {
            set_note(app, arg.trim())?;
            Ok(CommandOutcome::None)
        }
        "watch" => {
            let mode = match arg.trim() {
                "" if app.watch == WatchMode::Off => WatchMode::Hint,
                "" => WatchMode::Off,
                a => WatchMode::parse(a)
                    .ok_or_else(|| format!("usage: :watch [on|off|auto], not '{a}'"))?,
            };
            if mode != WatchMode::Off && !app.uncommitted {
                return Err(format!(
                    "nothing to watch: {} is committed — :e zz or :e main...zz reviews work that is still changing",
                    app.rev
                ));
            }
            app.watch = mode;
            if mode == WatchMode::Off {
                app.stale = None;
            }
            Ok(CommandOutcome::None)
        }
        "wave" => {
            app.notice = Some(match crate::waves::record(".", arg)? {
                Some(n) => format!("recorded wave/{n}"),
                None => "nothing changed since the last wave".to_string(),
            });
            Ok(CommandOutcome::None)
        }
        "comment" => {
            set_comment(app, arg.trim())?;
            Ok(CommandOutcome::None)
        }
        "comments" => {
            let lines = comment_list(app);
            if lines.is_empty() {
                return Err(
                    "no line comments in this review — `v` to select, `c` to comment".to_string(),
                );
            }
            app.popup = Some(Popup::new(
                "comments".to_string(),
                lines.into_iter().map(prose).collect(),
            ));
            Ok(CommandOutcome::None)
        }
        "yank" | "send" => {
            let all = match arg.trim() {
                "" => false,
                "all" => true,
                other => return Err(format!("usage: :{name} [all], not '{other}'")),
            };
            let Some(prompt) = review_prompt(app, all) else {
                return Err(if all {
                    "nothing to hand back: no notes or comments, and ordo found nothing".to_string()
                } else {
                    "nothing to hand back — :note a symbol, :comment a line, or `all` for ordo's own".to_string()
                });
            };
            let head = if name == "yank" {
                yank(&prompt)?;
                "copied to the clipboard (OSC 52)".to_string()
            } else {
                format!("sent to `{}`", send(&prompt)?)
            };
            app.popup = Some(Popup::new(
                head,
                prompt.lines().map(|l| prose(l.to_string())).collect(),
            ));
            Ok(CommandOutcome::None)
        }
        "mode" => {
            let to = parse_mode(arg.trim(), app.mode)?;
            let led = app.symbol_ledger.clone();
            set_mode(app, to, &led);
            Ok(CommandOutcome::None)
        }
        "goto" => {
            run_goto(app, arg.trim())?;
            Ok(CommandOutcome::None)
        }
        "quickfix" | "qf" | "vim-qfl" => {
            let script = quickfix_script(&app.rev, &app.strategy, &quickfix_hunks(app));
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

/// `:audit` — what the review covers and what the filters hide
fn audit_lines(app: &App) -> Vec<Line<'static>> {
    let hidden = hidden_breakdown(
        &app.items,
        app.comments_only,
        app.show_all,
        app.path_filter.as_ref().map(|(_, g)| g),
    );
    build_audit(
        &app.items,
        app.view.len(),
        &hidden,
        &app.ledger,
        app.path_filter.as_ref().map(|(p, _)| p.as_str()),
    )
    .into_iter()
    .map(prose)
    .collect()
}

/// `:delta` — what changed since the last run of this review
fn delta_lines(app: &App) -> Result<Vec<String>, String> {
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
        String::new(),
    ];
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
    Ok(lines)
}

/// `:note` — set (or, with no text, clear) the note on the selected hunk
fn set_note(app: &mut App, text: &str) -> Result<(), String> {
    let Some(key) = note_key(&app.items[app.sel]) else {
        return Err(
            "this hunk declares no symbol to anchor a note to (try one that defines something)"
                .to_string(),
        );
    };
    if text.is_empty() {
        app.notes.remove(&key);
    } else {
        app.notes.insert(key, text.to_string());
    }
    if let Some(p) = app.notes_path.as_deref() {
        save_notes(p, &app.notes);
    }
    Ok(())
}

/// The code-pane lines `:comment` acts on, 1-based: the `v` selection, or
/// the cursor's line.
pub(super) fn comment_lines(app: &App) -> (usize, usize) {
    let here = app.cursor.line + 1;
    match app.selection {
        Some(anchor) => {
            let a = anchor + 1;
            (a.min(here), a.max(here))
        }
        None => (here, here),
    }
}

/// `:comment` — add or replace the comment on the selected lines, or with no
/// text delete every comment touching them
fn set_comment(app: &mut App, text: &str) -> Result<(), String> {
    let path = app.items[app.sel].path.clone();
    let Some((_, lines)) = app.sources.get(&path) else {
        return Err(format!("{path} has no new side to comment on"));
    };
    let (start, end) = comment_lines(app);
    let touches = |c: &LineComment| c.path == path && c.start <= end && start <= c.end;
    if text.is_empty() {
        let before = app.comments.len();
        app.comments.retain(|c| !touches(c));
        if app.comments.len() == before {
            return Err("no comment on these lines to delete".to_string());
        }
    } else {
        let fresh = LineComment::new(&path, start, end, text, lines);
        match app
            .comments
            .iter_mut()
            .find(|c| c.path == path && (c.start, c.end) == (start, end))
        {
            Some(c) => *c = fresh,
            None => app.comments.push(fresh),
        }
    }
    app.selection = None;
    if let Some(p) = app.comments_path.as_deref() {
        save_comments(p, &app.comments);
    }
    Ok(())
}

/// every comment on a file of this review, in file and line order
pub(super) fn review_comments(app: &App) -> Vec<&LineComment> {
    let mut out: Vec<&LineComment> = app
        .comments
        .iter()
        .filter(|c| app.items.iter().any(|it| it.path == c.path))
        .collect();
    out.sort_by(|a, b| (&a.path, a.start).cmp(&(&b.path, b.start)));
    out
}

fn comment_list(app: &App) -> Vec<String> {
    review_comments(app)
        .into_iter()
        .map(|c| {
            let stale = if c.stale {
                "  (lines changed since)"
            } else {
                ""
            };
            format!("{}  {}{stale}", c.at(), c.text)
        })
        .collect()
}

/// `:mode [ledger|hunks]` — bare `:mode` toggles
fn parse_mode(arg: &str, current: ViewMode) -> Result<ViewMode, String> {
    Ok(match arg {
        "" => match current {
            ViewMode::Ledger => ViewMode::Hunks,
            ViewMode::Hunks => ViewMode::Ledger,
        },
        "ledger" | "symbols" | "symbol" => ViewMode::Ledger,
        "hunks" | "hunk" => ViewMode::Hunks,
        other => return Err(format!("unknown mode: {other} (ledger, hunks)")),
    })
}

/// the visible hunks as quickfix entries, in reading order
fn quickfix_hunks(app: &App) -> Vec<QfHunk> {
    app.view
        .iter()
        .map(|&i| {
            let it = &app.items[i];
            let warn = it.findings.iter().any(|f| {
                f.source == ordo::model::FindingSource::Catalog
                    || f.level != ordo::model::Level::Note
            });
            QfHunk {
                filename: it.path.clone(),
                lnum: it.new_range[0],
                kind: qf_kind(warn, app.reviewed[i]),
                cluster: it.cluster.map(|n| format!("c{n}")),
                text: it.rationale.clone(),
            }
        })
        .collect()
}
