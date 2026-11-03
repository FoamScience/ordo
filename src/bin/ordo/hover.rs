// ---------------------------------------------------------------------- hover
use crate::highlight::highlight_spec;
use crate::history::history_lines;
use crate::App;
use crate::Cursor;
use crate::ParsedFile;
use crate::Popup;
use std::collections::HashMap;
use tree_sitter::Node;
use tree_sitter::Parser;
use tree_sitter::Point;

pub(super) fn node_text(n: Node, src: &str) -> String {
    src.get(n.start_byte()..n.end_byte())
        .unwrap_or("")
        .to_string()
}

// byte offset of a char column within one line — tree-sitter Points are byte-indexed
pub(super) fn char_byte(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

// node kinds counted as a "definition" worth showing, by file extension —
// deliberately a small, curated set rather than every grammar's declaration
// kinds, so a hover only ever lands on something with a clear signature/body.
pub(super) fn def_kinds(path: &str) -> &'static [&'static str] {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" | "xsh" | "xonsh" | "xonshrc" => &["function_definition", "class_definition"],
        "rs" => &[
            "function_item",
            "struct_item",
            "enum_item",
            "trait_item",
            "impl_item",
            "const_item",
            "static_item",
            "type_item",
        ],
        "js" | "jsx" | "mjs" | "cjs" => &[
            "function_declaration",
            "class_declaration",
            "method_definition",
        ],
        "ts" | "tsx" | "mts" | "cts" => &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
        ],
        "go" => &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
        ],
        "c" | "h" => &["function_definition", "struct_specifier", "enum_specifier"],
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => {
            &["function_definition", "class_specifier", "struct_specifier"]
        }
        "java" => &[
            "method_declaration",
            "class_declaration",
            "interface_declaration",
        ],
        _ => &[],
    }
}

// A definition's name — the `name` field where the grammar has one; C/C++
// function definitions don't (the identifier is buried in `declarator`), so
// fall back to hunting one down there, skipping the parameter list.
fn def_name(n: Node, src: &str) -> Option<String> {
    if let Some(name) = n.child_by_field_name("name") {
        return Some(node_text(name, src));
    }
    find_identifier(n.child_by_field_name("declarator")?, src)
}

fn find_identifier(n: Node, src: &str) -> Option<String> {
    if n.kind() == "identifier" || n.kind() == "field_identifier" {
        return Some(node_text(n, src));
    }
    let mut cursor = n.walk();
    let children: Vec<Node> = n.children(&mut cursor).collect();
    children
        .into_iter()
        .filter(|c| !c.kind().contains("parameter"))
        .find_map(|c| find_identifier(c, src))
}

// depth-first search of the whole tree for a definition-kind node named `name`
pub(super) fn find_definition<'a>(
    root: Node<'a>,
    kinds: &[&str],
    name: &str,
    src: &str,
) -> Option<Node<'a>> {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if kinds.contains(&n.kind()) && def_name(n, src).as_deref() == Some(name) {
            return Some(n);
        }
        let mut cursor = n.walk();
        stack.extend(n.children(&mut cursor));
    }
    None
}

// the def's header — its own text up to (not including) its body, so a
// multi-line signature still reads as just the signature
pub(super) fn signature(n: Node, src: &str) -> String {
    let end = n
        .child_by_field_name("body")
        .map(|b| b.start_byte())
        .unwrap_or(n.end_byte());
    src.get(n.start_byte()..end)
        .unwrap_or("")
        .trim_end()
        .to_string()
}

fn python_docstring(n: Node, src: &str) -> Option<String> {
    let body = n.child_by_field_name("body")?;
    let first = body.named_child(0)?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let expr = first.named_child(0)?;
    (expr.kind() == "string").then(|| node_text(expr, src))
}

// the run of `//` / `/** */` comment nodes immediately preceding the
// definition, stopping at the first blank line or non-comment sibling
fn leading_comment(n: Node, src: &str) -> Option<String> {
    let mut lines = vec![];
    let mut cur = n.prev_sibling();
    let mut expect_row = n.start_position().row;
    while let Some(c) = cur {
        if !c.kind().contains("comment") || c.end_position().row + 1 < expect_row {
            break;
        }
        lines.push(node_text(c, src).trim_end().to_string());
        expect_row = c.start_position().row;
        cur = c.prev_sibling();
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

pub(super) fn doc_for(path: &str, n: Node, src: &str) -> Option<String> {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" => python_docstring(n, src),
        _ => leading_comment(n, src),
    }
}

// Ensure `path`'s new-content is parsed and cached, returning the cached tree.
// Shared by `hover` and the symbol-occurrence search so both cache the same tree.
pub(super) fn parse_cached<'a>(
    trees: &'a mut HashMap<String, ParsedFile>,
    path: &str,
    nl: &[String],
    lang: tree_sitter::Language,
) -> Option<&'a ParsedFile> {
    if !trees.contains_key(path) {
        let src = nl.join("\n");
        let mut parser = Parser::new();
        parser.set_language(&lang).ok()?;
        let tree = parser.parse(&src, None)?;
        trees.insert(path.to_string(), ParsedFile { tree, src });
    }
    trees.get(path)
}

// The identifier node at `cursor`, widening from whatever node is directly
// under it until an `identifier`-kind ancestor is found.
pub(super) fn identifier_at<'a>(
    parsed: &'a ParsedFile,
    nl: &[String],
    cursor: Cursor,
) -> Option<Node<'a>> {
    let line = nl.get(cursor.line)?;
    let col = char_byte(line, cursor.col);
    let point = Point {
        row: cursor.line,
        column: col,
    };
    if let Some(mut node) = parsed
        .tree
        .root_node()
        .descendant_for_point_range(point, point)
    {
        loop {
            if node.kind().contains("identifier") {
                return Some(node);
            }
            match node.parent() {
                // stop widening at the line: an identifier further up the tree
                // begins somewhere else entirely
                Some(p) if p.start_position().row == cursor.line => node = p,
                _ => break,
            }
        }
    }
    first_identifier_on_line(parsed, cursor.line, col)
}

/// The definition this identifier names a *parameter* of, if it does. Looks up
/// from the identifier to the enclosing definition and checks that definition's
/// own parameter list — so a parameter reads the same whether the cursor is on
/// its declaration or on a use of it further down the body. A parameter has no
/// definition to find, and reporting "not defined in this file" sends the
/// reviewer looking through other files for something that was never there.
fn parameter_owner(node: Node, kinds: &[&str], src: &str) -> Option<String> {
    let name = node_text(node, src);
    let mut cur = node;
    loop {
        cur = cur.parent()?;
        if !kinds.contains(&cur.kind()) {
            continue;
        }
        let params = cur.child_by_field_name("parameters")?;
        if !subtree_names(params, src).contains(&name) {
            return None; // the enclosing definition binds it some other way
        }
        let owner = cur
            .child_by_field_name("name")
            .or_else(|| cur.child_by_field_name("declarator"))?;
        return Some(node_text(owner, src));
    }
}

/// Every identifier text inside a subtree.
fn subtree_names(node: Node, src: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind().contains("identifier") {
            out.push(node_text(n, src));
        }
        stack.extend(n.named_children(&mut cur));
    }
    out
}

/// The leftmost identifier node beginning at or after `col` on `row`.
fn first_identifier_on_line<'a>(
    parsed: &'a ParsedFile,
    row: usize,
    col: usize,
) -> Option<Node<'a>> {
    let mut cur = parsed.tree.walk();
    let mut stack = vec![parsed.tree.root_node()];
    let mut best: Option<Node<'a>> = None;
    while let Some(n) = stack.pop() {
        let (s, e) = (n.start_position(), n.end_position());
        if s.row > row || e.row < row {
            continue;
        }
        if n.kind().contains("identifier") && s.row == row && s.column >= col {
            if best.is_none_or(|b| s.column < b.start_position().column) {
                best = Some(n);
            }
            continue;
        }
        stack.extend(n.named_children(&mut cur));
    }
    best
}

/// Resolve the symbol under the cursor via tree-sitter (not by scanning
/// characters) and show its definition in a popup. Parses lazily,
/// caching the tree per path so repeated presses are cheap. `brief` is for a
/// pointer resting on a name: no git history, and no popup at all when there
/// is no definition to show.
pub(super) fn hover(app: &mut App, brief: bool) {
    let path = app.items[app.sel].path.clone();
    let Some((_, nl)) = app.sources.get(&path) else {
        return;
    };
    if nl.is_empty() {
        return;
    }
    let Some((lang, _)) = highlight_spec(&path) else {
        if brief {
            return;
        }
        app.popup = Some(Popup::new(
            "hover",
            vec!["no grammar available for this file type".to_string()],
        ));
        return;
    };
    let Some(parsed) = parse_cached(&mut app.trees, &path, nl, lang) else {
        return;
    };
    let Some(node) = identifier_at(parsed, nl, app.cursor) else {
        if !brief {
            app.popup = Some(Popup::new("hover", vec!["no symbol here".to_string()]));
        }
        return;
    };
    let name = node_text(node, &parsed.src);
    let root = parsed.tree.root_node();
    let kinds = def_kinds(&path);
    // `def_id` carries owned data (kind, row, the exact source the tree was
    // built from) out of this branch so the `app.trees` borrow behind `parsed`
    // ends here — `history_lines` below needs `&mut app`.
    let (mut lines, def_id) = if brief && kinds.is_empty() {
        return;
    } else if kinds.is_empty() {
        (
            vec!["definition lookup not supported for this file type".to_string()],
            None,
        )
    } else if let Some(def) = find_definition(root, kinds, &name, &parsed.src) {
        let mut lines = vec![format!("kind: {}", def.kind()), String::new()];
        lines.extend(signature(def, &parsed.src).lines().map(str::to_string));
        if let Some(doc) = doc_for(&path, def, &parsed.src) {
            lines.push(String::new());
            lines.extend(doc.lines().map(str::to_string));
        }
        let id = (
            def.kind().to_string(),
            def.start_position().row,
            parsed.src.clone(),
        );
        (lines, Some(id))
    } else if let Some(owner) = parameter_owner(node, kinds, &parsed.src) {
        // a parameter has no definition to find, and saying so as "not defined
        // in this file" points the reviewer at other files for no reason
        (vec![format!("parameter of {owner}")], None)
    } else if brief {
        return;
    } else {
        (vec!["not defined in this file".to_string()], None)
    };
    // history only makes sense for a symbol whose definition resolved in the
    // current file — an unresolved lookup has nothing to look up history for
    if let Some((kind, row, content)) = def_id.filter(|_| !brief) {
        lines.push(String::new());
        lines.extend(history_lines(app, &path, &name, &kind, row, &content));
    }
    app.popup = Some(Popup::new(name, lines));
}
