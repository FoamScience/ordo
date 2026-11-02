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

/// 128 bits nobody else on the machine can guess: `RandomState` is seeded
/// from the OS, and two of them are two independent draws.
fn session_token() -> String {
    use std::hash::{BuildHasher, Hasher};
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
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                Err(_) => out.push(b'%'),
            },
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

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("an ascii header")
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
        let mut slots = self.slots.lock().expect("slots");
        if slots.contains_key(rev) {
            return Some(());
        }
        let session = self.seed.lock().expect("seed").for_rev(rev)?;
        let rx = session.spawn_load(session.target.clone());
        slots.insert(
            rev.to_string(),
            Slot {
                session,
                phase: Phase::Loading("starting…".to_string()),
                rx: Some(rx),
                version: 0,
                touched: Instant::now(),
                marks_seen: None,
                stale_seen: None,
            },
        );
        Some(())
    }

    fn broadcast(&self, rev: &str, event: &str, data: serde_json::Value) {
        let msg = format!("event: {event}\ndata: {data}\n\n");
        self.subs
            .lock()
            .expect("subs")
            .retain(|(r, tx)| r != rev || tx.send(msg.clone()).is_ok());
    }

    /// Everything that moves without a request: loads finishing, the working
    /// tree drifting under `--watch`, another reviewer marking hunks.
    fn tick(&self) {
        let mut events: Vec<(String, &str, serde_json::Value)> = vec![];
        {
            let mut slots = self.slots.lock().expect("slots");
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
                let token = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("X-Ordo-Token"))
                    .map(|h| h.value.as_str().to_string())
                    .or_else(|| q.get("t").cloned());
                if token.as_deref() != Some(self.token.as_str()) {
                    return fail(req, 401, "missing or wrong session token");
                }
                self.api(req, &path, &q);
            }
            _ => fail(req, 404, "no such page"),
        }
    }

    /// A page from this server, asked for by the name it was served under:
    /// anything else is another site resolving to localhost (DNS rebinding).
    fn host_ok(&self, req: &Request) -> bool {
        let host = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Host"))
            .map(|h| h.value.as_str().to_string());
        let port = self.port;
        host.is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"))
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
        let mut body = String::new();
        if *req.method() == Method::Post {
            let _ = req.as_reader().read_to_string(&mut body);
        }
        let body: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let num = |k: &str| q.get(k).and_then(|v| v.parse::<usize>().ok());
        match path {
            "/api/review" => {
                let slots = self.slots.lock().expect("slots");
                let slot = &slots[&rev];
                json(req, view::slot(slot))
            }
            "/api/file" => {
                let p = q.get("path").cloned().unwrap_or_default();
                let slots = self.slots.lock().expect("slots");
                match &slots[&rev].phase {
                    Phase::Ready(app) => json(req, view::file(app, &p)),
                    _ => fail(req, 409, "the review is still loading"),
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
            let mut slots = self.slots.lock().expect("slots");
            let Phase::Ready(app) = &mut slots.get_mut(rev).expect("slot").phase else {
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
            let mut slots = self.slots.lock().expect("slots");
            if let Phase::Ready(app) = &mut slots.get_mut(rev).expect("slot").phase {
                app.churn_cache.insert(key, churn);
            }
        }
        let slots = self.slots.lock().expect("slots");
        match &slots[rev].phase {
            Phase::Ready(app) => json(req, view::item(app, i)),
            _ => fail(req, 409, "the review is still loading"),
        }
    }

    fn act(&self, req: Request, rev: &str, body: &serde_json::Value) {
        let mut slots = self.slots.lock().expect("slots");
        let slot = slots.get_mut(rev).expect("slot");
        slot.touched = Instant::now();
        let Phase::Ready(app) = &mut slot.phase else {
            return fail(req, 409, "the review is still loading");
        };
        let int = |k: &str| body.get(k).and_then(|v| v.as_u64()).map(|v| v as usize);
        if let Some(sel) = int("sel").filter(|&s| s < app.items.len()) {
            app.sel = sel;
            app.cursor = Cursor {
                line: int("line").unwrap_or_else(|| start_cursor(&app.items[sel]).line),
                col: 0,
            };
            app.selection = int("from");
        }
        let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
        let mut extra = serde_json::Map::new();
        let result: Result<(), String> = match op {
            "mark" => {
                let i = app.sel;
                app.reviewed[i] = !app.reviewed[i];
                persist_mark(app, i);
                Ok(())
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
            "command" => {
                let line = body.get("line_text").and_then(|v| v.as_str()).unwrap_or("");
                command(app, line, &mut extra)
            }
            _ => Err(format!("unknown action '{op}'")),
        };
        slot.version += 1;
        let version = slot.version;
        let mut out = serde_json::json!({ "version": version });
        if let Err(e) = result {
            out["error"] = e.into();
        }
        for (k, v) in extra {
            out[k] = v;
        }
        drop(slots);
        self.broadcast(rev, "changed", serde_json::json!({ "version": version }));
        json(req, out)
    }

    /// A server-sent event stream for one revision: load progress, `changed`
    /// when the review would read differently. Written straight to the socket
    /// and flushed per event — a chunked body would sit in its buffer — with a
    /// comment every so often, so a closed tab is noticed and dropped.
    fn events(&self, req: Request, rev: String) {
        let (tx, rx) = mpsc::channel::<String>();
        self.subs.lock().expect("subs").push((rev, tx));
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
