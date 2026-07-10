//! ordo-tui — a terminal reviewer that is a pure client of the ordo engine.
//! It owns git (shells out for a commit's blobs), calls `ordo::run`, and renders
//! the change in comprehension order with the diff, rationale, advisories and
//! def→use edges. The engine stays git-free; this binary is gated behind the
//! `tui` feature so the default build never pulls a UI stack.
use std::collections::HashMap;
use std::process::Command;

use ordo::model::{Change, Input, Options, Output};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const MAX_DIFF_LINES: usize = 40;

fn main() -> std::io::Result<()> {
    let rev = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "HEAD".to_string());
    let input = gather(&rev);
    let sources: Sources = input
        .changes
        .iter()
        .map(|c| {
            let split = |s: Option<&String>| {
                s.map(|t| t.lines().map(String::from).collect())
                    .unwrap_or_default()
            };
            (
                c.path.clone(),
                (split(c.old.as_ref()), split(c.new.as_ref())),
            )
        })
        .collect();
    let out = ordo::run(input);
    let items = build_items(&out, &sources);
    if items.is_empty() {
        eprintln!("ordo-tui: nothing to review in {rev}");
        return Ok(());
    }
    run(
        App {
            reviewed: vec![false; items.len()],
            items,
            sel: 0,
        },
        &rev,
    )
}

// ------------------------------------------------------------------- git layer

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn gather(rev: &str) -> Input {
    let parent = git(&["rev-parse", "--verify", "-q", &format!("{rev}^")]);
    let parent = parent.trim();
    let parent = if parent.is_empty() {
        EMPTY_TREE
    } else {
        parent
    };
    let names = git(&["diff-tree", "--no-commit-id", "--name-only", "-r", rev]);
    let changes = names
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|path| Change {
            path: path.to_string(),
            old: Some(git(&["show", &format!("{parent}:{path}")])),
            new: Some(git(&["show", &format!("{rev}:{path}")])),
            diff: None,
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// ------------------------------------------------------------------- view model

type Sources = HashMap<String, (Vec<String>, Vec<String>)>;

struct Item {
    header: String,
    label: String,
    rationale: String,
    notes: Vec<String>,
    edges: Vec<String>,
    advisories: Vec<(String, String, bool)>,
    diff: Vec<(char, String)>, // '-' old / '+' new
    extra_diff: usize,         // lines beyond MAX_DIFF_LINES
    noise: bool,
}

struct App {
    items: Vec<Item>,
    reviewed: Vec<bool>,
    sel: usize,
}

fn build_items(out: &Output, sources: &Sources) -> Vec<Item> {
    let by_id: HashMap<&str, (&str, &ordo::model::HunkOut)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks
                .iter()
                .map(move |h| (h.id.as_str(), (f.path.as_str(), h)))
        })
        .collect();
    let loc = |id: &str| {
        by_id
            .get(id)
            .map(|(p, h)| format!("{p}:L{}", h.new_range[0]))
            .unwrap_or_else(|| id.to_string())
    };
    out.order
        .iter()
        .filter_map(|o| {
            let (path, h) = by_id.get(o.hunk.as_str())?;
            let cat = format!("{:?}", h.category).to_lowercase();
            let mark = if !h.advisories.is_empty() {
                "⚠"
            } else if h.noise {
                "·"
            } else {
                " "
            };
            let edges = out
                .edges
                .iter()
                .filter(|e| e.from == h.id || e.to == h.id)
                .map(|e| {
                    if e.from == h.id {
                        format!("→ {}   {}", loc(&e.to), e.why)
                    } else {
                        format!("← {}   {}", loc(&e.from), e.why)
                    }
                })
                .collect();

            let (mut diff, mut extra_diff) = (vec![], 0);
            if let Some((ol, nl)) = sources.get(*path) {
                let slice =
                    |lines: &[String], r: [usize; 2], sign: char, out: &mut Vec<(char, String)>| {
                        if r[0] >= 1 && r[0] <= r[1] && r[1] <= lines.len() {
                            for l in &lines[r[0] - 1..r[1]] {
                                out.push((sign, l.clone()));
                            }
                        }
                    };
                slice(ol, h.old_range, '-', &mut diff);
                slice(nl, h.new_range, '+', &mut diff);
                if diff.len() > MAX_DIFF_LINES {
                    extra_diff = diff.len() - MAX_DIFF_LINES;
                    diff.truncate(MAX_DIFF_LINES);
                }
            }

            Some(Item {
                header: format!("{path}:L{}", h.new_range[0]),
                label: format!("{mark} {path}:L{} [{cat}] {}", h.new_range[0], h.rationale),
                rationale: h.rationale.clone(),
                notes: h.notes.clone(),
                edges,
                advisories: h
                    .advisories
                    .iter()
                    .map(|a| (a.construct.clone(), a.message.clone(), a.verdict))
                    .collect(),
                diff,
                extra_diff,
                noise: h.noise,
            })
        })
        .collect()
}

// --------------------------------------------------------------------- tui loop

fn run(mut app: App, rev: &str) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let n = app.items.len();
    let result = loop {
        if let Err(e) = terminal.draw(|f| draw(f, &app, rev)) {
            break Err(e);
        }
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.sel + 1 < n {
                        app.sel += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => app.sel = app.sel.saturating_sub(1),
                KeyCode::Char('g') => app.sel = 0,
                KeyCode::Char('G') => app.sel = n - 1,
                KeyCode::Char('x') => {
                    app.reviewed[app.sel] = !app.reviewed[app.sel];
                    if app.sel + 1 < n {
                        app.sel += 1;
                    }
                }
                _ => {}
            },
            Ok(_) => {}
            Err(e) => break Err(e),
        }
    };
    ratatui::restore();
    result
}

fn draw(f: &mut Frame, app: &App, rev: &str) {
    let cols = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(f.area());
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let rows: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let check = if app.reviewed[i] { "✓" } else { " " };
            let base = if it.noise {
                Style::default().fg(Color::DarkGray)
            } else if app.reviewed[i] {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(format!("{check}{}", it.label)).style(base)
        })
        .collect();
    let done = app.reviewed.iter().filter(|r| **r).count();
    let mut state = ListState::default();
    state.select(Some(app.sel));
    let list = List::new(rows)
        .block(Block::bordered().title(format!(" {rev} — {done}/{} reviewed ", app.items.len())))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, cols[0], &mut state);

    let it = &app.items[app.sel];
    let mut lines = vec![
        Line::from(Span::styled(it.header.clone(), bold)),
        Line::from(""),
    ];
    for (sign, text) in &it.diff {
        let color = if *sign == '-' {
            Color::Red
        } else {
            Color::Green
        };
        lines.push(Line::from(Span::styled(
            format!("{sign} {text}"),
            Style::default().fg(color),
        )));
    }
    if it.extra_diff > 0 {
        lines.push(Line::from(Span::styled(
            format!("  … {} more lines", it.extra_diff),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("why: {}", it.rationale),
        Style::default().fg(Color::Cyan),
    )));
    if !it.notes.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("notes: {}", it.notes.join("; ")),
            Style::default().fg(Color::Yellow),
        )));
    }
    if !it.edges.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("dependencies", bold)));
        for e in &it.edges {
            lines.push(Line::from(format!("  {e}")));
        }
    }
    for (construct, message, verdict) in &it.advisories {
        lines.push(Line::from(""));
        let (head, color) = if *verdict {
            (format!("⚠ {construct}"), Color::Red)
        } else {
            (construct.clone(), Color::Magenta)
        };
        lines.push(Line::from(Span::styled(
            head,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));
        for ml in message.lines() {
            lines.push(Line::from(format!("  {ml}")));
        }
    }
    let detail = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title(" detail   j/k move · x review · g/G ends · q quit "))
        .wrap(Wrap { trim: false });
    f.render_widget(detail, cols[1]);
}
