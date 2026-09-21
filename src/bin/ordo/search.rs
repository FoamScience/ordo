// --------------------------------------------------------------------- search
use crate::code_view::identifier_at;
use crate::code_view::node_text;
use crate::code_view::parse_cached;
use crate::follow_hscroll;
use crate::follow_scroll;
use crate::highlight::highlight_spec;
use crate::App;
use crate::Cursor;
use crate::Search;
use crate::SearchKind;
use tree_sitter::Node;

// inverse of `char_byte`: the char index a byte offset falls at within one line
pub(super) fn byte_to_char_col(line: &str, byte_col: usize) -> usize {
    line.char_indices()
        .take_while(|(b, _)| *b < byte_col)
        .count()
}

/// Every identifier node in the tree whose text equals `name`, as document-order
/// (line, start_col, end_col) char ranges. Never a text scan: a short name that
/// occurs as a substring of a longer identifier (`may_refine` inside
/// `may_refine_camber_span`) is never counted, because node text equality is
/// exact, not a substring test.
pub(super) fn symbol_matches(
    root: Node,
    name: &str,
    src: &str,
    lines: &[String],
) -> Vec<(usize, usize, usize)> {
    let mut out = vec![];
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind().contains("identifier") && node_text(n, src) == name {
            let (sp, ep) = (n.start_position(), n.end_position());
            if sp.row == ep.row && sp.row < lines.len() {
                let scol = byte_to_char_col(&lines[sp.row], sp.column);
                let ecol = byte_to_char_col(&lines[sp.row], ep.column);
                out.push((sp.row, scol, ecol));
            }
        }
        let mut cursor = n.walk();
        stack.extend(n.children(&mut cursor));
    }
    out.sort();
    out
}

/// `/` text search: literal substring occurrences of `pattern`, one per
/// character position. This IS a text scan — the one place that's correct,
/// since the user is typing characters, not asking about a symbol.
pub(super) fn text_matches(lines: &[String], pattern: &str) -> Vec<(usize, usize, usize)> {
    if pattern.is_empty() {
        return vec![];
    }
    let plen = pattern.chars().count();
    let mut out = vec![];
    for (i, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < plen {
            continue;
        }
        for start in 0..=chars.len() - plen {
            if chars[start..start + plen].iter().collect::<String>() == pattern {
                out.push((i, start, start + plen));
            }
        }
    }
    out
}

// first match at/after (inclusive) or strictly after (!inclusive) `cursor`,
// wrapping to the first match when nothing qualifies
pub(super) fn seek_forward(
    matches: &[(usize, usize, usize)],
    cursor: Cursor,
    inclusive: bool,
) -> Option<usize> {
    let after = |l: usize, s: usize| {
        if inclusive {
            (l, s) >= (cursor.line, cursor.col)
        } else {
            (l, s) > (cursor.line, cursor.col)
        }
    };
    matches
        .iter()
        .position(|&(l, s, _)| after(l, s))
        .or((!matches.is_empty()).then_some(0))
}

// mirror of `seek_forward`, wrapping to the last match when nothing qualifies
pub(super) fn seek_backward(
    matches: &[(usize, usize, usize)],
    cursor: Cursor,
    inclusive: bool,
) -> Option<usize> {
    let before = |l: usize, s: usize| {
        if inclusive {
            (l, s) <= (cursor.line, cursor.col)
        } else {
            (l, s) < (cursor.line, cursor.col)
        }
    };
    matches
        .iter()
        .rposition(|&(l, s, _)| before(l, s))
        .or_else(|| matches.len().checked_sub(1))
}

// `n`/`N` index arithmetic: advance/retreat by one, wrapping. `len == 0` never
// panics (rem_euclid by zero would) — it's the empty-result case.
pub(super) fn cycle_index(index: usize, len: usize, dir: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + dir).rem_euclid(len as isize) as usize
}

// move the cursor to `search`'s current match and scroll to follow, then store it
pub(super) fn jump_search(app: &mut App, mut search: Search, forward: bool, inclusive: bool) {
    if !search.matches.is_empty() {
        let idx = if forward {
            seek_forward(&search.matches, app.cursor, inclusive)
        } else {
            seek_backward(&search.matches, app.cursor, inclusive)
        };
        search.index = idx.unwrap_or(0);
        let (line, col, _) = search.matches[search.index];
        app.cursor = Cursor { line, col };
        app.scroll = follow_scroll(line, app.scroll, app.code_height);
        app.hscroll = follow_hscroll(col, app.hscroll, app.code_width);
    }
    app.search = Some(search);
}

/// `*` / `#` — the symbol under the cursor, resolved via tree-sitter (sharing
/// the `K` popup's parse cache), jumping to the next/previous occurrence.
pub(super) fn symbol_search(app: &mut App, forward: bool) {
    let path = app.items[app.sel].path.clone();
    let Some((_, nl)) = app.sources.get(&path) else {
        return;
    };
    if nl.is_empty() {
        return;
    }
    let Some((lang, _)) = highlight_spec(&path) else {
        return;
    };
    let Some(parsed) = parse_cached(&mut app.trees, &path, nl, lang) else {
        return;
    };
    let Some(node) = identifier_at(parsed, nl, app.cursor) else {
        return;
    };
    let name = node_text(node, &parsed.src);
    let matches = symbol_matches(parsed.tree.root_node(), &name, &parsed.src, nl);
    let search = Search {
        kind: SearchKind::Symbol,
        pattern: name,
        matches,
        index: 0,
    };
    // strict inequality: the occurrence the cursor is already on doesn't count
    // as "next"
    jump_search(app, search, forward, false);
}

/// `n` / `N` — cycle through the currently active search's matches, wrapping.
pub(super) fn cycle_search(app: &mut App, dir: isize) {
    let Some(search) = app.search.as_mut() else {
        return;
    };
    if search.matches.is_empty() {
        return;
    }
    search.index = cycle_index(search.index, search.matches.len(), dir);
    let (line, col, _) = search.matches[search.index];
    app.cursor = Cursor { line, col };
    app.scroll = follow_scroll(line, app.scroll, app.code_height);
    app.hscroll = follow_hscroll(col, app.hscroll, app.code_width);
}

/// `Enter` on the `/` prompt: run the text search and jump to the first match
/// at or after the cursor.
pub(super) fn accept_search(app: &mut App) {
    let Some(prompt) = app.prompt.take() else {
        return;
    };
    let path = app.items[app.sel].path.clone();
    let matches = match app.sources.get(&path) {
        Some((_, nl)) => text_matches(nl, &prompt.text),
        None => vec![],
    };
    let search = Search {
        kind: SearchKind::Text,
        pattern: prompt.text,
        matches,
        index: 0,
    };
    jump_search(app, search, true, true);
}

/// `Esc` on the `/` prompt: cancel, restoring the cursor position from before
/// the prompt opened.
pub(super) fn cancel_search(app: &mut App) {
    if let Some(prompt) = app.prompt.take() {
        app.cursor = prompt.anchor;
        app.scroll = follow_scroll(app.cursor.line, app.scroll, app.code_height);
        app.hscroll = follow_hscroll(app.cursor.col, app.hscroll, app.code_width);
    }
}
