// --------------------------------------------------------------------- waves
//! Progressive review: the working tree recorded at each agent turn boundary
//! as a chain of commits, one per wave, so any two waves diff like revisions.
//! They live under `refs/worktree/`, which keeps each linked worktree's chain
//! its own and still roots them for `git gc`. The user's index, branches and
//! GitButler workspace are never touched: a wave's tree is built through a
//! scratch copy of the index.

use std::process::Command;

const REFS: &str = "refs/worktree/ordo/waves";

/// The recorded waves, by number, each with its commit.
pub(super) fn list(repo: &str) -> Vec<(usize, String)> {
    let out = git_env(
        repo,
        &[],
        &["for-each-ref", "--format=%(refname) %(objectname)", REFS],
    )
    .unwrap_or_default();
    let mut waves: Vec<(usize, String)> = out
        .lines()
        .filter_map(|l| {
            let (name, sha) = l.split_once(' ')?;
            let n = name.rsplit('/').next()?.parse().ok()?;
            Some((n, sha.to_string()))
        })
        .collect();
    waves.sort();
    waves
}

/// `wave/N` and `wave/last` as the refs they name, on either side of a range;
/// anything else is returned as it was.
pub(super) fn expand(arg: &str) -> String {
    for sep in ["...", ".."] {
        if let Some((a, b)) = arg.split_once(sep) {
            return format!("{}{sep}{}", expand_one(a), expand_one(b));
        }
    }
    expand_one(arg)
}

fn expand_one(rev: &str) -> String {
    let Some(which) = rev.strip_prefix("wave/") else {
        return rev.to_string();
    };
    let n = match which {
        "last" => list(".").last().map(|(n, _)| *n),
        n => n.parse().ok(),
    };
    n.map_or_else(|| rev.to_string(), |n| format!("{REFS}/{n}"))
}

/// Git run in `repo` with extra environment; the stdout, or the first line of
/// stderr.
fn git_env(repo: &str, env: &[(&str, &str)], args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(["-C", repo])
        .args(args)
        .envs(env.iter().copied())
        .output()
        .map_err(|e| format!("git could not be run: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("git failed").to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Record the working tree as the next wave, `message` as its commit message.
/// `Ok(None)` when nothing changed since the last wave: an agent turn that
/// edited nothing is not a wave.
pub(super) fn record(repo: &str, message: &str) -> Result<Option<usize>, String> {
    // this worktree's own git dir, absolute so the paths below do not depend
    // on where in the tree ordo was started
    let dir = std::path::PathBuf::from(git_env(repo, &[], &["rev-parse", "--absolute-git-dir"])?);
    // one scratch index per process: herdr-ordo's loop and `:wave` can record
    // at the same moment
    let scratch = dir.join(format!("ordo-wave-index-{}", std::process::id()));
    let index = dir.join("index");
    let scratch = scratch.to_string_lossy().into_owned();
    // a copy of the real index keeps its stat cache, so `add -A` hashes only
    // what changed; with no index yet (a fresh repo) git starts from empty
    if index.exists() {
        std::fs::copy(&index, &scratch).map_err(|e| format!("{scratch}: {e}"))?;
    }
    let with = [("GIT_INDEX_FILE", scratch.as_str())];
    let tree =
        git_env(repo, &with, &["add", "-A"]).and_then(|_| git_env(repo, &with, &["write-tree"]));
    let _ = std::fs::remove_file(&scratch);
    let tree = tree?;
    let waves = list(repo);
    let parent = match waves.last() {
        Some((_, sha)) => {
            if git_env(repo, &[], &["rev-parse", &format!("{sha}^{{tree}}")])? == tree {
                return Ok(None);
            }
            Some(sha.clone())
        }
        None => git_env(repo, &[], &["rev-parse", "--verify", "-q", "HEAD"]).ok(),
    };
    let n = waves.last().map_or(0, |(n, _)| n + 1);
    let who = [
        ("GIT_AUTHOR_NAME", "ordo"),
        ("GIT_AUTHOR_EMAIL", "ordo@localhost"),
        ("GIT_COMMITTER_NAME", "ordo"),
        ("GIT_COMMITTER_EMAIL", "ordo@localhost"),
    ];
    let subject = format!("wave {n}");
    let message = if message.trim().is_empty() {
        subject
    } else {
        format!("{subject}\n\n{}", message.trim())
    };
    let mut args = vec!["commit-tree", tree.as_str(), "-m", message.as_str()];
    if let Some(p) = parent.as_deref() {
        args.extend(["-p", p]);
    }
    let commit = git_env(repo, &who, &args)?;
    // the empty old value makes git refuse when the ref already exists, so of
    // two records racing for the same number one fails instead of one wave
    // silently replacing the other
    git_env(
        repo,
        &[],
        &["update-ref", &format!("{REFS}/{n}"), &commit, ""],
    )?;
    Ok(Some(n))
}

/// Forget every wave. The snapshots become unreachable and `git gc` takes
/// them in its own time.
pub(super) fn clear(repo: &str) -> Result<usize, String> {
    let waves = list(repo);
    for (n, _) in &waves {
        git_env(repo, &[], &["update-ref", "-d", &format!("{REFS}/{n}")])?;
    }
    Ok(waves.len())
}

const USAGE: &str = "\
usage: ordo wave [-m <message>]   record the working tree as the next wave
       ordo wave --list           the recorded waves
       ordo wave --clear          forget them all

The first wave is the starting point. Review one agent turn with
`ordo wave/2..wave/3`, everything so far with `ordo wave/0..wave/last`.
";

/// `ordo wave …`; the exit code.
pub(super) fn cli(args: &[String]) -> i32 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        [] => record(".", ""),
        ["-m", message] => record(".", message),
        ["--list"] => {
            for (n, sha) in list(".") {
                let subject = crate::git::git(&["log", "-1", "--format=%s%n%b", &sha]);
                let intent = subject.lines().skip(1).find(|l| !l.trim().is_empty());
                println!(
                    "wave/{n}  {}  {}",
                    &sha[..sha.len().min(12)],
                    intent.unwrap_or("")
                );
            }
            return 0;
        }
        ["--clear"] => {
            return match clear(".") {
                Ok(n) => {
                    println!("forgot {n} wave{}", if n == 1 { "" } else { "s" });
                    0
                }
                Err(e) => {
                    eprintln!("ordo wave: {e}");
                    1
                }
            };
        }
        _ => {
            eprint!("{USAGE}");
            return 2;
        }
    };
    match result {
        Ok(Some(n)) => {
            println!("wave/{n}");
            0
        }
        Ok(None) => {
            println!("nothing changed since the last wave");
            0
        }
        Err(e) => {
            eprintln!("ordo wave: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_rev_is_left_alone() {
        assert_eq!(expand("HEAD~2"), "HEAD~2");
        assert_eq!(expand("main...zz"), "main...zz");
        assert_eq!(expand("wave/x"), "wave/x");
    }

    #[test]
    fn wave_numbers_expand_on_both_sides_of_a_range() {
        assert_eq!(expand("wave/2..wave/3"), format!("{REFS}/2..{REFS}/3"));
        assert_eq!(expand("main...wave/4"), format!("main...{REFS}/4"));
    }

    /// A real repository: what a wave records, and what it leaves alone.
    #[test]
    fn a_wave_records_the_tree_and_leaves_the_index_alone() {
        let dir = std::env::temp_dir().join(format!("ordo-waves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let sh = |cmd: &str| {
            let out = Command::new("sh")
                .args(["-c", cmd])
                .current_dir(&dir)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        sh(
            "git init -q && git config user.email t@t && git config user.name t \
            && printf 'a\\n' > a.rs && printf 'target\\n' > .gitignore \
            && git add . && git commit -qm init",
        );
        assert_eq!(record(d, ""), Ok(Some(0)), "the starting point");
        assert_eq!(record(d, ""), Ok(None), "nothing changed");
        sh("printf 'b\\n' >> a.rs && printf n > new.rs && mkdir -p target && printf x > target/o");
        let status = sh("git status --porcelain");
        assert_eq!(record(d, "add b").unwrap(), Some(1));
        assert_eq!(
            sh("git status --porcelain"),
            status,
            "the index is untouched"
        );
        let files = sh(&format!("git ls-tree -r --name-only {REFS}/1"));
        assert!(
            files.contains("new.rs") && !files.contains("target/o"),
            "{files}"
        );
        assert_eq!(
            sh(&format!("git rev-parse {REFS}/1^")),
            sh(&format!("git rev-parse {REFS}/0"))
        );
        assert!(sh(&format!("git log -1 --format=%B {REFS}/1")).contains("add b"));
        assert_eq!(list(d).len(), 2);
        assert_eq!(clear(d), Ok(2));
        assert!(list(d).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
