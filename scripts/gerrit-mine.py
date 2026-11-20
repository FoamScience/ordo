#!/usr/bin/env -S uv run --script
# /// script
# dependencies = []
# ///
"""Mine merged Gerrit changes and their inline review comments into SQLite,
the Gerrit counterpart of pr-mine.py. A comment names the patch set it was
left on, and every patch set's commit and fetch ref are stored, so a later
fix is the next patch set rewriting the commented lines; diffs come from
fetching those refs downstream.

Changes listed with fewer than --min-comments comments are recorded as
skipped without another call. A kept change adds its comments and its
revisions. Paced to --rate changes an hour (skipped ones count). Anonymous
reads; resumable, a change already in the DB is not fetched again.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/gerrit-mine.py [--projects FILE | --project HOST PROJECT ...]
       [--db FILE] [--per-project 60] [--rate 1000] [--min-comments 3]
"""
import argparse
import http.client
import json
import os
import re
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PAGE = 100
BOT = re.compile(r"\b(bot|robot|luci|presubmit|ci|service)\b|bot$", re.I)

SCHEMA = """
CREATE TABLE IF NOT EXISTS change (
  host TEXT, project TEXT, number INTEGER, kept INTEGER, change_id TEXT, branch TEXT,
  subject TEXT, owner TEXT, created TEXT, submitted TEXT, current_revision TEXT,
  total_comments INTEGER, human_comments INTEGER, fetched_at TEXT,
  PRIMARY KEY (host, number));
CREATE TABLE IF NOT EXISTS comment (
  host TEXT, number INTEGER, id TEXT, in_reply_to TEXT, author TEXT, bot INTEGER,
  path TEXT, side TEXT, patch_set INTEGER, commit_id TEXT, line INTEGER,
  start_line INTEGER, end_line INTEGER, unresolved INTEGER, message TEXT, updated TEXT,
  PRIMARY KEY (host, id));
CREATE TABLE IF NOT EXISTS revision (
  host TEXT, number INTEGER, patch_set INTEGER, sha TEXT, parents TEXT, ref TEXT,
  kind TEXT, uploader TEXT, created TEXT, message TEXT,
  PRIMARY KEY (host, number, patch_set));
CREATE TABLE IF NOT EXISTS revision_file (
  host TEXT, number INTEGER, patch_set INTEGER, path TEXT, old_path TEXT, status TEXT,
  lines_inserted INTEGER, lines_deleted INTEGER,
  PRIMARY KEY (host, number, patch_set, path));
CREATE TABLE IF NOT EXISTS scan (host TEXT, project TEXT, start INTEGER, done INTEGER,
  PRIMARY KEY (host, project));
"""


class Gerrit:
    """One keep-alive connection per host. Its short timeout lets a dead
    address (e.g. an unrouted IPv6 one) fall through to the next."""

    def __init__(self):
        self.calls, self.conns = 0, {}

    def get(self, host, path):
        for attempt in range(6):
            conn = self.conns.setdefault(host, http.client.HTTPSConnection(host, timeout=15))
            try:
                conn.request("GET", f"/{path}", headers={"Accept": "application/json"})
                r = conn.getresponse()
                body = r.read()
            except (OSError, http.client.HTTPException):
                conn.close()
                del self.conns[host]
                time.sleep(2 ** attempt)
                continue
            self.calls += 1
            if r.status == 200:
                return json.loads(body.decode()[4:])
            if r.status == 429 or r.status >= 500:
                time.sleep(int(r.getheader("Retry-After") or 0) or 2 ** attempt * 5)
            else:
                raise urllib.error.HTTPError(f"https://{host}/{path}", r.status, r.reason, r.headers, None)
        raise RuntimeError(f"giving up on {host}/{path}")


def name(account):
    return (account or {}).get("username") or (account or {}).get("name") or str((account or {}).get("_account_id"))


def is_bot(account):
    return "SERVICE_USER" in (account or {}).get("tags", []) or bool(BOT.search(name(account)))


def mine_change(g, db, host, c, min_comments):
    key = urllib.parse.quote(f"{c['project']}~{c['_number']}", safe="")
    comments, human = [], 0
    if c.get("total_comment_count", 0) >= min_comments:
        comments = [dict(cm, path=path) for path, cms in g.get(host, f"changes/{key}/comments").items()
                    for cm in cms if path not in ("/PATCHSET_LEVEL", "/COMMIT_MSG")]
        human = sum(not is_bot(cm.get("author")) for cm in comments)
    kept = human >= min_comments
    db.execute("INSERT OR REPLACE INTO change VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,datetime('now'))",
               (host, c["project"], c["_number"], kept, c["change_id"], c["branch"], c["subject"],
                name(c.get("owner")), c["created"], c.get("submitted"), c.get("current_revision"),
                c.get("total_comment_count"), human))
    if not kept:
        return False
    db.executemany("INSERT OR REPLACE INTO comment VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", [
        (host, c["_number"], cm["id"], cm.get("in_reply_to"), name(cm.get("author")),
         is_bot(cm.get("author")), cm["path"], cm.get("side", "REVISION"), cm.get("patch_set"),
         cm.get("commit_id"), cm.get("line"), (cm.get("range") or {}).get("start_line"),
         (cm.get("range") or {}).get("end_line"), cm.get("unresolved"), cm.get("message"),
         cm.get("updated")) for cm in comments])
    detail = g.get(host, f"changes/{key}?o=ALL_REVISIONS&o=ALL_COMMITS&o=ALL_FILES")
    for sha, r in detail["revisions"].items():
        db.execute("INSERT OR REPLACE INTO revision VALUES (?,?,?,?,?,?,?,?,?,?)",
                   (host, c["_number"], r["_number"], sha,
                    " ".join(p["commit"] for p in r["commit"]["parents"]), r["ref"], r.get("kind"),
                    name(r.get("uploader")), r["created"], r["commit"]["message"]))
        db.executemany("INSERT OR REPLACE INTO revision_file VALUES (?,?,?,?,?,?,?,?)", [
            (host, c["_number"], r["_number"], path, f.get("old_path"), f.get("status", "M"),
             f.get("lines_inserted", 0), f.get("lines_deleted", 0))
            for path, f in r.get("files", {}).items()])
    return True


def mine_project(g, db, host, project, per_project, min_comments, interval, clock):
    kept = db.execute("SELECT count(*) FROM change WHERE host=? AND project=? AND kept",
                      (host, project)).fetchone()[0]
    row = db.execute("SELECT start, done FROM scan WHERE host=? AND project=?", (host, project)).fetchone()
    start = row[0] if row else 0
    if (row and row[1]) or kept >= per_project:
        return
    seen = {r[0] for r in db.execute("SELECT number FROM change WHERE host=?", (host,))}
    q = urllib.parse.quote(f"project:{project} status:merged", safe=":")
    while kept < per_project:
        page = g.get(host, f"changes/?q={q}&n={PAGE}&S={start}&o=DETAILED_ACCOUNTS")
        for c in page:
            if kept >= per_project:
                break
            if c["_number"] in seen:
                continue
            clock[0] = max(clock[0] + interval, time.monotonic())
            time.sleep(max(0, clock[0] - time.monotonic()))
            kept += mine_change(g, db, host, c, min_comments)
            db.commit()
        start += len(page)
        more = bool(page) and page[-1].get("_more_changes", False)
        db.execute("INSERT OR REPLACE INTO scan VALUES (?,?,?,?)", (host, project, start, not more))
        db.commit()
        print(f"  {host} {project}: {start} listed, {kept}/{per_project} kept, {g.calls} calls", flush=True)
        if not more:
            break


def main():
    corpus = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus"))
    ap = argparse.ArgumentParser()
    ap.add_argument("--projects", type=Path, default=ROOT / "corpus/gerrit-projects.txt")
    ap.add_argument("--project", nargs=2, action="append", metavar=("HOST", "PROJECT"),
                    help="overrides --projects")
    ap.add_argument("--db", type=Path, default=corpus / "reviews-gerrit.sqlite")
    ap.add_argument("--per-project", type=int, default=60, help="kept changes per project")
    ap.add_argument("--rate", type=float, default=1000, help="changes examined per hour")
    ap.add_argument("--min-comments", type=int, default=3, help="human inline comments to keep a change")
    a = ap.parse_args()
    projects = a.project or [l.split() for l in a.projects.read_text().splitlines()
                             if l.strip() and not l.startswith("#")]
    a.db.parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(a.db)
    db.executescript(SCHEMA)
    g, clock = Gerrit(), [time.monotonic() - 3600 / a.rate]
    for host, project in projects:
        try:
            mine_project(g, db, host, project, a.per_project, a.min_comments, 3600 / a.rate, clock)
        except urllib.error.HTTPError as e:
            print(f"  {host} {project}: HTTP {e.code}, skipped", file=sys.stderr, flush=True)
    kept, comments = db.execute("SELECT (SELECT count(*) FROM change WHERE kept),"
                                " (SELECT count(*) FROM comment WHERE NOT bot)").fetchone()
    print(f"{a.db}: {kept} kept changes, {comments} human comments, {g.calls} calls this run")


if __name__ == "__main__":
    main()
