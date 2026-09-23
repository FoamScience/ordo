#!/usr/bin/env python3
"""Review a change. ordo's review pack and the diff go to a generative model,
which writes comments in reading order; the judge (a jeff server) keeps the ones
it rates correct and actionable. Nothing is posted: kept comments are printed.

usage: scripts/review-bot.py REV [--judge URL] [--gate P] [--generator CMD] [--comments FILE] [--show-dropped]
       scripts/review-bot.py --self-test

REV is a commit, A..B, A...B (from the merge base), or a branch name (its
commits since main). The generator is any command that reads a prompt on stdin
and answers on stdout; the default is `claude -p`. --comments FILE saves the
generator's answer there, or reuses it when the file exists, so a judge run can
be repeated without paying for generation again. Start the judge first:

    cd ~/repo/jeff && JEFF_BACKEND=llm JEFF_MODEL=Qwen/Qwen2.5-Coder-3B-Instruct \
        JEFF_LLM_QUANT=4bit JEFF_LLM_ADAPTER=models/judge-lora-coder3b-v1 \
        JEFF_MODEL_NAME=judge-coder3b-lora JEFF_HOST=127.0.0.1 JEFF_PORT=8017 uv run jeff

Progress goes to stderr, one line per step; the review goes to stdout.
"""
import argparse
import json
import re
import shlex
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from contextlib import contextmanager
from pathlib import Path

from corpuslib import ENGINE, ROOT, git, range_input
from judgelib import JEFF_URL

# the diff a generator is shown, and the slice of it a judge sees per comment
MAX_DIFF_CHARS = 60_000
MAX_HUNK_LINES = 80
QUESTION = "Is this review comment correct and actionable for the code shown?"

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


def resolve(repo: Path, rev: str) -> tuple[str, str]:
    """(base, head) for a revision spec."""
    def sha(r):
        out = git(repo, "rev-parse", "--verify", "-q", f"{r}^{{commit}}").strip()
        if not out:
            sys.exit(f"review-bot: unknown revision {r!r}")
        return out

    if "..." in rev:
        a, b = rev.split("...", 1)
        return git(repo, "merge-base", sha(a), sha(b)).strip(), sha(b)
    if ".." in rev:
        a, b = rev.split("..", 1)
        return sha(a), sha(b)
    if git(repo, "rev-parse", "--verify", "-q", f"refs/heads/{rev}").strip():
        return git(repo, "merge-base", sha("main"), sha(rev)).strip(), sha(rev)
    return sha(f"{rev}^"), sha(rev)


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


def hunk_for(file_diff: str, line: int) -> str | None:
    """The hunk of one file's diff whose new side holds `line`, cut to
    MAX_HUNK_LINES; None when no hunk does."""
    for h in re.split(r"(?m)^(?=@@ )", file_diff):
        m = re.match(r"@@ -\S+ \+(\d+)(?:,(\d+))? @@", h)
        if m and int(m.group(1)) <= line < int(m.group(1)) + int(m.group(2) or 1):
            return "\n".join(h.splitlines()[:MAX_HUNK_LINES])
    return None


def served_model(url: str) -> str:
    """The name the jeff server answers to."""
    with urllib.request.urlopen(f"{url}/v1/models", timeout=10) as r:
        return json.load(r)["models"][0]["name"]


def judge(url: str, model: str, state: str) -> float:
    body = json.dumps({"state": state, "model": model,
                       "questions": {"keep": {"type": "noul", "instructions": QUESTION}}}).encode()
    req = urllib.request.Request(f"{url}/v1/systemone", body, {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.load(r)["answers"]["keep"]["noul"]


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


def self_test():
    assert parse_comments('noise\n{"path": "a.rs", "line": "3", "comment": "x"}\n{"path": ""}\n{bad\n'
                          '{"path": "b.rs", "line": "ten", "comment": "y"}') == \
        [{"path": "a.rs", "line": 3, "comment": "x"}]
    d = "diff --git a/f b/f\n@@ -1,2 +1,2 @@\n-a\n+b\n@@ -10,3 +10,4 @@\n c\n+d\n"
    assert hunk_for(d, 11).startswith("@@ -10,3 +10,4 @@")
    assert hunk_for(d, 99) is None
    print("ok")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("rev", nargs="?")
    ap.add_argument("--judge", default=JEFF_URL, help="jeff base URL")
    ap.add_argument("--gate", type=float, default=0.5, help="keep a comment at or above this probability")
    ap.add_argument("--generator", default="claude -p", help="command: prompt on stdin, comments on stdout")
    ap.add_argument("--comments", type=Path, help="save the generator's answer here, or reuse it")
    ap.add_argument("--show-dropped", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.rev:
        ap.error("REV is required")

    cached = a.comments is not None and a.comments.exists()
    pr = Progress(f"Reviewing {a.rev}", 5)
    with pr.step(f"resolve {a.rev}"):
        base, head = resolve(ROOT, a.rev)
        inp = range_input(ROOT, base, head)
        if not inp:
            sys.exit("review-bot: no textual change in that range")
        pr.update(f"resolve {a.rev} -> {base[:8]}..{head[:8]}, {len(inp['changes'])} files")
    with pr.step("ordo-engine pack"):
        pack = subprocess.run([ENGINE, "pack", "--json"], input=json.dumps(inp),
                              capture_output=True, text=True, check=True).stdout
        diff = git(ROOT, "diff", base, head)
        if len(diff) > MAX_DIFF_CHARS:
            diff = diff[:MAX_DIFF_CHARS] + "\n[… diff truncated]"
    # the judge is checked before the generator runs, so a missing server
    # costs nothing
    with pr.step(f"judge at {a.judge}"):
        try:
            model = served_model(a.judge)
        except urllib.error.URLError as e:
            sys.exit(f"review-bot: judge at {a.judge} unreachable ({e.reason}); start jeff first")
        pr.update(f"judge at {a.judge}: {model}")
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
    file_diffs = {}
    kept, dropped, stray = [], [], []
    with pr.step(f"judge {len(comments)} comments"):
        for n, c in enumerate(comments, 1):
            pr.update(f"judge {n}/{len(comments)} comments, {len(kept)} kept")
            if c["path"] not in file_diffs:
                file_diffs[c["path"]] = git(ROOT, "diff", base, head, "--", c["path"])
            code = hunk_for(file_diffs[c["path"]], c["line"])
            # a comment off every hunk has no code to be judged against
            if code is None:
                stray.append(c)
                continue
            state = f"file: {c['path']}:{c['line']}\nreview comment: {c['comment']}\n\ncode:\n{code}"
            c["p"] = judge(a.judge, model, state)
            (kept if c["p"] >= a.gate else dropped).append(c)
        pr.update(f"judge {len(comments)} comments, {len(kept)} kept (gate {a.gate}), {len(stray)} off the diff")
    pr.done()

    print(f"# {a.rev}: {len(kept)} of {len(comments)} comments kept (gate {a.gate})\n")
    for c in kept:
        print(f"{c['path']}:{c['line']}  p={c['p']:.2f}\n  {c['comment']}\n")
    if a.show_dropped and dropped:
        print(f"# dropped ({len(dropped)})\n")
        for c in dropped:
            print(f"{c['path']}:{c['line']}  p={c['p']:.2f}\n  {c['comment']}\n")
    if a.show_dropped and stray:
        print(f"# off the diff, not judged ({len(stray)})\n")
        for c in stray:
            print(f"{c['path']}:{c['line']}\n  {c['comment']}\n")


if __name__ == "__main__":
    main()
