// ------------------------------------------------------------------------ web
//! `ordo web`: the reviewer in a browser, served from this binary.
//!
//! The server owns every review it has been asked for — one `Session` and its
//! `App` per revision — and runs the same load, watch and command code the
//! terminal reviewer runs. The browser renders `view::review` and posts back
//! what the reader did. Nothing here decides what a hunk means.

mod view;

use crate::*;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::time::SystemTime;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const INDEX: &str = include_str!("../../../../web/dist/index.html");
const APP_JS: &str = include_str!("../../../../web/dist/app.js");
const APP_CSS: &str = include_str!("../../../../web/dist/app.css");

/// How often loads, the watcher and the marks file are looked at.
const TICK: Duration = Duration::from_millis(100);

/// A page asset: the bundle built into this binary, or, while working on the
/// client, the one `$ORDO_WEB_DIST` names, read on every request.
fn asset(name: &str, built: &str) -> String {
    std::env::var_os("ORDO_WEB_DIST")
        .and_then(|dir| std::fs::read_to_string(Path::new(&dir).join(name)).ok())
        .unwrap_or_else(|| built.to_string())
}

/// What `ordo web` was asked for on top of a review's own settings.
pub(super) struct WebArgs {
    pub(super) port: u16,
    pub(super) open: bool,
}

enum Phase {
    Loading(String),
    Ready(Box<App>),
    Empty(String),
}

/// One revision's review, as `run` keeps it for the terminal.
struct Slot {
    session: Session,
    phase: Phase,
    rx: Option<mpsc::Receiver<LoadMsg>>,
    /// bumped whenever what `view::review` returns would differ
    version: u64,
    /// bumped when a load lands: hunk indices from before it mean other hunks
    generation: u64,
    /// when a reader last did something here, for `--watch=auto`'s idle rule
    touched: Instant,
    marks_seen: Option<SystemTime>,
    stale_seen: Option<String>,
}

struct Hub {
    seed: Mutex<Session>,
    slots: Mutex<HashMap<String, Slot>>,
    /// open event streams, by the revision each one follows
    subs: Mutex<Vec<(String, mpsc::Sender<String>)>>,
    token: String,
    port: u16,
}

pub(super) fn serve(seed: Session, args: WebArgs) -> std::io::Result<()> {
    let server = Server::http(("127.0.0.1", args.port)).map_err(std::io::Error::other)?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(args.port);
    let rev = seed.rev.clone();
    let hub = Arc::new(Hub {
        seed: Mutex::new(seed),
        slots: Mutex::new(HashMap::new()),
        subs: Mutex::new(vec![]),
        token: session_token(),
        port,
    });
    hub.slot_for(&rev);
    let url = format!(
        "http://127.0.0.1:{port}/?rev={}#t={}",
        url_encode(&rev),
        hub.token
    );
    eprintln!("ordo: reviewing {rev} at {url}\nordo: Ctrl-C stops the server");
    if args.open {
        open_browser(&url);
    }
    let ticker = Arc::clone(&hub);
    thread::spawn(move || loop {
        ticker.tick();
        thread::sleep(TICK);
    });
    for req in server.incoming_requests() {
        let hub = Arc::clone(&hub);
        thread::spawn(move || hub.handle(req));
    }
    Ok(())
}

/// 128 bits nobody else on the machine can guess: the OS's random device, or
/// where there is none, std's per-process random hasher keys over the clock.
fn session_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::io::Read;
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok()
    {
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    let draw = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        h.finish()
    };
    format!("{:016x}{:016x}", draw(), draw())
}

fn open_browser(url: &str) {
    let cmd = std::env::var("BROWSER")
        .ok()
        .filter(|b| !b.trim().is_empty());
    let cmd = cmd.as_deref().unwrap_or(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    });
    let mut parts = cmd.split_whitespace();
    let Some(program) = parts.next() else { return };
    let _ = Command::new(program)
        .args(parts)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub(super) fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len()
                && b[i + 1].is_ascii_hexdigit()
                && b[i + 2].is_ascii_hexdigit() =>
            {
                let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                out.push(hex(b[i + 1]) << 4 | hex(b[i + 2]));
                i += 3;
                continue;
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query(url: &str) -> HashMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (!k.is_empty()).then(|| (url_decode(k), url_decode(v)))
        })
        .collect()
}

/// A panic while one request held the lock leaves the review as it was; the
/// server goes on rather than failing every request after it.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("an ascii header")
}

/// Equal without saying, by how long it took, how much of a guess was right.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn reply(req: Request, status: u16, ctype: &str, body: String) {
    let resp = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", ctype))
        .with_header(header("Cache-Control", "no-store"));
    let _ = req.respond(resp);
}

fn json(req: Request, body: serde_json::Value) {
    reply(req, 200, "application/json", body.to_string());
}

fn fail(req: Request, status: u16, msg: &str) {
    reply(
        req,
        status,
        "application/json",
        serde_json::json!({ "error": msg }).to_string(),
    );
}

impl Hub {
    /// The slot reviewing `rev`, its load started the first time it is asked
    /// for. `None` when `rev` names nothing reviewable.
    fn slot_for(&self, rev: &str) -> Option<()> {
        if lock(&self.slots).contains_key(rev) {
            return Some(());
        }
        // resolving runs git: not while every other request waits on `slots`
        let session = lock(&self.seed).for_rev(rev)?;
        let mut slots = lock(&self.slots);
        if slots.contains_key(rev) {
            return Some(());
        }
        let rx = session.spawn_load(session.target.clone());
        slots.insert(
            rev.to_string(),
            Slot {
                session,
                phase: Phase::Loading("starting…".to_string()),
                rx: Some(rx),
                version: 0,
                generation: 0,
                touched: Instant::now(),
                marks_seen: None,
                stale_seen: None,
            },
        );
        Some(())
    }

    fn broadcast(&self, rev: &str, event: &str, data: serde_json::Value) {
        let msg = format!("event: {event}\ndata: {data}\n\n");
        lock(&self.subs).retain(|(r, tx)| r != rev || tx.send(msg.clone()).is_ok());
    }

    /// Everything that moves without a request: loads finishing, the working
    /// tree drifting under `--watch`, another reviewer marking hunks.
    fn tick(&self) {
        let mut events: Vec<(String, &str, serde_json::Value)> = vec![];
        {
            let mut slots = lock(&self.slots);
            for (rev, slot) in slots.iter_mut() {
                drain(slot, rev, &mut events);
                watch(slot, rev, &mut events);
            }
        }
        for (rev, event, data) in events {
            self.broadcast(&rev, event, data);
        }
    }

    fn handle(&self, req: Request) {
        if !self.host_ok(&req) {
            return fail(req, 403, "unexpected Host header");
        }
        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("").to_string();
        match (req.method(), path.as_str()) {
            (Method::Get, "/") => reply(
                req,
                200,
                "text/html; charset=utf-8",
                asset("index.html", INDEX),
            ),
            (Method::Get, "/app.js") => reply(
                req,
                200,
                "text/javascript; charset=utf-8",
                asset("app.js", APP_JS),
            ),
            (Method::Get, "/app.css") => reply(
                req,
                200,
                "text/css; charset=utf-8",
                asset("app.css", APP_CSS),
            ),
            (_, p) if p.starts_with("/api/") => {
                let q = query(&url);
                // an EventSource cannot send a header; nothing else may use
                // the query, which lands in more places than a header does
                let token = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("X-Ordo-Token"))
                    .map(|h| h.value.as_str().to_string())
                    .or_else(|| {
                        (path == "/api/events")
                            .then(|| q.get("t").cloned())
                            .flatten()
                    });
                if !token.is_some_and(|t| same_secret(&t, &self.token)) {
                    return fail(req, 401, "missing or wrong session token");
                }
                self.api(req, &path, &q);
            }
            _ => fail(req, 404, "no such page"),
        }
    }

    /// A page from this server, asked for by the name it was served under
    /// (a browser leaves out port 80):
    /// anything else is another site resolving to localhost (DNS rebinding).
    fn host_ok(&self, req: &Request) -> bool {
        let host = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Host"))
            .map(|h| h.value.as_str().to_string());
        let port = self.port;
        host.is_some_and(|h| {
            let (name, at) = h.rsplit_once(':').unwrap_or((h.as_str(), "80"));
            matches!(name, "127.0.0.1" | "localhost") && at == port.to_string()
        })
    }

    fn api(&self, mut req: Request, path: &str, q: &HashMap<String, String>) {
        let rev = q.get("rev").cloned().unwrap_or_default();
        match path {
            "/api/themes" => return json(req, view::themes()),
            "/api/events" => return self.events(req, rev),
            _ => {}
        }
        if self.slot_for(&rev).is_none() {
            return fail(
                req,
                404,
                &format!("'{rev}' is not a git revision, a commit range or a GitButler CLI ID"),
            );
        }
        let mut body = serde_json::Value::Null;
        if *req.method() == Method::Post {
            let mut text = String::new();
            let read = req.as_reader().read_to_string(&mut text);
            match read.ok().and_then(|_| serde_json::from_str(&text).ok()) {
                Some(v) => body = v,
                None => return fail(req, 400, "the body is not JSON"),
            }
        }
        let num = |k: &str| q.get(k).and_then(|v| v.parse::<usize>().ok());
        match path {
            "/api/review" => {
                let out = view::slot(&lock(&self.slots)[&rev]);
                json(req, out)
            }
            "/api/file" => {
                let p = q.get("path").cloned().unwrap_or_default();
                let out = {
                    let mut slots = lock(&self.slots);
                    let slot = slots.get_mut(&rev).expect("slot");
                    slot.touched = Instant::now();
                    match &slot.phase {
                        Phase::Ready(app) => Some(view::file(app, &p)),
                        _ => None,
                    }
                };
                match out {
                    Some(v) => json(req, v),
                    None => fail(req, 409, "the review is still loading"),
                }
            }
            "/api/item" => {
                let Some(i) = num("i") else {
                    return fail(req, 400, "which item?");
                };
                self.item(req, &rev, i)
            }
            "/api/act" => self.act(req, &rev, &body),
            _ => fail(req, 404, "no such endpoint"),
        }
    }

    /// The facts about one hunk that cost more than a list row should: what
    /// rejecting it strands, and how often its lines changed before. The churn
    /// is `git log -L` (~0.2s on a large file), run with the lock released.
    fn item(&self, req: Request, rev: &str, i: usize) {
        let todo = {
            let mut slots = lock(&self.slots);
            let slot = slots.get_mut(rev).expect("slot");
            slot.touched = Instant::now();
            let Phase::Ready(app) = &mut slot.phase else {
                return fail(req, 409, "the review is still loading");
            };
            if i >= app.items.len() {
                return fail(req, 400, "no such item");
            }
            let it = &app.items[i];
            let key = (it.path.clone(), it.new_range[0], it.new_range[1]);
            (!app.churn_cache.contains_key(&key)).then(|| {
                (
                    key,
                    churn_query(it, app.uncommitted),
                    app.review_sha.clone(),
                )
            })
        };
        if let Some((key, query, sha)) = todo {
            let churn = sha.map(|sha| hunk_churn(&sha, &key.0, &query));
            let mut slots = lock(&self.slots);
            if let Phase::Ready(app) = &mut slots.get_mut(rev).expect("slot").phase {
                app.churn_cache.insert(key, churn);
            }
        }
        // a reload may have landed while the lock was down
        let out = match &lock(&self.slots)[rev].phase {
            Phase::Ready(app) if i < app.items.len() => Some(view::item(app, i)),
            _ => None,
        };
        match out {
            Some(v) => json(req, v),
            None => fail(req, 409, "the review was read again; ask once more"),
        }
    }

    fn act(&self, req: Request, rev: &str, body: &serde_json::Value) {
        let mut slots = lock(&self.slots);
        let slot = slots.get_mut(rev).expect("slot");
        slot.touched = Instant::now();
        let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
        let int = |k: &str| body.get(k).and_then(|v| v.as_u64()).map(|v| v as usize);
        if op == "reload" && matches!(slot.phase, Phase::Empty(_)) {
            // the terminal quits on an empty review; a server stays, so the
            // reader can ask again once there is something to read
            slot.rx = Some(slot.session.spawn_load(slot.session.target.clone()));
            slot.phase = Phase::Loading("starting…".to_string());
            slot.version += 1;
            let version = slot.version;
            drop(slots);
            self.broadcast(rev, "changed", serde_json::json!({ "version": version }));
            return json(req, serde_json::json!({ "version": version }));
        }
        // indices into a review that has since been read again name other hunks
        if int("generation").is_some_and(|g| g as u64 != slot.generation) {
            return fail(req, 409, "the review was read again; try once more");
        }
        let generation = slot.generation;
        let Phase::Ready(app) = &mut slot.phase else {
            return fail(req, 409, "the review is still loading");
        };
        if let Some(sel) = int("sel").filter(|&s| s < app.items.len()) {
            app.sel = sel;
            let lines = app
                .sources
                .get(&app.items[sel].path)
                .map_or(0, |(_, n)| n.len());
            let clamp = |l: usize| l.min(lines.saturating_sub(1));
            app.cursor = Cursor {
                line: int("line")
                    .map(clamp)
                    .unwrap_or_else(|| start_cursor(&app.items[sel]).line),
                col: 0,
            };
            app.selection = int("from").map(clamp);
        }
        let before = fingerprint(app);
        let mut extra = serde_json::Map::new();
        let result: Result<(), String> = match op {
            "mark" => {
                let i = app.sel;
                app.reviewed[i] = !app.reviewed[i];
                persist_mark(app, i)
            }
            "fold" => {
                let bucket = app.items[app.sel].bucket.clone();
                if !app.collapsed.remove(&bucket) {
                    app.collapsed.insert(bucket);
                }
                Ok(())
            }
            "reload" => {
                app.reload_requested = true;
                Ok(())
            }
            "action" => {
                let name = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
                // the moves whose rule lives on this side: the finding cycle,
                // folds and waves. Plain motion stays in the browser.
                const SERVED: &[&str] = &[
                    "finding-next",
                    "finding-prev",
                    "fold-toggle",
                    "fold-open",
                    "fold-close",
                    "fold-open-all",
                    "fold-close-all",
                    "wave-next",
                    "wave-prev",
                ];
                match crate::keys::action_by_name(name).filter(|_| SERVED.contains(&name)) {
                    Some(a) => {
                        app.focus = Pane::List;
                        app.popup = None;
                        apply_anywhere(app, a);
                        if let Some(n) = app.notice.take() {
                            extra.insert("notice".into(), n.into());
                        }
                        Ok(())
                    }
                    None => Err(format!("'{name}' is not an action the browser sends")),
                }
            }
            "hover" => {
                app.cursor.col = int("col").unwrap_or(0);
                hover(app);
                if let Some(p) = app.popup.take() {
                    extra.insert("popup".into(), view::popup(&p));
                }
                Ok(())
            }
            "config" => config(app, body),
            "open-editor" => open_in_editor(app, &mut extra),
            "command" => {
                let line = body.get("line_text").and_then(|v| v.as_str()).unwrap_or("");
                command(app, line, &mut extra)
            }
            _ => Err(format!("unknown action '{op}'")),
        };
        let sel = app.sel;
        let changed = fingerprint(app) != before;
        if changed {
            slot.version += 1;
        }
        let version = slot.version;
        let mut out =
            serde_json::json!({ "version": version, "generation": generation, "sel": sel });
        if let Err(e) = result {
            out["error"] = e.into();
        }
        for (k, v) in extra {
            out[k] = v;
        }
        drop(slots);
        if changed {
            self.broadcast(rev, "changed", serde_json::json!({ "version": version }));
        }
        json(req, out)
    }

    /// A server-sent event stream for one revision: load progress, `changed`
    /// when the review would read differently. Written straight to the socket
    /// and flushed per event — a chunked body would sit in its buffer — with a
    /// comment every so often, so a closed tab is noticed and dropped.
    fn events(&self, req: Request, rev: String) {
        let (tx, rx) = mpsc::channel::<String>();
        lock(&self.subs).push((rev, tx));
        let mut out = req.into_writer();
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                    Cache-Control: no-store\r\nConnection: keep-alive\r\n\r\n: ordo\n\n";
        if out
            .write_all(head.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            return;
        }
        loop {
            let msg = match rx.recv_timeout(Duration::from_secs(15)) {
                Ok(m) => m,
                Err(mpsc::RecvTimeoutError::Timeout) => ": keep-alive\n\n".to_string(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            if out
                .write_all(msg.as_bytes())
                .and_then(|()| out.flush())
                .is_err()
            {
                return;
            }
        }
    }
}

/// The review as every tab reads it; the selection is each tab's own.
fn fingerprint(app: &App) -> String {
    let mut v = view::review(app);
    v["sel"] = serde_json::Value::Null;
    v.to_string()
}

/// What the load worker has said since the last tick.
fn drain(slot: &mut Slot, rev: &str, events: &mut Vec<(String, &'static str, serde_json::Value)>) {
    let Some(rx) = slot.rx.as_ref() else { return };
    loop {
        match rx.try_recv() {
            Ok(LoadMsg::Progress(s)) => {
                events.push((
                    rev.to_string(),
                    "progress",
                    serde_json::json!({ "status": s }),
                ));
                slot.phase = Phase::Loading(s);
            }
            Ok(LoadMsg::Empty(msg)) => {
                slot.phase = Phase::Empty(msg);
                slot.rx = None;
                break;
            }
            Ok(LoadMsg::Done(r)) => {
                let app = slot.session.fresh_app(*r);
                slot.marks_seen = app.marks_path.as_deref().and_then(modified);
                slot.phase = Phase::Ready(Box::new(app));
                slot.generation += 1;
                slot.rx = None;
                break;
            }
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                slot.phase = Phase::Empty("ordo: loading failed unexpectedly".to_string());
                slot.rx = None;
                break;
            }
        }
    }
    slot.version += 1;
    events.push((
        rev.to_string(),
        "changed",
        serde_json::json!({ "version": slot.version }),
    ));
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// `--watch`, and marks written by another reviewer of this repository (a
/// second tab is the same `App`; the terminal reviewer is not).
fn watch(slot: &mut Slot, rev: &str, events: &mut Vec<(String, &'static str, serde_json::Value)>) {
    let Phase::Ready(app) = &mut slot.phase else {
        return;
    };
    if watch_tick(&mut slot.session, app, slot.touched.elapsed()) {
        let (target, rev_now) = (slot.session.target.clone(), slot.session.rev.clone());
        let target = match slot.session.waves_seen {
            Some(_) => resolve(&rev_now).unwrap_or(target),
            None => target,
        };
        slot.rx = Some(slot.session.reload(app, target, rev_now.clone()));
        slot.phase = Phase::Loading(format!("reading {rev_now} again…"));
        slot.version += 1;
        events.push((
            rev.to_string(),
            "progress",
            serde_json::json!({ "status": format!("reading {rev_now} again…") }),
        ));
        return;
    }
    let stale = app.stale.as_ref().map(Stale::line);
    if stale != slot.stale_seen {
        slot.stale_seen = stale.clone();
        slot.version += 1;
        events.push((
            rev.to_string(),
            "changed",
            serde_json::json!({ "version": slot.version }),
        ));
    }
    let seen = app.marks_path.as_deref().and_then(modified);
    if seen != slot.marks_seen {
        slot.marks_seen = seen;
        if sync_marks(app) {
            slot.version += 1;
            events.push((
                rev.to_string(),
                "changed",
                serde_json::json!({ "version": slot.version }),
            ));
        }
    }
}

/// `:config` from the browser: the same `ConfigUi` the terminal edits, driven
/// by row rather than by cursor.
fn config(app: &mut App, body: &serde_json::Value) -> Result<(), String> {
    let what = body.get("do").and_then(|v| v.as_str()).unwrap_or("");
    let row = body.get("row").and_then(|v| v.as_u64()).map(|v| v as usize);
    if what == "open" {
        app.config = Some(crate::config::ConfigUi::open(app));
        return Ok(());
    }
    let Some(c) = app.config.as_mut() else {
        return Err(":config is not open".into());
    };
    if let Some(r) = row.filter(|&r| r < c.rows.len()) {
        c.sel = r;
    }
    match what {
        "toggle" => config_toggle(app),
        "edit" => {
            c.editing = Some(
                body.get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            );
            config_commit_edit(app);
        }
        "write" => config_write(app),
        "close" => app.config = None,
        _ => return Err(format!("unknown config step '{what}'")),
    }
    Ok(())
}

/// `ge`: the hunk's file at its line, in the editor `$ORDO_WEB_EDITOR` names —
/// a URL the browser opens (`{root}`, `{path}` and `{line}` filled in; the
/// default is VS Code's), or `server`, to start `$VISUAL` from here the way
/// the terminal's `ge` does.
fn open_in_editor(
    app: &App,
    extra: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if std::env::var("ORDO_WEB_EDITOR").as_deref() != Ok("server") {
        return Err("the browser opens this one".into());
    }
    let (path, _, approx) = crate::editor::edit_target(app);
    let line = app.cursor.line + 1;
    let (program, args) =
        crate::editor::build_command(&crate::editor::resolve_editor(), &path, line)
            .ok_or("$VISUAL is empty")?;
    let mut child = Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    thread::spawn(move || child.wait());
    if approx {
        extra.insert(
            "notice".into(),
            format!("{path} has changed since this revision; line {line} is approximate").into(),
        );
    }
    Ok(())
}

/// A command from the palette, as the terminal's `:` bar runs it. The few
/// that act on the terminal itself answer the browser instead.
fn command(
    app: &mut App,
    line: &str,
    extra: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let line = line.trim().trim_start_matches(':');
    let (name, arg) = crate::commands::parse_command_line(line);
    match name {
        "q" => return Ok(()),
        "quickfix" | "qf" | "vim-qfl" => {
            return Err(":quickfix is the terminal reviewer's; the list is already here".into())
        }
        "e" => {
            let rev = arg.trim();
            if rev.is_empty() {
                return Err("usage: :e <rev>".into());
            }
            resolve(rev).ok_or_else(|| format!("'{rev}' is not a revision"))?;
            extra.insert("navigate".into(), rev.into());
            return Ok(());
        }
        "yank" => {
            let all = arg.trim() == "all";
            let Some(prompt) = crate::handoff::review_prompt(app, all) else {
                return Err(
                    "nothing to hand back — :note a symbol, :comment a line, or `all` for ordo's own"
                        .into(),
                );
            };
            extra.insert("clipboard".into(), prompt.into());
            return Ok(());
        }
        _ => {}
    }
    match crate::commands::execute_command(app, line)? {
        CommandOutcome::Reload(_, rev) => {
            extra.insert("navigate".into(), rev.into());
        }
        CommandOutcome::Quit | CommandOutcome::None | CommandOutcome::OpenQuickfix(_) => {}
    }
    if let Some(p) = app.popup.take() {
        extra.insert("popup".into(), view::popup(&p));
    }
    if let Some(n) = app.notice.take() {
        extra.insert("notice".into(), n.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_decodes_without_trusting_what_follows_a_percent() {
        assert_eq!(url_decode("a%20b+c"), "a b c");
        assert_eq!(url_decode("%a\u{e9}"), "%a\u{e9}");
        assert_eq!(url_decode("%+f"), "% f");
        assert_eq!(url_decode("100%"), "100%");
    }

    #[test]
    fn a_secret_matches_only_itself() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abd", "abc"));
        assert!(!same_secret("ab", "abc"));
    }
}
