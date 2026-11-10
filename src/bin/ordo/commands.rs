// -------------------------------------------------------------- command mode
use crate::build_globs;
use crate::build_items;
use crate::comments::load_comments;
use crate::comments::save_comments;
use crate::comments::LineComment;
use crate::compute_view;
use crate::config::ConfigUi;
use crate::facts::draft_rule;
use crate::git::command_failures;
use crate::group_reasons;
use crate::handoff::review_prompt;
use crate::handoff::send;
use crate::hidden_breakdown;
use crate::keys::ViewMode;
use crate::marks::load_notes;
use crate::marks::mark_key;
use crate::marks::note_key;
use crate::marks::save_notes;
use crate::marks::Delta;
use crate::plural;
use crate::select;
use crate::set_mode;
use crate::watch::WatchMode;
use crate::App;
use crate::Hidden;
use crate::Item;
use crate::Ledger;
use crate::PathGlobs;
use crate::Popup;
use ordo::model::Input;
use ordo::model::Options;
use ordo::model::Strategy;

/// One `:` command — name, a short argument hint for `:help` (empty when it
/// takes none), and the help line itself.
pub(super) struct Cmd {
    pub(super) name: &'static str,
    pub(super) args: &'static str,
    pub(super) help: &'static str,
}

/// The command table — the single source of truth for command names,
/// `:help`'s listing, and the browser's command palette. Adding a command here is
/// the only step that makes it discoverable; `execute_command`'s `match`
/// still has to know what to *do* with it, but a name present here with no
/// matching arm there falls into that match's `unknown command` case rather
/// than silently doing nothing. `e` and `yank` are the exception: `web::command`
/// answers them itself before `execute_command` sees the line.
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
        help: "toggle grouping the reading-order list: a group's symbol on its first row only, and folds",
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
        name: "only-wave",
        args: "<N|last|all>",
        help: "show only the hunks one wave changed; dep lines still reach the others",
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
        help: "select the first hunk of <path>",
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
    if hidden.wave > 0 {
        out.push(row(hidden.wave, "changed in another wave (:only-wave all)"));
    }

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

/// `:help`'s body, generated straight from `COMMANDS`, so the listing can't
/// name a command that doesn't exist or omit one that does.
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

/// Splits a typed command line (without its leading `:`) into the command
/// name and its argument, trimming both — `"filter   src/*  "` ->
/// `("filter", "src/*")`. A bare name or an empty line gets an empty argument.
pub(super) fn parse_command_line(line: &str) -> (&str, &str) {
    let line = line.trim();
    match line.split_once(char::is_whitespace) {
        Some((name, rest)) => (name, rest.trim()),
        None => (line, ""),
    }
}

fn parse_strategy(s: &str) -> Option<Strategy> {
    match s {
        "comprehension" => Some(Strategy::Comprehension),
        "defs-first" => Some(Strategy::DefsFirst),
        "file" => Some(Strategy::File),
        _ => None,
    }
}

/// `:strategy <name>` — re-runs `ordo::run` synchronously on the calling
/// thread rather than the worker-thread path `load` uses. The
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
            levels: app.levels.clone(),
            ..Options::default()
        },
        consumers: vec![],
    };
    let out = ordo::run(input);
    let mut items = build_items(&out);
    crate::waves::tag(&mut items, &app.waves.lines);
    let groups = group_reasons(&out);
    if items.is_empty() {
        return Err("that strategy leaves nothing to review".to_string());
    }
    let reviewed: Vec<bool> = items
        .iter()
        .map(|it| mark_key(&app.rev, it, &app.sources).is_some_and(|k| app.marks.contains_key(&k)))
        .collect();
    let glob = app.path_filter.as_ref().map(|(_, g)| g);
    let view = compute_view(&items, app.comments_only, app.show_all, glob, app.only_wave);
    if view.is_empty() {
        return Err("that strategy leaves nothing visible under the current filters".to_string());
    }
    app.items = items;
    app.reviewed = reviewed;
    app.view = view;
    // the new items arrive keyed for ledger mode; re-key them for the mode
    // the reviewer is in, against this run's own ledger
    app.group_reasons = groups;
    app.symbol_ledger = out.ledger.clone();
    app.summary = out.summary.clone();
    let led = app.symbol_ledger.clone();
    set_mode(app, app.mode, &led);
    app.popup = None;
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
    only_wave: Option<usize>,
) -> Result<(), String> {
    let glob = path_filter.as_ref().map(|(_, g)| g);
    let view = compute_view(&app.items, comments_only, show_all, glob, only_wave);
    if view.is_empty() {
        return Err("that filter combination leaves nothing to review".to_string());
    }
    app.comments_only = comments_only;
    app.show_all = show_all;
    app.path_filter = path_filter;
    app.only_wave = only_wave;
    app.view = view;
    if !app.view.contains(&app.sel) {
        select(app, app.view[0]);
    }
    Ok(())
}

/// `gw`/`gW`: the next or previous wave on its own, stepping from all of them
/// to the first (or last) and back to all past the end. The status line says
/// which wave and what it was asked.
pub(super) fn step_wave(app: &mut App, forward: bool) {
    let mut waves: Vec<usize> = app.items.iter().filter_map(|it| it.wave).collect();
    waves.sort_unstable();
    waves.dedup();
    let (Some(&first), Some(&last)) = (waves.first(), waves.last()) else {
        app.notice = Some("no waves in this review — :e wave/0..wave/last".to_string());
        return;
    };
    let at = app
        .only_wave
        .and_then(|w| waves.iter().position(|x| *x == w));
    let next = match (at, forward) {
        (None, true) => Some(first),
        (None, false) => Some(last),
        (Some(i), true) => waves.get(i + 1).copied(),
        (Some(i), false) => i.checked_sub(1).map(|i| waves[i]),
    };
    let glob = app.path_filter.clone();
    if set_filters(app, app.comments_only, app.show_all, glob, next).is_err() {
        app.notice = Some(format!(
            "wave {} has nothing under the current filters",
            next.map_or("all".to_string(), |w| w.to_string())
        ));
        return;
    }
    app.notice = Some(match next {
        None => "all waves".to_string(),
        Some(w) => match app.waves.asked(w) {
            Some(asked) => format!("wave {w} · asked: {asked}"),
            None => format!("wave {w}"),
        },
    });
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
    Ok(())
}

/// Runs a full command line (without its leading `:`). `Err` carries a
/// message for the reader rather than silently doing nothing, so a typo or a
/// bad argument is never mistaken for "nothing happened".
pub(super) fn execute_command(app: &mut App, line: &str) -> Result<(), String> {
    let (name, arg) = parse_command_line(line);
    match name {
        "" => Ok(()),
        "help" => {
            app.popup = Some(Popup::new("commands", build_command_help()));
            Ok(())
        }
        "config" => {
            app.config = Some(ConfigUi::open(app));
            Ok(())
        }
        "rules" => {
            app.popup = Some(Popup::new("rules", app.rules_report.clone()));
            Ok(())
        }
        "audit" => {
            app.popup = Some(Popup::new("audit", audit_lines(app)));
            Ok(())
        }
        "only-comments" => {
            set_filters(
                app,
                !app.comments_only,
                app.show_all,
                app.path_filter.clone(),
                app.only_wave,
            )?;
            Ok(())
        }
        "all" => {
            set_filters(
                app,
                app.comments_only,
                !app.show_all,
                app.path_filter.clone(),
                app.only_wave,
            )?;
            Ok(())
        }
        "filter" => {
            let arg = arg.trim();
            let next = if arg.is_empty() {
                None
            } else {
                let globs = build_globs(&[arg.to_string()])?;
                Some((arg.to_string(), globs))
            };
            set_filters(app, app.comments_only, app.show_all, next, app.only_wave)?;
            Ok(())
        }
        "strategy" => {
            run_strategy(app, arg.trim())?;
            Ok(())
        }
        "group" => {
            app.show_groups = !app.show_groups;
            Ok(())
        }
        "rule" => {
            app.popup = Some(Popup::new(
                "draft rule".to_string(),
                draft_rule(app, app.sel),
            ));
            Ok(())
        }
        "delta" => {
            let lines = delta_lines(app)?;
            app.popup = Some(Popup::new("since you last looked", lines));
            Ok(())
        }
        "note" => {
            set_note(app, arg.trim())?;
            Ok(())
        }
        "watch" => {
            let mode = match arg.trim() {
                "" if app.watch == WatchMode::Off => WatchMode::Hint,
                "" => WatchMode::Off,
                a => WatchMode::parse(a)
                    .ok_or_else(|| format!("usage: :watch [on|off|auto], not '{a}'"))?,
            };
            if mode != WatchMode::Off && !app.uncommitted && !app.rev.contains("wave/last") {
                return Err(format!(
                    "nothing to watch: {} is committed — :e zz or :e main...zz reviews work that is still changing",
                    app.rev
                ));
            }
            app.watch = mode;
            if mode == WatchMode::Off {
                app.stale = None;
            }
            Ok(())
        }
        "only-wave" => {
            let waves: Vec<usize> = app.items.iter().filter_map(|it| it.wave).collect();
            let Some(&last) = waves.iter().max() else {
                return Err(
                    "no waves in this review — review a range of them, like :e wave/0..wave/last"
                        .to_string(),
                );
            };
            let wave = match arg.trim() {
                "all" | "" => None,
                "last" => Some(last),
                n => Some(
                    n.parse()
                        .map_err(|_| format!("usage: :only-wave <N|last|all>, not '{n}'"))?,
                ),
            };
            set_filters(
                app,
                app.comments_only,
                app.show_all,
                app.path_filter.clone(),
                wave,
            )
            .map_err(|_| format!("wave {} changed nothing in view", arg.trim()))?;
            Ok(())
        }
        "wave" => {
            app.notice = Some(match crate::waves::record(".", arg)? {
                Some(n) => format!("recorded wave/{n}"),
                None => "nothing changed since the last wave".to_string(),
            });
            Ok(())
        }
        "comment" => {
            set_comment(app, arg.trim())?;
            Ok(())
        }
        "comments" => {
            let lines = comment_list(app);
            if lines.is_empty() {
                return Err(
                    "no line comments in this review — `v` to select, `c` to comment".to_string(),
                );
            }
            app.popup = Some(Popup::new("comments", lines));
            Ok(())
        }
        "send" => {
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
            let head = format!("sent to `{}`", send(&prompt)?);
            app.popup = Some(Popup::new(
                head,
                prompt.lines().map(str::to_string).collect(),
            ));
            Ok(())
        }
        "mode" => {
            let to = parse_mode(arg.trim(), app.mode)?;
            let led = app.symbol_ledger.clone();
            set_mode(app, to, &led);
            Ok(())
        }
        "goto" => {
            run_goto(app, arg.trim())?;
            Ok(())
        }
        other => Err(format!("unknown command '{other}' — :help lists them")),
    }
}

/// `:audit` — what the review covers and what the filters hide
fn audit_lines(app: &App) -> Vec<String> {
    let hidden = hidden_breakdown(
        &app.items,
        app.comments_only,
        app.show_all,
        app.path_filter.as_ref().map(|(_, g)| g),
        app.only_wave,
    );
    build_audit(
        &app.items,
        app.view.len(),
        &hidden,
        &app.ledger,
        app.path_filter.as_ref().map(|(p, _)| p.as_str()),
    )
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
    // another reviewer of this repository may have written since this one
    // loaded: change their file, not this session's copy of it
    if let Some(p) = app.notes_path.as_deref() {
        app.notes = load_notes(p);
    }
    if text.is_empty() {
        app.notes.remove(&key);
    } else {
        app.notes.insert(key, text.to_string());
    }
    if let Some(p) = app.notes_path.as_deref() {
        save_notes(p, &app.notes)
            .map_err(|e| format!("the note holds for this session, but was not written: {e}"))?;
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
    if let Some(p) = app.comments_path.as_deref() {
        app.comments = load_comments(p);
        for c in &mut app.comments {
            if let Some((_, new)) = app.sources.get(&c.path) {
                c.relocate(new);
            }
        }
    }
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
        save_comments(p, &app.comments)
            .map_err(|e| format!("the comment holds for this session, but was not written: {e}"))?;
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
