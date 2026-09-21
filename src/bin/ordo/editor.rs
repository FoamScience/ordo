// ----------------------------------------------------------------------- edit
use crate::git::git;
use crate::plural;
use crate::prose;
use crate::App;
use crate::Popup;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// `$VISUAL` then `$EDITOR`, whitespace-split so trailing arguments (`code
/// --wait`, `emacsclient -nw`) survive; `vi` when neither is set or both are
/// blank — a POSIX-guaranteed binary rather than a guess.
pub(super) fn resolve_editor() -> Vec<String> {
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

pub(super) fn split_command(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// The location arguments for one program, keyed by its basename (not the
/// full `$EDITOR` string, which may carry flags ahead of the program). An
/// editor outside this curated set gets just the file — passing a guessed
/// `+LINE` or `file:LINE` syntax to something that doesn't understand it risks
/// it being read as a second filename.
pub(super) fn editor_args(basename: &str, path: &str, line: usize) -> Vec<String> {
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
pub(super) fn build_command(
    spec: &[String],
    path: &str,
    line: usize,
) -> Option<(String, Vec<String>)> {
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

// -------------------------------------------------------- suspend for an editor

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
pub(super) fn open_editor(app: &mut App, terminal: &mut ratatui::DefaultTerminal) {
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
pub(super) struct QfHunk {
    pub(super) filename: String,
    pub(super) lnum: usize,
    /// `Some('W')`/`Some('I')` for the sign column; `None` renders as `''`
    /// (no sign) — see `qf_kind`.
    pub(super) kind: Option<char>,
    /// the hunk's cluster label (`"cluster N"`), prefixed onto the item's
    /// text. NOT emitted as vim's `module` key: vim renders `module` *instead
    /// of* the filename in the quickfix window, which would cost a reviewer
    /// the one column they navigate by.
    pub(super) cluster: Option<String>,
    pub(super) text: String,
}

/// `:quickfix`'s per-hunk `type`: `'W'` when the hunk carries a warn-level
/// rule hit or an advisory (warn wins when both apply), `'I'` when it's
/// marked reviewed, otherwise no sign at all.
pub(super) fn qf_kind(warn: bool, reviewed: bool) -> Option<char> {
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
pub(super) fn quickfix_script(rev: &str, strategy: &str, hunks: &[QfHunk]) -> String {
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
pub(super) fn is_vim_family(spec: &[String]) -> bool {
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
pub(super) fn write_quickfix_script(script: &str) -> Result<PathBuf, String> {
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
pub(super) fn open_quickfix_editor(
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    path: &Path,
) {
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
