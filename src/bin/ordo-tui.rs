//! ordo-tui — a terminal reviewer that is a pure client of the ordo engine.
//! It owns git (shells out for a commit's blobs), calls `ordo::run`, and renders
//! the change in comprehension order with rationale, advisories and def→use
//! edges. The engine stays git-free; this binary is gated behind the `tui`
//! feature so the default build never pulls a UI stack.
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

fn main() -> std::io::Result<()> {
    let rev = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "HEAD".to_string());
    let out = ordo::run(gather(&rev));
    let items = build_items(&out);
    if items.is_empty() {
        eprintln!("ordo-tui: nothing to review in {rev}");
        return Ok(());
    }
    run(items, &rev)
}

// ------------------------------------------------------------------- git layer

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// Gather a commit's changed files as {path, old (parent blob), new (commit blob)}.
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

struct Item {
    header: String,
    label: String, // left-list line
    rationale: String,
    notes: Vec<String>,
    edges: Vec<String>,
    advisories: Vec<(String, String, bool)>, // construct, message, verdict
    noise: bool,
}

fn build_items(out: &Output) -> Vec<Item> {
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
                noise: h.noise,
            })
        })
        .collect()
}

// --------------------------------------------------------------------- tui loop

fn run(items: Vec<Item>, rev: &str) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let mut sel = 0usize;
    let result = loop {
        if let Err(e) = terminal.draw(|f| draw(f, &items, sel, rev)) {
            break Err(e);
        }
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                KeyCode::Down | KeyCode::Char('j') => {
                    if sel + 1 < items.len() {
                        sel += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => sel = sel.saturating_sub(1),
                KeyCode::Char('g') => sel = 0,
                KeyCode::Char('G') => sel = items.len() - 1,
                _ => {}
            },
            Ok(_) => {}
            Err(e) => break Err(e),
        }
    };
    ratatui::restore();
    result
}

fn draw(f: &mut Frame, items: &[Item], sel: usize, rev: &str) {
    let cols = Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(f.area());

    let rows: Vec<ListItem> = items
        .iter()
        .map(|it| {
            let style = if it.noise {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(it.label.clone()).style(style)
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(sel));
    let list = List::new(rows)
        .block(Block::bordered().title(format!(" {rev} — reading order ({}) ", items.len())))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, cols[0], &mut state);

    let it = &items[sel];
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines = vec![
        Line::from(Span::styled(it.header.clone(), bold)),
        Line::from(""),
        Line::from(it.rationale.clone()),
    ];
    if !it.notes.is_empty() {
        lines.push(Line::from(""));
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
            (construct.clone(), Color::Cyan)
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
        .block(Block::bordered().title(" detail   j/k move · g/G top/bottom · q quit "))
        .wrap(Wrap { trim: false });
    f.render_widget(detail, cols[1]);
}
