// ---------------------------------------------------------------------- help
//! `ordo help <topic>` on a terminal: the topic's markdown painted by the same
//! highlighter the code pane uses, with the HTML that only GitHub needs
//! dropped and pipe tables set out in columns.

use crate::code_view::Syntax;
use crate::highlight::{highlight_file, LineSpans};
use ratatui::crossterm::style::{ResetColor, SetForegroundColor};
use ratatui::style::Color;
use ratatui::text::Line;

type Cell = Vec<(char, Color)>;

/// `columns` is the terminal's width, which a table is fitted to.
pub(super) fn render_markdown(src: &str, syn: &Syntax, dim: Color, columns: usize) -> String {
    let Some(lines) = highlight_file("help.md", src, syn) else {
        return src.to_string();
    };
    let mut out = String::new();
    let mut table: Vec<Vec<Cell>> = vec![];
    let mut fenced = false;
    for (text, spans) in src.lines().zip(&lines) {
        let t = text.trim();
        if t.starts_with("```") {
            fenced = !fenced;
        }
        if t.starts_with('|') && !fenced {
            table.push(
                cells(&chars(spans), syn)
                    .into_iter()
                    .map(unescape)
                    .collect(),
            );
            continue;
        }
        flush_table(&mut out, &mut table, columns, dim);
        let html_only =
            t == "<details>" || t == "</details>" || (t.starts_with("<!--") && t.ends_with("-->"));
        if html_only && !fenced {
            continue;
        }
        if let Some(summary) = t.strip_prefix("<summary>").filter(|_| !fenced) {
            let title = strip_tags(summary.trim_end_matches("</summary>"));
            paint(
                &mut out,
                &title.chars().map(|c| (c, syn.function)).collect::<Vec<_>>(),
            );
        } else {
            paint(&mut out, &unescape(chars(spans)));
        }
        out.push('\n');
    }
    flush_table(&mut out, &mut table, columns, dim);
    out
}

fn chars(spans: &LineSpans) -> Cell {
    spans
        .iter()
        .flat_map(|(s, c)| s.chars().map(move |ch| (ch, *c)))
        .collect()
}

/// The HTML entities the docs use, and `\|` inside a table.
fn unescape(line: Cell) -> Cell {
    let mut out = Cell::new();
    let mut i = 0;
    while i < line.len() {
        let rest: String = line[i..].iter().take(5).map(|(c, _)| *c).collect();
        let (ch, skip) = match () {
            _ if rest.starts_with("&lt;") => ('<', 4),
            _ if rest.starts_with("&gt;") => ('>', 4),
            _ if rest.starts_with("&amp;") => ('&', 5),
            _ if rest.starts_with("\\|") => ('|', 2),
            _ => (line[i].0, 1),
        };
        out.push((ch, line[i].1));
        i += skip;
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// A table row split on its `|`s, except an escaped `\|` and one inside a
/// code span, which the docs write unescaped (`:watch [on|off|auto]`). The
/// grammar leaves cell text unhighlighted, so code spans are painted here.
fn cells(row: &Cell, syn: &Syntax) -> Vec<Cell> {
    let mut out = vec![];
    let mut cur = Cell::new();
    let mut code = false;
    let mut prev = ' ';
    for &(ch, col) in row {
        let escaped = prev == '\\';
        prev = ch;
        match ch {
            '`' => {
                code = !code;
                cur.push((ch, syn.operator));
            }
            '|' if !code && !escaped => out.push(std::mem::take(&mut cur)),
            _ if code => cur.push((ch, syn.string)),
            _ => cur.push((ch, col)),
        }
    }
    out.push(cur);
    // the row's leading and trailing pipes leave an empty cell at each end
    out.remove(0);
    if out
        .last()
        .is_some_and(|c| c.iter().all(|(ch, _)| ch.is_whitespace()))
    {
        out.pop();
    }
    out.into_iter().map(trim).collect()
}

fn trim(c: Vec<(char, Color)>) -> Cell {
    let start = c
        .iter()
        .position(|(ch, _)| !ch.is_whitespace())
        .unwrap_or(c.len());
    let end = c
        .iter()
        .rposition(|(ch, _)| !ch.is_whitespace())
        .map_or(start, |e| e + 1);
    c[start..end].to_vec()
}

fn width(c: &Cell) -> usize {
    Line::raw(c.iter().map(|(ch, _)| *ch).collect::<String>()).width()
}

fn is_rule(row: &[Cell]) -> bool {
    row.iter()
        .all(|c| !c.is_empty() && c.iter().all(|(ch, _)| matches!(ch, '-' | ':' | ' ')))
}

/// Narrowest a column is squeezed to before a table is let run past the edge.
const MIN_COL: usize = 12;

fn flush_table(out: &mut String, table: &mut Vec<Vec<Cell>>, columns: usize, dim: Color) {
    let rows = std::mem::take(table);
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths: Vec<usize> = (0..cols)
        .map(|i| {
            rows.iter()
                .filter(|r| !is_rule(r))
                .filter_map(|r| r.get(i).map(width))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let budget = columns.saturating_sub(3 * cols.saturating_sub(1));
    while widths.iter().sum::<usize>() > budget {
        let Some(widest) = (0..cols).max_by_key(|&i| widths[i]) else {
            break;
        };
        if widths[widest] <= MIN_COL {
            break;
        }
        widths[widest] -= 1;
    }
    let sep = |s: &str| s.chars().map(|c| (c, dim)).collect::<Cell>();
    for row in &rows {
        if is_rule(row) {
            let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            paint(out, &sep(&rule.join("─┼─")));
            out.push('\n');
            continue;
        }
        let wrapped: Vec<Vec<Cell>> = widths
            .iter()
            .enumerate()
            .map(|(i, w)| wrap(row.get(i).map_or(&[][..], Vec::as_slice), *w))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for n in 0..height {
            let mut line = Cell::new();
            for (i, w) in widths.iter().enumerate() {
                if i > 0 {
                    line.extend(sep(" │ "));
                }
                let piece = wrapped[i].get(n).cloned().unwrap_or_default();
                let pad = w.saturating_sub(width(&piece));
                line.extend(piece);
                if i + 1 < cols {
                    line.extend(sep(&" ".repeat(pad)));
                }
            }
            paint(out, &line);
            out.push('\n');
        }
    }
}

/// A cell broken into lines of at most `w` columns, at spaces where it can be
/// and mid-word where a word is itself too long.
fn wrap(cell: &[(char, Color)], w: usize) -> Vec<Cell> {
    let mut lines = vec![Cell::new()];
    for word in cell.split_inclusive(|(ch, _)| *ch == ' ') {
        let cur = lines.last().expect("a current line");
        let word_w = width(&trim(word.to_vec()));
        if !cur.is_empty() && width(cur) + word_w > w {
            lines.push(Cell::new());
        }
        for &c in word {
            let cur = lines.last_mut().expect("a current line");
            if cur.is_empty() && c.0 == ' ' {
                continue;
            }
            if width(cur) >= w && c.0 != ' ' {
                lines.push(vec![c]);
            } else {
                cur.push(c);
            }
        }
    }
    lines.into_iter().map(trim).collect()
}

fn paint(out: &mut String, line: &[(char, Color)]) {
    let mut current = None;
    for &(ch, col) in line {
        if current != Some(col) {
            out.push_str(&SetForegroundColor(col.into()).to_string());
            current = Some(col);
        }
        out.push(ch);
    }
    if current.is_some() {
        out.push_str(&ResetColor.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> String {
        let mut out = String::new();
        let mut esc = false;
        for c in s.chars() {
            match c {
                '\x1b' => esc = true,
                'm' if esc => esc = false,
                _ if !esc => out.push(c),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn github_only_html_goes_and_tables_line_up() {
        let md = "# Title\n\n<details>\n<summary><b>Keys</b> (<code>vim</code>)</summary>\n\
                  <!-- ordo:begin keys -->\n| key | does |\n| --- | --- |\n\
                  | `q` | quit |\n| `:watch [on\\|off]` | follow &lt;tree&gt; |\n\
                  <!-- ordo:end keys -->\n</details>\n";
        let syn = crate::code_view::theme("dark").expect("dark").syn;
        let got = plain(&render_markdown(md, &syn, Color::DarkGray, 80));
        assert_eq!(
            got,
            "# Title\n\nKeys (vim)\n\
             key               │ does\n\
             ──────────────────┼──────────────\n\
             `q`               │ quit\n\
             `:watch [on|off]` │ follow <tree>\n"
        );
    }

    #[test]
    fn a_pipe_inside_a_code_fence_is_code_not_a_table() {
        let md = "```jsonnet\n|||\n  text\n|||\n```\n";
        let syn = crate::code_view::theme("dark").expect("dark").syn;
        assert_eq!(plain(&render_markdown(md, &syn, Color::DarkGray, 80)), md);
    }

    #[test]
    fn a_wide_table_wraps_to_the_terminal_without_losing_words() {
        let md = "| key | does |\n| --- | --- |\n| `gD` | open the dependency canvas: what this hunk needs, and what needs it |\n";
        let syn = crate::code_view::theme("dark").expect("dark").syn;
        let got = plain(&render_markdown(md, &syn, Color::DarkGray, 40));
        assert!(got.lines().all(|l| l.chars().count() <= 40), "{got}");
        let words = |s: &str| {
            s.split(|c: char| c.is_whitespace() || "│|─┼-".contains(c))
                .filter(|w| !w.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(words(&got), words(md), "{got}");
    }
}
