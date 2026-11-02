//! `ordo web` end to end: a real binary serving a real repository, spoken to
//! over a socket the way a browser would. Every cache and config path points
//! into a scratch directory, so nothing the user has is read or written.
//!
//! The view model is snapshotted in `tests/web/review.json`; regenerate after
//! an intentional change with `UPDATE_GOLDEN=1 cargo test --test web`.
#![cfg(feature = "web")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

mod common;
use common::update_requested;

struct Server {
    child: Child,
    port: u16,
    token: String,
    home: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

/// A repository whose last commit adds a helper and its caller, then
/// `ordo web HEAD` started inside it.
fn serve(name: &str) -> Server {
    let home = std::env::temp_dir().join(format!("ordo-web-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let repo = home.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("app.py"), "def main():\n    return 1\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "start"]);
    std::fs::write(repo.join("util.py"), "def helper(x):\n    return x * 2\n").unwrap();
    std::fs::write(
        repo.join("app.py"),
        "from util import helper\n\n\ndef main():\n    return helper(1)\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "use a helper"]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_ordo"))
        .args(["web", "HEAD", "--no-open", "--no-catalog"])
        .current_dir(&repo)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("ORDO_WEB_DIST")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("ordo web");
    let err = child.stderr.take().unwrap();
    let mut line = String::new();
    BufReader::new(err).read_line(&mut line).unwrap();
    let url = line
        .split_whitespace()
        .find(|w| w.starts_with("http://"))
        .unwrap_or_else(|| panic!("no URL in {line:?}"))
        .to_string();
    let port = url["http://127.0.0.1:".len()..]
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let token = url.rsplit_once("#t=").unwrap().1.to_string();
    Server {
        child,
        port,
        token,
        home,
    }
}

struct Reply {
    status: u16,
    body: String,
}

impl Server {
    fn raw(&self, method: &str, path: &str, host: &str, token: Option<&str>, body: &str) -> Reply {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        let auth = token
            .map(|t| format!("X-Ordo-Token: {t}\r\n"))
            .unwrap_or_default();
        write!(
            s,
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let status = out[9..12].parse().unwrap();
        let body = out.split_once("\r\n\r\n").map_or("", |(_, b)| b);
        Reply {
            status,
            body: dechunk(body),
        }
    }

    fn get(&self, path: &str) -> serde_json::Value {
        let host = format!("127.0.0.1:{}", self.port);
        let r = self.raw("GET", path, &host, Some(&self.token), "");
        assert_eq!(r.status, 200, "{path}: {}", r.body);
        serde_json::from_str(&r.body).unwrap()
    }

    fn post(&self, path: &str, body: serde_json::Value) -> serde_json::Value {
        let host = format!("127.0.0.1:{}", self.port);
        let r = self.raw("POST", path, &host, Some(&self.token), &body.to_string());
        assert_eq!(r.status, 200, "{path}: {}", r.body);
        serde_json::from_str(&r.body).unwrap()
    }

    /// An open event stream, past its response head.
    fn stream(&self) -> BufReader<TcpStream> {
        let mut es = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(
            es,
            "GET /api/events?rev=HEAD&t={} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
            self.token, self.port
        )
        .unwrap();
        es.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut events = BufReader::new(es);
        let mut head = String::new();
        while events.read_line(&mut head).unwrap() > 0 && !head.ends_with("\r\n\r\n") {}
        assert!(head.contains("text/event-stream"), "{head}");
        events
    }

    fn ready(&self) -> serde_json::Value {
        let t = Instant::now();
        loop {
            let v = self.get("/api/review?rev=HEAD");
            if v["state"] == "ready" {
                return v;
            }
            assert!(v["state"] == "loading", "{v}");
            assert!(t.elapsed() < Duration::from_secs(30), "never loaded");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// tiny_http sends a body of unknown length chunked
fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some((size, tail)) = rest.split_once("\r\n") else {
            return body.to_string();
        };
        let Ok(n) = usize::from_str_radix(size.trim(), 16) else {
            return body.to_string();
        };
        if n == 0 {
            return out;
        }
        out.push_str(&tail[..n]);
        rest = &tail[n + 2..];
    }
}

#[test]
fn the_api_wants_the_session_token_and_this_server_as_host() {
    let s = serve("auth");
    let host = format!("127.0.0.1:{}", s.port);
    assert_eq!(
        s.raw("GET", "/api/review?rev=HEAD", &host, None, "").status,
        401
    );
    assert_eq!(
        s.raw("GET", "/api/review?rev=HEAD", &host, Some("wrong"), "")
            .status,
        401
    );
    let rebound = s.raw(
        "GET",
        "/api/review?rev=HEAD",
        "evil.example:80",
        Some(&s.token),
        "",
    );
    assert_eq!(
        rebound.status, 403,
        "a page on another name reached the API"
    );
    // the page itself carries no review, so it is served without a token
    let page = s.raw("GET", "/", &host, None, "");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("app.js"));
    // only the event stream, which cannot send a header, takes it in the query
    let in_query = format!("/api/review?rev=HEAD&t={}", s.token);
    assert_eq!(s.raw("GET", &in_query, &host, None, "").status, 401);
}

#[test]
fn the_review_reads_as_the_terminal_reads_it() {
    let s = serve("review");
    let v = s.ready();
    // the stable part of the view model: what each hunk is, in reading order
    let snapshot: Vec<serde_json::Value> = v["view"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            let it = &v["items"][i.as_u64().unwrap() as usize];
            serde_json::json!({
                "path": it["path"], "new": it["new"], "what": it["what"],
                "grouped_by": it["grouped_by"],
                "edges": it["edges"], "noise": it["noise"],
            })
        })
        .collect();
    let got = serde_json::to_string_pretty(&snapshot).unwrap();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/web/review.json");
    if update_requested("UPDATE_GOLDEN") {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, format!("{got}\n")).unwrap();
    } else {
        let want =
            std::fs::read_to_string(&golden).expect("tests/web/review.json; UPDATE_GOLDEN=1");
        assert_eq!(got.trim(), want.trim());
    }
    assert_eq!(v["subject"], "use a helper");
    assert_eq!(
        v["rules"]["catalog"], false,
        "--no-catalog reached the review"
    );
    let path = v["items"][0]["path"].as_str().unwrap().to_string();
    let f = s.get(&format!("/api/file?rev=HEAD&path={path}"));
    assert!(
        f["new_spans"].as_array().is_some_and(|l| !l.is_empty()),
        "highlighted by role"
    );
}

#[test]
fn a_mark_persists_and_is_announced_to_every_open_stream() {
    let s = serve("mark");
    let before = s.ready()["version"].as_u64().unwrap();
    let mut streams: Vec<BufReader<TcpStream>> = (0..2).map(|_| s.stream()).collect();

    let out = s.post(
        "/api/act?rev=HEAD",
        serde_json::json!({ "op": "mark", "sel": 0 }),
    );
    assert_eq!(out["sel"], 0);
    let version = out["version"].as_u64().unwrap();
    assert!(version > before);
    for events in &mut streams {
        let want = format!("data: {{\"version\":{version}}}");
        let mut line = String::new();
        let t = Instant::now();
        while !line.contains(&want) {
            line.clear();
            events.read_line(&mut line).unwrap();
            assert!(t.elapsed() < Duration::from_secs(10), "no changed event");
        }
    }
    let v = s.get("/api/review?rev=HEAD");
    assert_eq!(v["items"][0]["reviewed"], true);
    assert_eq!(v["coverage"]["done"], 1);
    let dir = s.home.join("cache/ordo/reviewed");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .expect("the marks file was written under XDG_CACHE_HOME")
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    let marks: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&files[0]).unwrap()).unwrap();
    assert_eq!(marks.len(), 1, "one hunk marked, one mark written");
}

#[test]
fn an_act_against_a_review_since_read_again_is_refused() {
    let s = serve("generation");
    let v = s.ready();
    let generation = v["generation"].as_u64().unwrap();
    let host = format!("127.0.0.1:{}", s.port);
    let stale = serde_json::json!({ "op": "mark", "sel": 0, "generation": generation + 1 });
    let r = s.raw(
        "POST",
        "/api/act?rev=HEAD",
        &host,
        Some(&s.token),
        &stale.to_string(),
    );
    assert_eq!(r.status, 409, "{}", r.body);
    assert_eq!(s.get("/api/review?rev=HEAD")["items"][0]["reviewed"], false);
    let bad = s.raw("POST", "/api/act?rev=HEAD", &host, Some(&s.token), "{");
    assert_eq!(bad.status, 400);
}

#[test]
fn a_hover_leaves_the_review_as_it_was() {
    let s = serve("hover");
    let version = s.ready()["version"].clone();
    let out = s.post(
        "/api/act?rev=HEAD",
        serde_json::json!({ "op": "hover", "sel": 0, "col": 4 }),
    );
    assert_eq!(out["version"], version, "a hover made every tab read again");
}

#[test]
fn a_command_answers_with_what_the_terminal_would_have_shown() {
    let s = serve("command");
    s.ready();
    let out = s.post(
        "/api/act?rev=HEAD",
        serde_json::json!({ "op": "command", "sel": 0, "line_text": "rules" }),
    );
    assert!(out["popup"]["title"].is_string(), "{out}");
    let out = s.post(
        "/api/act?rev=HEAD",
        serde_json::json!({ "op": "command", "sel": 0, "line_text": "e HEAD~1" }),
    );
    assert_eq!(
        out["navigate"], "HEAD~1",
        "a revision switch is the browser's to make"
    );
    let out = s.post(
        "/api/act?rev=HEAD",
        serde_json::json!({ "op": "command", "sel": 0, "line_text": "mode hunks" }),
    );
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(s.get("/api/review?rev=HEAD")["mode"], "hunks");
}
