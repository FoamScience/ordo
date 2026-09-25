// --------------------------------------------------------------------- watch
//! `--watch`: notice that the working tree has drifted from the loaded review.
//! A background thread fingerprints the tree every couple of seconds — `git
//! status` plus the size and mtime of each changed path, and `HEAD` — with no
//! file-watcher crate: polling needs no per-platform backend and misses
//! nothing on network or overlay filesystems. A burst of writes (an agent
//! mid-edit) settles for a quiet window before the review counts as stale.

use crate::git::git;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What the reviewer asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum WatchMode {
    #[default]
    Off,
    /// say the review is stale; `r` reloads
    Hint,
    /// reload by itself, when the reviewer is not in the middle of something
    Auto,
}

impl WatchMode {
    pub(super) fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(WatchMode::Off),
            "on" | "hint" => Some(WatchMode::Hint),
            "auto" => Some(WatchMode::Auto),
            _ => None,
        }
    }
}

/// How often the watcher thread looks.
pub(super) const POLL: Duration = Duration::from_secs(2);
/// A burst of writes counts as one change once the tree has been quiet this long.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(1500);
/// `auto` never reloads within this long of the reviewer's last key.
pub(super) const IDLE: Duration = Duration::from_secs(3);

/// The working tree as the review sees it: `HEAD`, and per changed path its
/// status, size and mtime. The mtime is what catches a second edit to a file
/// that was already modified — its status line alone does not change.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(super) struct Fingerprint {
    pub(super) head: String,
    pub(super) files: BTreeMap<String, String>,
}

impl Fingerprint {
    /// The paths whose entry differs between the two, either way round.
    pub(super) fn changed(&self, other: &Fingerprint) -> usize {
        let a = &self.files;
        let b = &other.files;
        a.iter().filter(|(k, v)| b.get(*k) != Some(*v)).count()
            + b.keys().filter(|k| !a.contains_key(*k)).count()
    }
}

/// The fingerprint of the repository at `dir`, from `git status --porcelain
/// -z` (untracked included, ignored excluded) and `stat`.
pub(super) fn fingerprint(dir: &str) -> Fingerprint {
    let head = git(&["-C", dir, "rev-parse", "HEAD"]).trim().to_string();
    let status = git(&[
        "-C",
        dir,
        "status",
        "--porcelain",
        "-z",
        "--untracked-files=all",
    ]);
    Fingerprint {
        head,
        files: parse_status(&status, |p| {
            std::fs::metadata(std::path::Path::new(dir).join(p))
                .ok()
                .map(|m| {
                    let mtime = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos());
                    format!("{} {mtime}", m.len())
                })
        }),
    }
}

/// `XY path\0`, a rename or copy followed by `orig\0`, keyed by path. `stat`
/// is asked about each path; a deleted one has nothing to say.
fn parse_status(status: &str, stat: impl Fn(&str) -> Option<String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut records = status.split('\0').filter(|r| !r.is_empty());
    while let Some(r) = records.next() {
        let (Some(xy), Some(path)) = (r.get(..2), r.get(3..)) else {
            continue;
        };
        let renamed = xy.contains('R') || xy.contains('C');
        let orig = if renamed { records.next() } else { None };
        let seen = stat(path).unwrap_or_default();
        out.insert(
            path.to_string(),
            format!("{xy} {} {seen}", orig.unwrap_or("")),
        );
    }
    out
}

/// Fingerprints from a background thread, `POLL` apart, for as long as the
/// receiver lives.
pub(super) fn spawn_watcher() -> mpsc::Receiver<Fingerprint> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while tx.send(fingerprint(".")).is_ok() {
            std::thread::sleep(POLL);
        }
    });
    rx
}

/// Why the review no longer matches the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Stale {
    /// files changed since it loaded
    Drift(usize),
    /// `HEAD` moved forward: something was committed, and `zz` lost it
    Committed(usize),
    /// `HEAD` moved somewhere that does not contain the old one: a rebase or
    /// a branch switch, which is never reloaded on its own
    BaseMoved,
    /// a review up to `wave/last`, and wave `n` was recorded since
    NewWave(usize),
}

impl Stale {
    pub(super) fn line(&self) -> String {
        match self {
            Stale::Drift(n) => format!("stale · {n} file{} changed · r reloads", s(*n)),
            Stale::Committed(n) => format!(
                "stale · committed, {n} file{} changed · r reloads, :e main...zz keeps the commits in view",
                s(*n)
            ),
            Stale::BaseMoved => "stale · HEAD moved (rebase or branch switch) · r reloads".to_string(),
            Stale::NewWave(n) => format!("stale · wave {n} recorded · r reloads"),
        }
    }
}

fn s(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The fingerprint the loaded review was read against, and what the watcher
/// has seen since.
#[derive(Default)]
pub(super) struct Drift {
    pub(super) loaded: Option<Fingerprint>,
    /// the latest fingerprint, and since when it has read that way
    last: Option<(Fingerprint, Instant)>,
}

impl Drift {
    pub(super) fn new(loaded: Fingerprint) -> Self {
        Drift {
            loaded: Some(loaded),
            last: None,
        }
    }

    /// Take in one fingerprint seen at `now`. The review is stale once the
    /// tree differs from what it loaded *and* has held still for `debounce`;
    /// `descends` answers whether a new `HEAD` contains the old one.
    pub(super) fn observe(
        &mut self,
        fp: Fingerprint,
        now: Instant,
        debounce: Duration,
        descends: impl Fn(&str, &str) -> bool,
    ) -> Option<Stale> {
        let since = match &self.last {
            Some((prev, at)) if *prev == fp => *at,
            _ => now,
        };
        self.last = Some((fp.clone(), since));
        let loaded = self.loaded.as_ref()?;
        if *loaded == fp || now.duration_since(since) < debounce {
            return None;
        }
        let n = loaded.changed(&fp);
        Some(if loaded.head == fp.head {
            Stale::Drift(n)
        } else if descends(&loaded.head, &fp.head) {
            Stale::Committed(n)
        } else {
            Stale::BaseMoved
        })
    }
}

/// Does `new` contain `old` — did `HEAD` only move forward?
pub(super) fn descends(old: &str, new: &str) -> bool {
    std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", old, new])
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(head: &str, files: &[(&str, &str)]) -> Fingerprint {
        Fingerprint {
            head: head.to_string(),
            files: files
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn status_parsing_keeps_untracked_and_skips_a_rename_source() {
        let got = parse_status(" M a.rs\0?? new.rs\0R  b.rs\0old_b.rs\0", |p| {
            Some(format!("stat:{p}"))
        });
        let keys: Vec<&str> = got.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["a.rs", "b.rs", "new.rs"]);
        assert!(got["b.rs"].contains("old_b.rs"), "{got:?}");
    }

    #[test]
    fn an_edit_to_an_already_modified_file_changes_the_fingerprint() {
        let a = parse_status(" M a.rs\0", |_| Some("10 1".into()));
        let b = parse_status(" M a.rs\0", |_| Some("12 2".into()));
        assert_ne!(a, b);
    }

    #[test]
    fn stale_only_after_the_tree_holds_still_for_the_debounce() {
        let t0 = Instant::now();
        let loaded = fp("h1", &[("a.rs", "1")]);
        let mut d = Drift::new(loaded.clone());
        let deb = Duration::from_millis(1500);
        let never = |_: &str, _: &str| false;
        assert_eq!(d.observe(loaded.clone(), t0, deb, never), None);
        let edited = fp("h1", &[("a.rs", "2")]);
        assert_eq!(
            d.observe(edited.clone(), t0, deb, never),
            None,
            "just changed"
        );
        let edited_more = fp("h1", &[("a.rs", "3"), ("b.rs", "1")]);
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(
            d.observe(edited_more.clone(), t1, deb, never),
            None,
            "still being written"
        );
        let t2 = t1 + Duration::from_secs(2);
        assert_eq!(
            d.observe(edited_more, t2, deb, never),
            Some(Stale::Drift(2))
        );
    }

    #[test]
    fn a_tree_back_where_it_loaded_is_not_stale() {
        let t0 = Instant::now();
        let loaded = fp("h1", &[]);
        let mut d = Drift::new(loaded.clone());
        let later = t0 + Duration::from_secs(10);
        assert_eq!(d.observe(loaded, later, DEBOUNCE, |_, _| false), None);
    }

    #[test]
    fn head_moving_forward_is_a_commit_and_elsewhere_is_a_moved_base() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(10);
        let loaded = fp("h1", &[("a.rs", "1")]);
        let moved = fp("h2", &[]);
        let mut d = Drift::new(loaded.clone());
        d.observe(moved.clone(), t0, DEBOUNCE, |_, _| true);
        assert_eq!(
            d.observe(moved.clone(), later, DEBOUNCE, |_, _| true),
            Some(Stale::Committed(1))
        );
        let mut d = Drift::new(loaded);
        d.observe(moved.clone(), t0, DEBOUNCE, |_, _| false);
        assert_eq!(
            d.observe(moved, later, DEBOUNCE, |_, _| false),
            Some(Stale::BaseMoved)
        );
    }

    /// A real repository: what the watcher must see, and what it must not.
    #[test]
    fn the_fingerprint_sees_edits_untracked_files_and_renames_not_ignored_ones() {
        let dir = std::env::temp_dir().join(format!("ordo-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let sh = |cmd: &str| {
            std::process::Command::new("sh")
                .args(["-c", cmd])
                .current_dir(&dir)
                .output()
                .unwrap();
        };
        sh(
            "git init -q && git config user.email t@t && git config user.name t \
            && printf 'a\n' > a.rs && printf 'target\n' > .gitignore \
            && git add . && git commit -qm init",
        );
        let base = fingerprint(d);
        sh("mkdir -p target && printf x > target/out.o");
        assert_eq!(fingerprint(d), base, "an ignored file is not drift");
        sh("printf 'b\n' >> a.rs");
        let edited = fingerprint(d);
        assert_ne!(edited, base, "an edit is");
        sh("printf 'c\n' >> a.rs");
        assert_ne!(
            fingerprint(d),
            edited,
            "so is a second edit to a modified file"
        );
        sh("git checkout -q a.rs && printf n > new.rs");
        assert_ne!(fingerprint(d), base, "and an untracked file");
        sh("rm new.rs && git mv a.rs b.rs");
        assert_ne!(fingerprint(d), base, "and a rename");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_modes_parse() {
        assert_eq!(WatchMode::parse("on"), Some(WatchMode::Hint));
        assert_eq!(WatchMode::parse("auto"), Some(WatchMode::Auto));
        assert_eq!(WatchMode::parse("sometimes"), None);
    }
}
