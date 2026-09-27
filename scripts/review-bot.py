#!/usr/bin/env python3
"""EXPERIMENTAL. Review a change: ordo's review pack and the diff go to a
generative model, which writes comments in reading order. Nothing is posted:
comments are printed, in color on a terminal (NO_COLOR turns it off), and with
--sarif also written as SARIF 2.1.0, so `ordo --sarif FILE REV` shows them
beside their hunks.

usage: scripts/review-bot.py REV [<glob>...] [--generator CMD] [--comments FILE] [--sarif FILE]
       scripts/review-bot.py --self-test

REV and the globs are whatever `ordo` takes (`ordo help`): a commit, a range,
zz for uncommitted work, main...zz, a GitButler ID, wave/N; `ordo --json`
gathers the change, so build ordo first (cargo build --release). The generator
is any command that reads a prompt on stdin and answers on stdout; the default
is `claude -p`, `opencode run` works too.
--comments FILE saves the generator's answer there, or reuses it when the file
exists, so the output can be redone without paying for generation again.

Progress goes to stderr, one line per step; the review goes to stdout.
"""
import argparse
import difflib
import json
import os
import re
import shlex
import subprocess
import sys
import threading
import time
from contextlib import contextmanager
from pathlib import Path

from corpuslib import ENGINE, ROOT

ORDO = ROOT / "target/release/ordo"

# the diff a generator is shown
MAX_DIFF_CHARS = 60_000

PROMPT = """You are reviewing a code change. Below is ordo's review pack — the
change in reading order, with what each hunk does and which parts are
independent — and then the diff itself.

Comment only where something is wrong, risky or unclear: a bug, a missed case,
a misleading name, a test that does not test what it claims. No praise, no
summaries, no style nits. Follow the reading order.

Answer with one JSON object per line and nothing else:
{{"path": "<file>", "line": <new-side line number>, "comment": "<one or two sentences>"}}

# pack
{pack}

# diff
{diff}
"""


def unified(changes: list[dict]) -> str:
    """One unified diff for the changes ordo gathered, new-side line numbers
    matching the files as they are."""
    out = []
    for c in changes:
        old, new = (c.get("old") or "").splitlines(True), (c.get("new") or "").splitlines(True)
        out.extend(difflib.unified_diff(old, new, f"a/{c['path']}", f"b/{c['path']}"))
    return "".join(l if l.endswith("\n") else l + "\n" for l in out)


def parse_comments(text: str) -> list[dict]:
    """The generator's JSON lines; anything that is not one is skipped."""
    out = []
    for line in text.splitlines():
        line = line.strip().strip("`")
        if not line.startswith("{"):
            continue
        try:
            c = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(c, dict) and c.get("path") and c.get("comment") \
                and str(c.get("line") or 0).isdigit():
            c["line"] = int(c.get("line") or 0)
            out.append(c)
    return out


class Progress:
    """Docker-style step lines on stderr. On a terminal the running step's
    timer ticks in place; piped, each step prints once, when it ends."""

    def __init__(self, title: str, total: int):
        self.total, self.i, self.label = total, 0, ""
        self.tty = sys.stderr.isatty()
        self.t0 = time.monotonic()
        print(f"[+] {title}", file=sys.stderr)

    def _write(self, elapsed: float, mark: str, end: str):
        line = f" {mark} [{self.i}/{self.total}] {self.label}"
        sys.stderr.write(f"\r\033[K{line:<72} {elapsed:6.1f}s{end}" if self.tty else f"{line} {elapsed:.1f}s{end}")
        sys.stderr.flush()

    def update(self, label: str):
        self.label = label

    @contextmanager
    def step(self, label: str):
        self.i, self.label = self.i + 1, label
        start = time.monotonic()
        stop = threading.Event()

        def tick():
            while not stop.wait(0.2):
                self._write(time.monotonic() - start, "=>", "")

        ticker = threading.Thread(target=tick, daemon=True)
        if self.tty:
            ticker.start()
        try:
            yield self
        except BaseException:
            stop.set()
            if self.tty:
                ticker.join()
            self._write(time.monotonic() - start, "✗ ", "\n")
            raise
        stop.set()
        if self.tty:
            ticker.join()
        self._write(time.monotonic() - start, "✔ ", "\n")

    def done(self):
        print(f"[+] done in {time.monotonic() - self.t0:.1f}s", file=sys.stderr)


COLOR = sys.stdout.isatty() and not os.environ.get("NO_COLOR")


def paint(sgr: str, text: str) -> str:
    return f"\033[{sgr}m{text}\033[0m" if COLOR else text


def show(c: dict) -> str:
    """One comment: its location, and the text with its `code` spans set apart."""
    head = paint("1;36", c["path"]) + paint("2", ":") + paint("33", str(c["line"]))
    body = re.sub(r"`([^`]+)`", lambda m: paint("35", m.group(1)) if COLOR else m.group(0), c["comment"])
    return f"{head}\n  {body}\n"


def sarif(comments: list[dict]) -> dict:
    """SARIF 2.1.0, in the shape ordo's own --sarif reader takes."""
    return {
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {"driver": {"name": "review-bot"}},
            "results": [{
                "kind": "review",
                "level": "warning",
                "message": {"text": c["comment"]},
                "locations": [{"physicalLocation": {
                    "artifactLocation": {"uri": c["path"]},
                    "region": {"startLine": max(c["line"], 1)},
                }}],
            } for c in comments],
        }],
    }


def self_test():
    assert parse_comments('noise\n{"path": "a.rs", "line": "3", "comment": "x"}\n{"path": ""}\n{bad\n'
                          '{"path": "b.rs", "line": "ten", "comment": "y"}') == \
        [{"path": "a.rs", "line": 3, "comment": "x"}]
    d = unified([{"path": "f", "old": "a\nb\n", "new": "a\nc"}, {"path": "g", "old": None, "new": "x\n"}])
    assert "--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n" in d and "+++ b/g\n@@ -0,0 +1 @@\n+x\n" in d
    global COLOR
    c = {"path": "a.rs", "line": 3, "comment": "`x` is unused"}
    COLOR = False
    assert show(c) == "a.rs:3\n  `x` is unused\n"
    COLOR = True
    assert "\033[35mx\033[0m is unused" in show(c)
    r = sarif([c, {**c, "line": 0}])["runs"][0]["results"]
    assert r[0]["locations"][0]["physicalLocation"]["region"]["startLine"] == 3
    assert r[0]["kind"] == "review"
    assert r[1]["locations"][0]["physicalLocation"]["region"]["startLine"] == 1
    print("ok")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("rev", nargs="?")
    ap.add_argument("globs", nargs="*", help="keep matching paths only, as ordo takes them")
    ap.add_argument("--generator", default="claude -p", help="command: prompt on stdin, comments on stdout")
    ap.add_argument("--comments", type=Path, help="save the generator's answer here, or reuse it")
    ap.add_argument("--sarif", type=Path, help="also write the comments here as SARIF 2.1.0")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.rev:
        ap.error("REV is required")

    cached = a.comments is not None and a.comments.exists()
    pr = Progress(f"Reviewing {a.rev} (experimental)", 3)
    with pr.step(f"resolve {a.rev}"):
        got = subprocess.run([ORDO, a.rev, *a.globs, "--json"], capture_output=True, text=True)
        if got.returncode:
            sys.exit(got.stderr.strip() or f"review-bot: ordo exited {got.returncode}")
        inp = json.loads(got.stdout)
        if not inp["changes"]:
            sys.exit("review-bot: no textual change there")
        pr.update(f"resolve {a.rev} -> {len(inp['changes'])} files")
    with pr.step("ordo-engine pack"):
        pack = subprocess.run([ENGINE, "pack", "--json"], input=json.dumps(inp),
                              capture_output=True, text=True, check=True).stdout
        diff = unified(inp["changes"])
        if len(diff) > MAX_DIFF_CHARS:
            diff = diff[:MAX_DIFF_CHARS] + "\n[… diff truncated]"
    with pr.step(f"generate: cached {a.comments}" if cached else f"generate: {a.generator}"):
        if cached:
            answer = a.comments.read_text()
        else:
            gen = subprocess.run(shlex.split(a.generator), input=PROMPT.format(pack=pack, diff=diff),
                                 capture_output=True, text=True)
            if gen.returncode:
                sys.exit(f"review-bot: generator failed: {gen.stderr.strip()[:500]}")
            answer = gen.stdout
            if a.comments:
                a.comments.write_text(answer)
        comments = parse_comments(answer)
        if not comments:
            sys.exit("review-bot: the generator wrote no comments in the expected format")
    pr.done()
    if a.sarif:
        a.sarif.write_text(json.dumps(sarif(comments), indent=2) + "\n")

    print(paint("1", f"# {a.rev}: {len(comments)} comments") + "\n")
    for c in comments:
        print(show(c))


if __name__ == "__main__":
    main()
