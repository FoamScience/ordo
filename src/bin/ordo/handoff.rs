// ------------------------------------------------------------ review handoff
//! The review, handed back to whoever wrote the change — usually an agent. One
//! markdown message, each point anchored to a file and a line of the new side
//! so it can be acted on without this screen: `:yank` copies it, `:send` pipes
//! it to the command in `ORDO_SEND`.

use crate::commands::review_comments;
use crate::marks::note_key;
use crate::App;
use base64::Engine;
use std::fmt::Write;
use std::io::Write as _;
use std::process::{Command, Stdio};

/// The command `:send` pipes the review to, on its stdin. The herdr plugin
/// sets it to deliver to the workspace's agent; anyone can point it at
/// anything that reads a prompt.
pub(super) const SEND_ENV: &str = "ORDO_SEND";
/// How long `:send` waits on that command before giving up on it.
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// The review as a prompt: the reviewer's notes and line comments, and with
/// `all` ordo's own notes and findings too. `None` when there is nothing to say.
pub(super) fn review_prompt(app: &App, all: bool) -> Option<String> {
    let mut points = String::new();
    let mut n = 0;
    for it in &app.items {
        let mine = note_key(it).and_then(|k| app.notes.get(&k));
        let ordo: Vec<&str> = if all {
            it.notes
                .iter()
                .map(String::as_str)
                .chain(it.findings.iter().filter_map(|f| f.message.lines().next()))
                .collect()
        } else {
            vec![]
        };
        if mine.is_none() && ordo.is_empty() {
            continue;
        }
        let [start, end] = it.new_range;
        let at = if end > start {
            format!("{}:{start}-{end}", it.path)
        } else {
            format!("{}:{start}", it.path)
        };
        let _ = writeln!(points, "\n## {at}");
        // the turn that wrote it, so the agent answers in terms of what it
        // was doing then
        if let Some(w) = it.wave {
            match app.waves.asked(w) {
                Some(asked) => {
                    let _ = writeln!(points, "(wave {w}, when you were asked: {asked})");
                }
                None => {
                    let _ = writeln!(points, "(wave {w})");
                }
            }
        }
        if let Some(note) = mine {
            let _ = writeln!(points, "{note}");
            n += 1;
        }
        for o in ordo {
            let _ = writeln!(points, "- ordo: {o}");
            n += 1;
        }
    }
    for c in review_comments(app) {
        let stale = if c.stale {
            " (these lines changed after the comment was written)"
        } else {
            ""
        };
        let _ = writeln!(points, "\n## {}{stale}\n{}", c.at(), c.text);
        n += 1;
    }
    (n > 0).then(|| {
        format!(
            "Review of {} ({n} point{}). Each is anchored to a file and a line \
             range of the changed code; address them, or say why not.\n{points}",
            app.rev,
            if n == 1 { "" } else { "s" },
        )
    })
}

/// The escape that asks the terminal to put `text` on the system clipboard.
/// Works over ssh and inside a multiplexer that passes it through, where
/// no clipboard program on this machine would help.
pub(super) fn osc52(text: &str) -> String {
    format!(
        "\x1b]52;c;{}\x07",
        base64::engine::general_purpose::STANDARD.encode(text)
    )
}

pub(super) fn yank(text: &str) -> Result<(), String> {
    let mut out = std::io::stdout();
    out.write_all(osc52(text).as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| format!("could not write to the terminal: {e}"))
}

/// Pipes `text` to the `ORDO_SEND` command. Its output is kept off the screen
/// the review is drawn on; a failure comes back with its first stderr line.
pub(super) fn send(text: &str) -> Result<String, String> {
    let cmd = std::env::var(SEND_ENV)
        .ok()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            format!("set {SEND_ENV} to a command that reads the review on stdin, or :yank it")
        })?;
    let mut child = Command::new("sh")
        .args(["-c", &cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{SEND_ENV}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // a command that exits without reading closes the pipe; its exit
        // status and stderr below say more than the broken pipe would
        match stdin.write_all(text.as_bytes()) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
                return Err(format!("{SEND_ENV}: {e}"));
            }
            _ => {}
        }
    }
    // the review is drawn by this same thread: a command that never returns
    // must not freeze it
    let deadline = std::time::Instant::now() + SEND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{SEND_ENV} gave no answer in {}s and was stopped",
                    SEND_TIMEOUT.as_secs()
                ));
            }
            Err(e) => return Err(format!("{SEND_ENV}: {e}")),
        }
    };
    if status.success() {
        return Ok(cmd);
    }
    let mut why = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut err, &mut why);
    }
    Err(format!(
        "{SEND_ENV} failed ({status}): {}",
        why.lines().next().unwrap_or("")
    ))
}
