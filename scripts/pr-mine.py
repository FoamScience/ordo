#!/usr/bin/env -S uv run --script
# /// script
# dependencies = []
# ///
"""Mine merged GitHub PRs and their line-level review comments into SQLite,
as real-reviewer labels for hunch: a hunk a human commented on drew
attention; one rewritten by a later commit of the same PR after a comment was
a later fix. Only the raw API facts are stored; mapping them onto engine
hunks happens downstream from the stored SHAs.

Per PR: one call for its review comments; a PR with fewer than
--min-comments human comments is recorded as skipped and costs nothing more.
A kept PR adds its files and commits. Paced to --rate PRs an hour (skipped
ones count), and it sleeps to the reset when the API quota runs low.
Resumable: a PR already in the DB is not fetched again.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/pr-mine.py [--repos FILE | --repo OWNER/NAME ...]
       [--db FILE] [--per-repo 60] [--rate 1000] [--min-comments 3]
"""
import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
API = "https://api.github.com"
QUOTA_FLOOR = 50

SCHEMA = """
CREATE TABLE IF NOT EXISTS pr (
  repo TEXT, number INTEGER, kept INTEGER, title TEXT, body TEXT, author TEXT,
  base_ref TEXT, base_sha TEXT, head_sha TEXT, merge_sha TEXT,
  created_at TEXT, merged_at TEXT, human_comments INTEGER, fetched_at TEXT,
  PRIMARY KEY (repo, number));
CREATE TABLE IF NOT EXISTS comment (
  repo TEXT, number INTEGER, id INTEGER PRIMARY KEY, in_reply_to INTEGER,
  author TEXT, bot INTEGER, path TEXT, side TEXT, line INTEGER, start_line INTEGER,
  original_line INTEGER, original_start_line INTEGER, outdated INTEGER,
  commit_id TEXT, original_commit_id TEXT, diff_hunk TEXT, body TEXT, created_at TEXT);
CREATE TABLE IF NOT EXISTS file (
  repo TEXT, number INTEGER, path TEXT, previous_path TEXT, status TEXT,
  additions INTEGER, deletions INTEGER, patch TEXT,
  PRIMARY KEY (repo, number, path));
CREATE TABLE IF NOT EXISTS pr_commit (
  repo TEXT, number INTEGER, seq INTEGER, sha TEXT, parents TEXT,
  author_date TEXT, committer_date TEXT, message TEXT,
  PRIMARY KEY (repo, number, sha));
CREATE TABLE IF NOT EXISTS scan (repo TEXT PRIMARY KEY, page INTEGER, done INTEGER);
"""


class GitHub:
    def __init__(self):
        token = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True,
                               check=True).stdout.strip()
        self.headers = {"Authorization": f"Bearer {token}",
                        "Accept": "application/vnd.github+json",
                        "X-GitHub-Api-Version": "2022-11-28",
                        "User-Agent": "ordo-pr-mine"}
        self.calls = 0

    def get(self, url):
        """(json, next page url) for one request, waiting out the quota and
        retrying server errors."""
        for attempt in range(5):
            try:
                with urllib.request.urlopen(urllib.request.Request(url, headers=self.headers),
                                            timeout=60) as r:
                    self.calls += 1
                    self._guard(r.headers)
                    nxt = re.search(r'<([^>]+)>;\s*rel="next"', r.headers.get("Link", ""))
                    return json.load(r), nxt and nxt.group(1)
            except urllib.error.HTTPError as e:
                if e.code in (403, 429) and (e.headers.get("Retry-After") or
                                             e.headers.get("X-RateLimit-Remaining") == "0"):
                    self._sleep_until(e.headers)
                elif e.code >= 500:
                    time.sleep(2 ** attempt)
                else:
                    raise
            except urllib.error.URLError:
                time.sleep(2 ** attempt)
        raise RuntimeError(f"giving up on {url}")

    def all(self, url):
        out = []
        while url:
            page, url = self.get(url)
            out += page
        return out

    def _guard(self, headers):
        if int(headers.get("X-RateLimit-Remaining", QUOTA_FLOOR + 1)) <= QUOTA_FLOOR:
            self._sleep_until(headers)

    def _sleep_until(self, headers):
        wait = int(headers.get("Retry-After") or 0) or \
            max(0, int(headers.get("X-RateLimit-Reset", time.time() + 60)) - int(time.time())) + 5
        print(f"  quota low, sleeping {wait}s", flush=True)
        time.sleep(wait)


def is_bot(user):
    return user is None or user.get("type") == "Bot" or user["login"].endswith("[bot]")


def mine_pr(gh, db, repo, pr, min_comments):
    n = pr["number"]
    comments = gh.all(f"{API}/repos/{repo}/pulls/{n}/comments?per_page=100")
    human = sum(not is_bot(c["user"]) for c in comments)
    kept = human >= min_comments
    db.execute("INSERT OR REPLACE INTO pr VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,datetime('now'))",
               (repo, n, kept, pr["title"], pr["body"], (pr["user"] or {}).get("login"),
                pr["base"]["ref"], pr["base"]["sha"], pr["head"]["sha"], pr["merge_commit_sha"],
                pr["created_at"], pr["merged_at"], human))
    if not kept:
        return False
    db.executemany("INSERT OR REPLACE INTO comment VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", [
        (repo, n, c["id"], c.get("in_reply_to_id"), (c["user"] or {}).get("login"), is_bot(c["user"]),
         c["path"], c.get("side"), c.get("line"), c.get("start_line"), c.get("original_line"),
         c.get("original_start_line"), c.get("line") is None, c["commit_id"],
         c["original_commit_id"], c["diff_hunk"], c["body"], c["created_at"])
        for c in comments])
    files = gh.all(f"{API}/repos/{repo}/pulls/{n}/files?per_page=100")
    db.executemany("INSERT OR REPLACE INTO file VALUES (?,?,?,?,?,?,?,?)", [
        (repo, n, f["filename"], f.get("previous_filename"), f["status"], f["additions"],
         f["deletions"], f.get("patch")) for f in files])
    commits = gh.all(f"{API}/repos/{repo}/pulls/{n}/commits?per_page=100")
    db.executemany("INSERT OR REPLACE INTO pr_commit VALUES (?,?,?,?,?,?,?,?)", [
        (repo, n, i, c["sha"], " ".join(p["sha"] for p in c["parents"]),
         c["commit"]["author"]["date"], c["commit"]["committer"]["date"], c["commit"]["message"])
        for i, c in enumerate(commits)])
    return True


def mine_repo(gh, db, repo, per_repo, min_comments, interval, clock):
    kept = db.execute("SELECT count(*) FROM pr WHERE repo=? AND kept", (repo,)).fetchone()[0]
    row = db.execute("SELECT page, done FROM scan WHERE repo=?", (repo,)).fetchone()
    page = row[0] if row else 1
    if (row and row[1]) or kept >= per_repo:
        return
    seen = {r[0] for r in db.execute("SELECT number FROM pr WHERE repo=?", (repo,))}
    while kept < per_repo:
        pulls, nxt = gh.get(f"{API}/repos/{repo}/pulls?state=closed&sort=created"
                            f"&direction=desc&per_page=100&page={page}")
        for pr in pulls:
            if kept >= per_repo:
                break
            if not pr["merged_at"] or pr["number"] in seen:
                continue
            clock[0] = max(clock[0] + interval, time.monotonic())
            time.sleep(max(0, clock[0] - time.monotonic()))
            kept += mine_pr(gh, db, repo, pr, min_comments)
            db.commit()
        print(f"  {repo}: page {page}, {kept}/{per_repo} kept, {gh.calls} calls", flush=True)
        page += 1
        db.execute("INSERT OR REPLACE INTO scan VALUES (?,?,?)", (repo, page, nxt is None))
        db.commit()
        if nxt is None:
            break


def main():
    corpus = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus"))
    ap = argparse.ArgumentParser()
    ap.add_argument("--repos", type=Path, default=ROOT / "corpus/pr-repos.txt")
    ap.add_argument("--repo", action="append", help="owner/name, overrides --repos")
    ap.add_argument("--db", type=Path, default=corpus / "reviews.sqlite")
    ap.add_argument("--per-repo", type=int, default=60, help="kept PRs per repo")
    ap.add_argument("--rate", type=float, default=1000, help="PRs examined per hour")
    ap.add_argument("--min-comments", type=int, default=3, help="human review comments to keep a PR")
    a = ap.parse_args()
    repos = a.repo or [l.strip() for l in a.repos.read_text().splitlines()
                       if l.strip() and not l.startswith("#")]
    a.db.parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(a.db)
    db.executescript(SCHEMA)
    gh, clock = GitHub(), [time.monotonic() - 3600 / a.rate]
    for repo in repos:
        try:
            mine_repo(gh, db, repo, a.per_repo, a.min_comments, 3600 / a.rate, clock)
        except urllib.error.HTTPError as e:
            print(f"  {repo}: HTTP {e.code}, skipped", file=sys.stderr, flush=True)
    kept, comments = db.execute("SELECT (SELECT count(*) FROM pr WHERE kept),"
                                " (SELECT count(*) FROM comment WHERE NOT bot)").fetchone()
    print(f"{a.db}: {kept} kept PRs, {comments} human comments, {gh.calls} calls this run")


if __name__ == "__main__":
    main()
