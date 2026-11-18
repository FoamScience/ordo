// ------------------------------------------------- dismissed catalog findings
//! `:dismiss` records that a catalog finding was not worth the reader's time
//! on one hunk. Whether a construct deserves attention depends on the
//! codebase, so once one rule has been dismissed on enough hunks of one
//! repository the reviewer suggests the `.ordo/rules.toml` change that would
//! quiet it there. It only suggests: nothing here writes a rules file.

use crate::marks::snap_key;
use crate::App;
use crate::Popup;
use ordo_diff::model::FindingSource;
use ordo_diff::model::Level;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

/// rule name -> the hunks (`snap_key`) it was dismissed on
pub(super) type Dismissals = BTreeMap<String, BTreeSet<String>>;

/// how many distinct hunks of one repository before a rule earns a suggestion
pub(super) const SUGGEST_AFTER: usize = 3;

/// Beside the notes, one file per repository.
pub(super) fn dismissed_file_path(notes_path: &Path) -> Option<PathBuf> {
    let name = notes_path.file_name()?;
    Some(notes_path.parent()?.parent()?.join("dismissed").join(name))
}

/// Degrades to "nothing dismissed" on any failure, as the notes do.
pub(super) fn load_dismissals(path: &Path) -> Dismissals {
    read_dismissals(path).unwrap_or_default()
}

/// `None` when the file is there and cannot be read, which a write must not
/// take for "nothing dismissed".
fn read_dismissals(path: &Path) -> Option<Dismissals> {
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str(&t).ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Dismissals::new()),
        Err(_) => None,
    }
}

fn save_dismissals(path: &Path, d: &Dismissals) -> std::io::Result<()> {
    crate::marks::write_atomic(path, &serde_json::to_string(d)?)
}

/// The catalog rules dismissed on item `i`, for the view.
pub(super) fn dismissed_on(app: &App, i: usize) -> Vec<&str> {
    let key = snap_key(&app.items[i]);
    app.dismissed
        .iter()
        .filter(|(_, keys)| keys.contains(&key))
        .map(|(name, _)| name.as_str())
        .collect()
}

/// `:dismiss [rule]` — toggle a dismissal of one of the selected hunk's
/// catalog findings; bare when the hunk has only one catalog rule.
pub(super) fn dismiss(app: &mut App, arg: &str) -> Result<(), String> {
    let it = &app.items[app.sel];
    let mut names: Vec<(&str, Level)> = it
        .findings
        .iter()
        .filter(|f| f.source == FindingSource::Catalog)
        .map(|f| (f.name.as_str(), f.level))
        .collect();
    // the loudest first, so the suggestion lowers what the reader saw
    names.sort_by_key(|&(n, l)| (n, std::cmp::Reverse(l)));
    names.dedup_by_key(|(n, _)| *n);
    let (name, level) = match (arg, names.as_slice()) {
        (_, []) => return Err("this hunk has no catalog finding to dismiss".to_string()),
        ("", [one]) => *one,
        ("", many) => {
            let all: Vec<&str> = many.iter().map(|(n, _)| *n).collect();
            return Err(format!("which one? :dismiss <rule> — {}", all.join(", ")));
        }
        (want, _) => *names
            .iter()
            .find(|(n, _)| *n == want)
            .ok_or_else(|| format!("no catalog finding named '{want}' on this hunk"))?,
    };
    let (name, key) = (name.to_string(), snap_key(it));
    // another reviewer of this repository may have written since this one
    // loaded: change their file, not this session's copy of it
    if let Some(p) = app.dismissed_path.as_deref() {
        app.dismissed = read_dismissals(p)
            .ok_or_else(|| format!("{} cannot be read; not overwriting it", p.display()))?;
    }
    let keys = app.dismissed.entry(name.clone()).or_default();
    let n = if keys.remove(&key) {
        if keys.is_empty() {
            app.dismissed.remove(&name);
        }
        None
    } else {
        keys.insert(key);
        Some(keys.len())
    };
    if let Some(p) = app.dismissed_path.as_deref() {
        save_dismissals(p, &app.dismissed).map_err(|e| {
            format!("the dismissal holds for this session, but was not written: {e}")
        })?;
    }
    match n {
        Some(n) if n >= SUGGEST_AFTER => {
            app.popup = Some(Popup::new(
                format!("{name}: dismissed on {n} hunks"),
                suggestion(&name, level, n),
            ));
        }
        Some(n) => {
            app.notice = Some(format!(
                "dismissed {name} here ({n} of {SUGGEST_AFTER} before ordo suggests a level for this repository)"
            ));
        }
        None => app.notice = Some(format!("{name} is no longer dismissed here")),
    }
    Ok(())
}

/// The `.ordo/rules.toml` lines that would quiet `name` in this repository:
/// one level lower, or `disable` when it is already a note.
pub(super) fn suggestion(name: &str, level: Level, n: usize) -> Vec<String> {
    #[derive(serde::Serialize)]
    struct Levels<'a> {
        levels: BTreeMap<&'a str, Level>,
    }
    #[derive(serde::Serialize)]
    struct Disable<'a> {
        disable: [&'a str; 1],
    }
    let (what, toml) = match level {
        Level::Verdict | Level::Warn => {
            let lower = if level == Level::Verdict {
                Level::Warn
            } else {
                Level::Note
            };
            (
                format!("lower it from {} to {}", level.as_str(), lower.as_str()),
                toml::to_string(&Levels {
                    levels: BTreeMap::from([(name, lower)]),
                }),
            )
        }
        Level::Note => (
            "it is already a note, so turn it off".to_string(),
            toml::to_string(&Disable { disable: [name] }),
        ),
    };
    let mut out = vec![
        format!("You dismissed {name} on {n} hunks of this repository."),
        "Whether a construct is worth flagging depends on the codebase; to".to_string(),
        format!("{what} here, add this to <repo>/.ordo/rules.toml:"),
        String::new(),
    ];
    out.extend(
        toml.expect("a name and a level serialize")
            .lines()
            .map(|l| format!("    {l}")),
    );
    out.extend([
        String::new(),
        "ordo has changed nothing. :config sets the same level in your own".to_string(),
        "rules file instead, for every repository; `ordo help rules` explains both.".to_string(),
    ]);
    out
}
