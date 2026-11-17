"""What the corpus scripts share: the manifest, git plumbing, and one engine
run per commit. Imported by gold-sample.py and corpus-judge.py."""
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENGINE = ROOT / "target/release/ordo-engine"


def repos():
    """(name, lang, rev) per `[[repo]]` in corpus/manifest.toml."""
    text = (ROOT / "corpus/manifest.toml").read_text()
    field = lambda b, k: (m.group(1) if (m := re.search(rf'^{k}\s*=\s*"?([^"\n]+)"?', b, re.M)) else None)
    for b in text.split("[[repo]]")[1:]:
        yield field(b, "name"), field(b, "lang"), field(b, "rev")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True,
                          errors="replace").stdout


def commit_input(repo, sha):
    """(parent sha, engine input) for one commit, or (None, None) at the clone's
    edge or when nothing textual changed. Every text file goes in; the engine
    flags what it has no grammar for and the caller drops those, so the
    supported-extension list lives in one place."""
    parent = git(repo, "rev-parse", "--verify", "-q", f"{sha}^").strip()
    if not parent:
        return None, None
    inp = range_input(repo, parent, sha)
    return (parent, inp) if inp else (None, None)


def range_input(repo, base, head):
    """Engine input for everything between two revisions, or None when no
    text changed."""
    names = [l.split("\t", 2)[2] for l in git(repo, "diff", "--numstat", "--no-renames", base, head).splitlines()
             if not l.startswith("-\t-\t")]
    if not names:
        return None
    return {"changes": [{"path": p, "old": git(repo, "show", f"{base}:{p}"),
                         "new": git(repo, "show", f"{head}:{p}")} for p in names]}


def with_features(repo, sha, inp):
    """`inp` asking the engine for hunch's per-hunk feature vectors, with the
    history of the files it changes: `sha`'s ancestors, never `sha` itself or
    what came after. The engine keeps only the ancestors inside its window,
    reaching back from `sha`'s own commit time; the cutoff here, a day wider,
    only bounds the log. A file `sha` itself renames, which `range_input` shows
    as brand-new, takes its history from the old name. A shallow clone whose
    boundary falls inside that window would undercount, so its commits get no
    history at all, as a stdin diff does."""
    at = int(git(repo, "show", "-s", "--format=%ct", sha).strip())
    since = at - 91 * 86400
    inp = {**inp, "options": {**inp.get("options", {}), "features": True}}
    shallow = Path(repo) / git(repo, "rev-parse", "--git-path", "shallow").strip()
    if shallow.is_file():
        edges = shallow.read_text().split()
        if any(int(t) > since for t in git(repo, "show", "-s", "--format=%ct", *edges).split()):
            return inp
    log = git(repo, "-c", "core.quotePath=false", "log", "-M", "--topo-order", "--name-status", "--format=%x1e%ct%x09%s",
              f"--max-age={since}", f"{sha}^")
    renamed = own_renames(repo, sha)
    heads = {c["path"]: renamed.get(c["path"], c["path"]) for c in inp["changes"]}
    return {**inp, "history": {"at": at, "commits": followed(log, heads)}}


def own_renames(repo, sha):
    """new path -> old path for the files `sha` renames, against its parent."""
    out = git(repo, "-c", "core.quotePath=false", "diff", "-M", "--name-status", "--diff-filter=R", f"{sha}^", sha)
    return {new: old for _, old, new in (l.split("\t") for l in out.splitlines())}


def followed(log, heads):
    """The commits of a `git log -M --topo-order --name-status` that touch the
    files of `heads` (path -> its name at the log's newest commit), each
    listing them by path, so a file renamed inside the window keeps the
    history from before the rename. The log is unfiltered because a pathspec
    would hide the rename's old side."""
    current = {}
    for p, n in heads.items():
        current.setdefault(n, []).append(p)
    commits = []
    for rec in log.split("\x1e")[1:]:
        head, *entries = rec.strip("\n").split("\n")
        time, _, subject = head.partition("\t")
        touched = []
        for e in filter(None, entries):
            status, *names = e.split("\t")
            if (ps := current.get(names[-1])) is not None:
                touched += ps
                if status.startswith("R"):
                    current.setdefault(names[0], []).extend(current.pop(names[-1]))
        if touched:
            commits.append({"time": int(time), "subject": subject, "paths": touched})
    return commits


def engine_output(inp):
    r = subprocess.run([ENGINE, "order", "--json"], input=json.dumps(inp), capture_output=True, text=True)
    return json.loads(r.stdout)


def template(rationale):
    return rationale.split(" ", 1)[0] if rationale else "change"
