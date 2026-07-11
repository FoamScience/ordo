//! ordo-tui — a terminal reviewer that is a pure client of the ordo engine.
//! It owns git (shells out for a commit's blobs), calls `ordo::run`, and renders
//! the change in comprehension order: the full file with the changed hunk
//! highlighted in context, plus rationale, advisories and def→use edges. The
//! engine stays git-free; gated behind the `tui` feature so the default build
//! never pulls a UI stack.
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
const PAGE: u16 = 15;

const USAGE: &str = "\
ordo-tui — interactive review of a commit, ordered for comprehension.

usage:
  ordo-tui [<rev>]     review <rev> vs its parent (default: HEAD)
  ordo-tui --help
  ordo-tui --version

<rev> is any git commit-ish (a sha, HEAD~2, a tag). Its diff is ordered by
def→use with rationale, structural notes, advisories and PR-split clusters.

keys:
  j / down     next hunk           k / up       previous hunk
  space / f    page down (code)    b            page up (code)
  x            toggle reviewed      g / G        first / last
  q / Esc      quit
";

fn parse_args() -> Result<String, i32> {
    let mut rev: Option<String> = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                return Err(0);
            }
            "-V" | "--version" | "version" => {
                println!(
                    "ordo-tui {} (ordo schema {})",
                    env!("CARGO_PKG_VERSION"),
                    ordo::SCHEMA_VERSION
                );
                return Err(0);
            }
            s if s.starts_with('-') => {
                eprintln!("ordo-tui: unknown flag '{s}'\n\n{USAGE}");
                return Err(2);
            }
            s if rev.is_some() => {
                eprintln!("ordo-tui: unexpected extra argument '{s}'\n\n{USAGE}");
                return Err(2);
            }
            s => rev = Some(s.to_string()),
        }
    }
    Ok(rev.unwrap_or_else(|| "HEAD".to_string()))
}

fn main() -> std::io::Result<()> {
    let rev = match parse_args() {
        Ok(rev) => rev,
        Err(code) => std::process::exit(code),
    };
    if git(&["rev-parse", "--verify", "-q", &format!("{rev}^{{commit}}")])
        .trim()
        .is_empty()
    {
        eprintln!("ordo-tui: not a git repository, or unknown revision '{rev}'");
        std::process::exit(1);
    }
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
    let items = build_items(&out);
    if items.is_empty() {
        eprintln!("ordo-tui: nothing to review in {rev}");
        return Ok(());
    }
    let scroll = auto_scroll(&items[0]);
    run(
        App {
            reviewed: vec![false; items.len()],
            items,
            sel: 0,
            scroll,
            sources,
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
    path: String,
    old_range: [usize; 2],
    new_range: [usize; 2],
    label: String,
    rationale: String,
    notes: Vec<String>,
    edges: Vec<String>,
    advisories: Vec<(String, String, bool)>,
    noise: bool,
}

struct App {
    items: Vec<Item>,
    reviewed: Vec<bool>,
    sel: usize,
    scroll: u16,
    sources: Sources,
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
                path: path.to_string(),
                old_range: h.old_range,
                new_range: h.new_range,
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

// position the code view so the hunk sits a few lines below the top
fn auto_scroll(it: &Item) -> u16 {
    let [o0, o1] = it.old_range;
    let removed = if o0 >= 1 && o0 <= o1 { o1 - o0 + 1 } else { 0 };
    ((it.new_range[0].saturating_sub(1) + removed).saturating_sub(3)) as u16
}

// ------------------------------------------------------------------- code view

// The whole new file with the changed hunk highlighted in place: removed lines
// (red) shown at the change point, added/changed lines (green) marked, the rest
// as plain context so a reviewer sees the full surrounding code.
fn code_view<'a>(it: &Item, sources: &'a Sources) -> Vec<Line<'a>> {
    let mut out = vec![];
    let Some((ol, nl)) = sources.get(&it.path) else {
        return out;
    };
    let [o0, o1] = it.old_range;
    let [n0, n1] = it.new_range;
    let removed: Vec<&String> = if o0 >= 1 && o0 <= o1 && o1 <= ol.len() {
        ol[o0 - 1..o1].iter().collect()
    } else {
        vec![]
    };
    let red = Style::default().fg(Color::Red);
    let green = Style::default().fg(Color::Green);
    let num = Style::default().fg(Color::DarkGray);
    let emit_removed = |out: &mut Vec<Line<'a>>| {
        for r in &removed {
            out.push(Line::from(Span::styled(format!("    - {r}"), red)));
        }
    };
    for (i, line) in nl.iter().enumerate() {
        let ln = i + 1;
        if ln == n0 {
            emit_removed(&mut out);
        }
        if n0 <= ln && ln <= n1 {
            out.push(Line::from(vec![
                Span::styled(format!("{ln:>4} "), num),
                Span::styled(format!("+ {line}"), green),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::styled(format!("{ln:>4}   "), num),
                Span::raw(line.as_str()),
            ]));
        }
    }
    if n0 > nl.len() {
        emit_removed(&mut out); // deletion at/after EOF
    }
    out
}

// --------------------------------------------------------------------- tui loop

fn run(mut app: App, rev: &str) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let n = app.items.len();
    let result = loop {
        if let Err(e) = terminal.draw(|f| draw(f, &app, rev)) {
            break Err(e);
        }
        let goto = |app: &mut App, to: usize| {
            app.sel = to;
            app.scroll = auto_scroll(&app.items[to]);
        };
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                KeyCode::Down | KeyCode::Char('j') if app.sel + 1 < n => {
                    let to = app.sel + 1;
                    goto(&mut app, to);
                }
                KeyCode::Up | KeyCode::Char('k') if app.sel > 0 => {
                    let to = app.sel - 1;
                    goto(&mut app, to);
                }
                KeyCode::Char('g') => goto(&mut app, 0),
                KeyCode::Char('G') => goto(&mut app, n - 1),
                KeyCode::Char('x') => {
                    app.reviewed[app.sel] = !app.reviewed[app.sel];
                    if app.sel + 1 < n {
                        let to = app.sel + 1;
                        goto(&mut app, to);
                    }
                }
                KeyCode::PageDown | KeyCode::Char(' ' | 'f') => {
                    app.scroll = app.scroll.saturating_add(PAGE)
                }
                KeyCode::PageUp | KeyCode::Char('b') => {
                    app.scroll = app.scroll.saturating_sub(PAGE)
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
    let cols = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(f.area());

    // left — reading order
    let rows: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let check = if app.reviewed[i] { "✓" } else { " " };
            let style = if it.noise {
                Style::default().fg(Color::DarkGray)
            } else if app.reviewed[i] {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(format!("{check}{}", it.label)).style(style)
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

    // right — code (top) + why (bottom)
    let rhs =
        Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).split(cols[1]);
    let it = &app.items[app.sel];

    let code = code_view(it, &app.sources);
    let max_scroll = code.len().saturating_sub(1) as u16;
    let scroll = app.scroll.min(max_scroll);
    let code_view = Paragraph::new(Text::from(code))
        .block(Block::bordered().title(format!(
            " {}  (space/b scroll · x review · q quit) ",
            it.path
        )))
        .scroll((scroll, 0));
    f.render_widget(code_view, rhs[0]);

    let mut why = vec![Line::from(Span::styled(
        format!("why: {}", it.rationale),
        Style::default().fg(Color::Cyan),
    ))];
    if !it.notes.is_empty() {
        why.push(Line::from(Span::styled(
            format!("notes: {}", it.notes.join("; ")),
            Style::default().fg(Color::Yellow),
        )));
    }
    for e in &it.edges {
        why.push(Line::from(format!("dep {e}")));
    }
    for (construct, message, verdict) in &it.advisories {
        let (head, color) = if *verdict {
            (format!("⚠ {construct}"), Color::Red)
        } else {
            (construct.clone(), Color::Magenta)
        };
        why.push(Line::from(Span::styled(
            head,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));
        for ml in message.lines() {
            why.push(Line::from(format!("  {ml}")));
        }
    }
    let info = Paragraph::new(Text::from(why))
        .block(Block::bordered().title(" why "))
        .wrap(Wrap { trim: false });
    f.render_widget(info, rhs[1]);
}
