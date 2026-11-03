#!/usr/bin/env python3
"""Robustness census: run ordo-engine over the newest commits of every repo in
corpus/census.toml, each run under a time and memory cap, and record how each
one ended.

Each repo is fetched shallow at its pinned rev into $ORDO_CENSUS (default
~/.cache/ordo-census) and removed once swept unless --keep. One row per commit
goes to corpus/census.csv; corpus/census.md summarises the run: completion
rate, every commit that did not complete, runtime percentiles, and the share of
files the engine flagged degraded or unsupported.

A commit ends `ok`, `panic` (a Rust panic), `oom` (past the memory cap),
`timeout`, or `error` (anything else, stderr's last line recorded).

A run narrowed with --only prints its summary instead of replacing the
recorded census.

Usage: scripts/census.py [--commits 30] [--timeout 60] [--mem-gb 4] [--workers 8] [--only name,...] [--keep]
Needs target/release/ordo-engine (cargo build --release --bin ordo-engine).
"""
import argparse
import csv
import json
import os
import platform
import shutil
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import date
from pathlib import Path

from corpuslib import ENGINE, ROOT, commit_input, git

LIST = ROOT / "corpus/census.toml"
CSV = ROOT / "corpus/census.csv"
SUMMARY = ROOT / "corpus/census.md"
FIELDS = ["repo", "lang", "sha", "status", "ms", "files", "hunks", "degraded", "unsupported", "detail"]


def repos(path):
    field = lambda b, k: next((l.split('"')[1] for l in b.splitlines() if l.startswith(f"{k} =")), None)
    for b in path.read_text().split("[[repo]]")[1:]:
        yield field(b, "name"), field(b, "url"), field(b, "lang"), field(b, "rev")


def fetch(url, rev, dest, depth):
    if (dest / ".git").is_dir():
        return True
    dest.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "init", "-q", str(dest)], check=True)
    r = subprocess.run(["git", "-C", str(dest), "fetch", "-q", "--depth", str(depth), url, rev],
                       capture_output=True, text=True)
    return r.returncode == 0


def run_engine(inp, timeout, mem):
    # prlimit rather than preexec_fn: these runs start from worker threads,
    # where a preexec_fn child can deadlock before it execs
    t0 = time.monotonic()
    try:
        r = subprocess.run(["prlimit", f"--as={mem}", ENGINE, "order", "--json"], input=json.dumps(inp),
                           capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return "timeout", timeout * 1000, None, ""
    ms = (time.monotonic() - t0) * 1000
    err = r.stderr.strip()
    last = err.splitlines()[-1] if err else ""
    if r.returncode == 0:
        try:
            return "ok", ms, json.loads(r.stdout), ""
        except json.JSONDecodeError:
            return "error", ms, None, "stdout is not json"
    if "memory allocation" in err or "out of memory" in err.lower():
        return "oom", ms, None, last
    if "panicked at" in err:
        at = next((l for l in err.splitlines() if "panicked at" in l), last)
        return "panic", ms, None, at
    return "error", ms, None, f"exit {r.returncode}: {last}"


def sweep_commit(name, lang, repo, sha, a):
    parent, inp = commit_input(repo, sha)
    if parent is None:
        return None
    status, ms, out, detail = run_engine(inp, a.timeout, a.mem_gb << 30)
    files = out["files"] if out else []
    return {
        "repo": name, "lang": lang, "sha": sha[:12], "status": status, "ms": round(ms, 1),
        "files": len(files), "hunks": sum(len(f["hunks"]) for f in files),
        "degraded": sum(bool(f.get("degraded")) for f in files),
        "unsupported": sum(bool(f.get("unsupported")) for f in files),
        "detail": detail,
    }


def pct(xs, p):
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else 0


def summarise(rows, a, fetched, unfetched):
    ok = [r for r in rows if r["status"] == "ok"]
    bad = [r for r in rows if r["status"] != "ok"]
    ms = sorted(r["ms"] for r in ok)
    files = sum(r["files"] for r in ok)
    cpu = next((l.split(":", 1)[1].strip() for l in Path("/proc/cpuinfo").read_text().splitlines()
                if l.startswith("model name")), platform.processor()) if Path("/proc/cpuinfo").exists() else platform.processor()
    by_lang = {}
    for r in rows:
        s = by_lang.setdefault(r["lang"], [0, 0])
        s[0] += 1
        s[1] += r["status"] == "ok"
    lines = [
        "# Census",
        "",
        f"`scripts/census.py`, {date.today().isoformat()}, on {cpu} ({os.cpu_count()} threads),",
        f"engine `{ENGINE.relative_to(ROOT)}` at `{git(ROOT, 'rev-parse', '--short', 'HEAD').strip()}` plus the working tree.",
        f"Newest {a.commits} commits of each repo in `corpus/census.toml`; each engine run capped at",
        f"{a.timeout} s and {a.mem_gb} GB of address space, {a.workers} at a time.",
        "",
        f"**{len(ok)} of {len(rows)} commits ({100 * len(ok) / max(1, len(rows)):.2f}%) completed**, over",
        f"{fetched} repositories" + (f" ({len(unfetched)} could not be fetched: {', '.join(unfetched)})." if unfetched else "."),
        "",
        "| | |",
        "|---|---|",
        f"| runtime per commit | p50 {pct(ms, .5):.0f} ms, p90 {pct(ms, .9):.0f} ms, p99 {pct(ms, .99):.0f} ms, max {ms[-1] if ms else 0:.0f} ms |",
        f"| files | {files}, {sum(r['degraded'] for r in ok)} degraded, {sum(r['unsupported'] for r in ok)} unsupported ({100 * sum(r['unsupported'] for r in ok) / max(1, files):.1f}%) |",
        f"| hunks | {sum(r['hunks'] for r in ok)} |",
        "",
        "| language | commits | completed |",
        "|---|---|---|",
        *[f"| {l} | {n} | {k} |" for l, (n, k) in sorted(by_lang.items())],
        "",
    ]
    if bad:
        lines += ["## Did not complete", "", "| repo | commit | status | ms | detail |", "|---|---|---|---|---|"]
        lines += [f"| {r['repo']} | {r['sha']} | {r['status']} | {r['ms']:.0f} | {r['detail'].replace('|', '/')} |" for r in bad]
        lines.append("")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--commits", type=int, default=30)
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--mem-gb", type=int, default=4)
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--only", default="")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--list", type=Path, default=LIST)
    a = ap.parse_args()
    if not ENGINE.exists():
        raise SystemExit(f"{ENGINE} missing: cargo build --release --bin ordo-engine")
    if not shutil.which("prlimit"):
        raise SystemExit("prlimit (util-linux) is needed to cap each run's memory")
    root = Path(os.environ.get("ORDO_CENSUS", "~/.cache/ordo-census")).expanduser()
    only = set(filter(None, a.only.split(",")))
    rows, fetched, unfetched = [], 0, []
    for name, url, lang, rev in repos(a.list):
        if only and name not in only:
            continue
        dest = root / name
        if not fetch(url, rev, dest, a.commits + 1):
            unfetched.append(name)
            shutil.rmtree(dest, ignore_errors=True)
            continue
        fetched += 1
        shas = git(dest, "rev-list", "--max-count", str(a.commits), rev).split()
        with ThreadPoolExecutor(a.workers) as pool:
            done = [r for r in pool.map(lambda s: sweep_commit(name, lang, dest, s, a), shas) if r]
        rows += done
        failed = sum(r["status"] != "ok" for r in done)
        print(f"  {name:40} {len(done):3} commits, {failed} did not complete", flush=True)
        if not a.keep:
            shutil.rmtree(dest, ignore_errors=True)
    summary = summarise(rows, a, fetched, unfetched)
    if only:
        print(summary)
        return
    with CSV.open("w", newline="") as f:
        w = csv.DictWriter(f, FIELDS)
        w.writeheader()
        w.writerows(rows)
    SUMMARY.write_text(summary)
    print(f"wrote {CSV} and {SUMMARY}")


if __name__ == "__main__":
    main()
