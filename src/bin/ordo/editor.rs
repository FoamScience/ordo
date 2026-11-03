// ----------------------------------------------------------------------- edit
use crate::git::git;
use crate::App;

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
pub(super) fn edit_target(app: &App) -> (String, usize, bool) {
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
