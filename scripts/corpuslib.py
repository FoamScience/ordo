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
    names = [l.split("\t", 2)[2] for l in git(repo, "diff", "--numstat", "--no-renames", parent, sha).splitlines()
             if not l.startswith("-\t-\t")]
    if not names:
        return None, None
    inp = {"changes": [{"path": p, "old": git(repo, "show", f"{parent}:{p}"),
                        "new": git(repo, "show", f"{sha}:{p}")} for p in names]}
    return parent, inp


def engine_output(inp):
    r = subprocess.run([ENGINE, "order", "--json"], input=json.dumps(inp), capture_output=True, text=True)
    return json.loads(r.stdout)


def template(rationale):
    return rationale.split(" ", 1)[0] if rationale else "change"
