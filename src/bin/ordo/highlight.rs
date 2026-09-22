// ------------------------------------------------------------------- highlight
use crate::code_view::Role;
use crate::code_view::Syntax;
use ratatui::style::Color;
use std::collections::HashMap;
use tree_sitter::Parser;
use tree_sitter::Query;
use tree_sitter::QueryCursor;
use tree_sitter::StreamingIterator;
use tree_sitter_highlight::HighlightConfiguration;
use tree_sitter_highlight::HighlightEvent;
use tree_sitter_highlight::Highlighter;

// tree-sitter highlight capture names we color, with their fg. The `Highlight`
// index a walk yields is the position of the matched name in this list.
/// Highlight capture → role. The names are tree-sitter's; the roles are what a
/// theme colours. Verified against each grammar's own highlights query.
const HL: &[(&str, Role)] = &[
    ("attribute", Role::Attribute),
    ("boolean", Role::Number),
    ("comment", Role::Comment),
    ("constant", Role::Number),
    ("constant.builtin", Role::Number),
    ("constructor", Role::Type),
    ("escape", Role::Number),
    ("function", Role::Function),
    ("function.builtin", Role::Function),
    ("function.method", Role::Function),
    ("keyword", Role::Keyword),
    ("label", Role::Attribute),
    ("number", Role::Number),
    ("operator", Role::Operator),
    ("property", Role::Property),
    ("punctuation", Role::Operator),
    ("punctuation.bracket", Role::Operator),
    ("punctuation.delimiter", Role::Operator),
    ("punctuation.special", Role::Operator),
    ("string", Role::Str),
    ("string.escape", Role::Number),
    ("string.special", Role::Str),
    ("tag", Role::Attribute),
    ("text.emphasis", Role::Keyword),
    ("text.literal", Role::Str),
    ("text.reference", Role::Property),
    ("text.strong", Role::Keyword),
    ("text.title", Role::Function),
    ("text.uri", Role::Property),
    ("type", Role::Type),
    ("type.builtin", Role::Type),
    ("variable", Role::Variable),
    ("variable.builtin", Role::Builtin),
    ("variable.parameter", Role::Param),
];

pub(super) type LineSpans = Vec<(String, Color)>;
pub(super) type Highlights = HashMap<String, Vec<LineSpans>>;

/// The grammar and highlights query for a path.
///
/// The path→language question is the engine's, and is asked through
/// `ordo::lang_name_for_path` so there is one answer to it: this used to carry
/// its own copy of the extension table plus the by-filename cases, and had
/// already drifted — `.C`, `.H` (C++ by the GNU/OpenFOAM convention) and
/// `.zsh` got full engine semantics and no highlighting at all. What stays
/// here is the part that really is presentation: which query paints which
/// language. A language with no query highlights as plain text, which is what
/// a bare `.j2` did before and still does.
/// A category as the list row spells it. The same lowercase spelling the wire
/// format uses, written once rather than derived from `Debug` at each site.
pub(super) fn cat_name(c: ordo::model::Category) -> &'static str {
    match c {
        ordo::model::Category::Import => "import",
        ordo::model::Category::Definition => "definition",
        ordo::model::Category::Other => "other",
    }
}

pub(super) fn highlight_spec(path: &str) -> Option<(tree_sitter::Language, String)> {
    highlight_for_lang(ordo::lang_name_for_path(path)?)
}

/// Languages the engine reads but nothing here paints. The template grammars
/// are deliberate — a `.j2` with no host format under it renders plain, which
/// is roughly what an editor does with one. Anything else appearing in this
/// list is a gap, and `every_engine_language_is_painted_or_listed` says so.
#[cfg(test)]
pub(super) const NO_HIGHLIGHT: &[&str] = &["jinja", "erb", "gotmpl"];

pub(super) fn highlight_for_lang(lang: &str) -> Option<(tree_sitter::Language, String)> {
    let owned = |l: tree_sitter::Language, q: &str| (l, q.to_string());
    Some(match lang {
        "python" => owned(
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
        ),
        // xonsh is python plus shell syntax; python's query covers the overlap
        // and leaves the shell parts uncoloured rather than miscoloured
        "xonsh" => owned(
            tree_sitter_xonsh::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
        ),
        "javascript" => owned(
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::HIGHLIGHT_QUERY,
        ),
        "rust" => owned(
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
        "typescript" => owned(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ),
        "tsx" => owned(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ),
        "go" => owned(
            tree_sitter_go::LANGUAGE.into(),
            tree_sitter_go::HIGHLIGHTS_QUERY,
        ),
        "c" => owned(
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY,
        ),
        // cpp's query inherits C's (Neovim `; inherits: c`, which
        // tree-sitter-highlight does not resolve), so C's is prepended
        "cpp" => (
            tree_sitter_cpp::LANGUAGE.into(),
            format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            ),
        ),
        "java" => owned(
            tree_sitter_java::LANGUAGE.into(),
            tree_sitter_java::HIGHLIGHTS_QUERY,
        ),
        "lua" => owned(
            tree_sitter_lua::LANGUAGE.into(),
            tree_sitter_lua::HIGHLIGHTS_QUERY,
        ),
        "toml" => owned(
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        ),
        "json" => owned(
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY,
        ),
        "yaml" => owned(
            tree_sitter_yaml::LANGUAGE.into(),
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
        ),
        "cmake" => owned(
            tree_sitter_cmake::LANGUAGE.into(),
            tree_sitter_cmake::HIGHLIGHTS_QUERY,
        ),
        "make" => owned(
            tree_sitter_make::LANGUAGE.into(),
            tree_sitter_make::HIGHLIGHTS_QUERY,
        ),
        "bash" => owned(
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY,
        ),
        "html" => owned(
            tree_sitter_html::LANGUAGE.into(),
            tree_sitter_html::HIGHLIGHTS_QUERY,
        ),
        "svelte" => owned(
            tree_sitter_svelte_ng::LANGUAGE.into(),
            tree_sitter_svelte_ng::HIGHLIGHTS_QUERY,
        ),
        "css" => owned(
            tree_sitter_css::LANGUAGE.into(),
            tree_sitter_css::HIGHLIGHTS_QUERY,
        ),
        "nix" => owned(
            tree_sitter_nix::LANGUAGE.into(),
            tree_sitter_nix::HIGHLIGHTS_QUERY,
        ),
        "ini" => owned(
            tree_sitter_ini::LANGUAGE.into(),
            tree_sitter_ini::HIGHLIGHTS_QUERY,
        ),
        "markdown" => (tree_sitter_md::LANGUAGE.into(), md_block_query()),
        _ => return None,
    })
}

// The block query's `[(link_title)(indented_code_block)(fenced_code_block)]
// @text.literal` wraps a fenced code block's *entire* span, content included,
// starting at the exact byte where the fence-language injection's own first
// token also starts. `tree-sitter-highlight` breaks that starting-byte tie by
// opening the *deeper* (injected) scope first, which — since scopes must
// close in the order they opened — forces that first token to stay "open"
// (and coloured) all the way to wherever `text.literal` closes, i.e. the rest
// of the fence. Dropping `fenced_code_block` from that one alternation
// (leaving `link_title`/`indented_code_block` untouched) removes the base
// layer's competing scope, so the fence body carries no ambient colour and
// the injected grammar alone colours it — which is also why `@none` on
// `code_fence_content` (the block query's own attempt at this) is left out
// of `HL` entirely rather than mapped to a role: giving it a colour of its
// own would reproduce the exact same starting-byte tie against the
// injection. Falls back to the query unmodified if upstream ever reformats
// that line — a missed match just brings the wash back, it doesn't break.
fn md_block_query() -> String {
    tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.replace("\n  (fenced_code_block)\n", "\n")
}

// Fence info-string (```rust, ```py, …) → a representative extension, so a
// fenced code block's language resolves through `highlight_spec` — the same
// table a real file uses — instead of a second copy of the grammar list.
// Mirrors `lang::for_lang_name`'s canonical names. A language ordo has no
// grammar for (`console`, `json`, `diff`, …) returns None and stays plain.
fn fence_ext(name: &str) -> Option<&'static str> {
    let word = name.trim().split([' ', ',', '{', ':']).next()?.trim();
    Some(match word.to_ascii_lowercase().as_str() {
        "py" | "python" | "python3" | "pyi" => "py",
        "xsh" | "xonsh" => "xsh",
        "js" | "javascript" | "node" | "mjs" | "cjs" | "jsx" => "js",
        "ts" | "typescript" | "mts" | "cts" => "ts",
        "tsx" => "tsx",
        "rs" | "rust" => "rs",
        "go" | "golang" => "go",
        "c" => "c",
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "lua" => "lua",
        "toml" => "toml",
        _ => return None,
    })
}

// Distinct fence languages (info-string text) named by fenced code blocks in
// `src`, in first-seen order. Parsed with the block grammar directly, ahead
// of highlighting, so the injected per-fence configs can be built before the
// highlighter borrows them.
fn md_fence_languages(src: &str) -> Vec<String> {
    let language: tree_sitter::Language = tree_sitter_md::LANGUAGE.into();
    let mut parser = Parser::new();
    let Ok(()) = parser.set_language(&language) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(
        &language,
        "(fenced_code_block (info_string (language) @lang))",
    ) else {
        return Vec::new();
    };
    let mut cursor = QueryCursor::new();
    let mut names = Vec::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src.as_bytes());
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if let Ok(text) = cap.node.utf8_text(src.as_bytes()) {
                if !names.iter().any(|n| n == text) {
                    names.push(text.to_string());
                }
            }
        }
    }
    names
}

// `tree_sitter_md::INJECTION_QUERY_BLOCK` looks like the obvious choice for
// both injections below, but under `tree-sitter-highlight` neither rule works
// as shipped, so this query is hand-written instead — don't "simplify" it
// back to the crate's constant, that regresses markdown highlighting
// silently:
//
// - `(inline) @injection.content (#set! injection.language
//   "markdown_inline")` produces zero highlight events: the callback does
//   get invoked for `markdown_inline`, but the layer it returns never emits
//   anything unless the injection also carries `injection.include-children`.
// - the fenced-code rule fires and *looks* right for a one-line body, but
//   `code_fence_content` is not one opaque text node — tree-sitter-markdown's
//   scanner splits it around a `block_continuation` per line *and* around
//   stray punctuation like `(`, `{`, `=`, `;` that it tokenizes for its own
//   purposes. Without `include-children`, those child ranges are excised
//   from what the fence-language parser sees, so it's handed something like
//   "fn add \n    let x  1\n\n" instead of the real source — which silently
//   breaks the fence grammar's own parse (a `let` after that mangled prefix
//   no longer parses as a keyword). `include-children` restores the full
//   contiguous span.
const MD_INJECTION_QUERY: &str = r#"
((inline) @injection.content
 (#set! injection.language "markdown_inline")
 (#set! injection.include-children))

(fenced_code_block
  (info_string (language) @injection.language)
  (code_fence_content) @injection.content
  (#set! injection.include-children))
"#;

thread_local! {
    // Compiling a `HighlightConfiguration` (parsing its query into a
    // capture-index table) is the expensive part of highlighting, and it only
    // depends on the (language, query, injection-query) triple — not on which
    // file it's for. Cache by the query text, which is a fixed string per
    // grammar (see `highlight_spec`/`md_block_query`), so a run touching many
    // files of one language compiles that language's query once. Lives on the
    // loader worker thread that calls `highlight_file`, so a plain
    // `thread_local!` needs no locking.
    static HL_CFG_CACHE: std::cell::RefCell<HashMap<String, HighlightConfiguration>> = std::cell::RefCell::new(HashMap::new());
}

// Ensures `cache[key]` holds a `HighlightConfiguration` for `language`/`query`,
// configured with `names` exactly once. Returns whether it's present after the
// call (false only if construction failed).
type HlCfgCache = std::cell::RefCell<HashMap<String, HighlightConfiguration>>;

fn ensure_hl_cfg(
    cache: &HlCfgCache,
    key: &str,
    language: tree_sitter::Language,
    name: &str,
    query: &str,
    injections: &str,
    names: &[&str],
) -> bool {
    if cache.borrow().contains_key(key) {
        return true;
    }
    let Ok(mut cfg) = HighlightConfiguration::new(language, name, query, injections, "") else {
        return false;
    };
    cfg.configure(names);
    cache.borrow_mut().insert(key.to_string(), cfg);
    true
}

// Syntax-highlight `src` into per-line colored segments. None when the language
// is unsupported or the grammar/query fails to build → caller renders plain.
pub(super) fn highlight_file(path: &str, src: &str, syn: &Syntax) -> Option<Vec<LineSpans>> {
    let (language, query) = highlight_spec(path)?;
    let names: Vec<&str> = HL.iter().map(|(n, _)| *n).collect();
    let is_markdown = matches!(path.rsplit('.').next(), Some("md" | "markdown"));
    let injections = if is_markdown { MD_INJECTION_QUERY } else { "" };

    HL_CFG_CACHE.with(|cache| {
        if !ensure_hl_cfg(cache, &query, language, path, &query, injections, &names) {
            return None;
        }
        let injected_keys = if is_markdown {
            markdown_layers(cache, path, src, &names)
        } else {
            vec![]
        };

        let cache_ref = cache.borrow();
        let cfg = cache_ref.get(&query)?;
        let injected: Vec<(String, &HighlightConfiguration)> = injected_keys
            .iter()
            .filter_map(|(n, k)| cache_ref.get(k).map(|c| (n.clone(), c)))
            .collect();

        let mut hl = Highlighter::new();
        let events = hl
            .highlight(cfg, src.as_bytes(), None, |name| {
                injected.iter().find(|(n, _)| n == name).map(|(_, c)| *c)
            })
            .ok()?;

        spans_per_line(events, src, syn)
    })
}

/// Injected-layer configs for a markdown file: `markdown_inline` plus one
/// per fenced-code language actually present, as (layer name, query text).
/// Keyed into the same cache, by query text, so they're built at most once
/// per grammar too.
fn markdown_layers(
    cache: &HlCfgCache,
    path: &str,
    src: &str,
    names: &[&str],
) -> Vec<(String, String)> {
    let mut layers: Vec<(String, String)> = Vec::new();
    let inline_query = tree_sitter_md::HIGHLIGHT_QUERY_INLINE;
    if ensure_hl_cfg(
        cache,
        inline_query,
        tree_sitter_md::INLINE_LANGUAGE.into(),
        path,
        inline_query,
        "",
        names,
    ) {
        layers.push(("markdown_inline".to_string(), inline_query.to_string()));
    }
    for lang_name in md_fence_languages(src) {
        if layers.iter().any(|(n, _)| *n == lang_name) {
            continue;
        }
        let Some(ext) = fence_ext(&lang_name) else {
            continue;
        };
        let Some((l, q)) = highlight_spec(&format!("x.{ext}")) else {
            continue;
        };
        if ensure_hl_cfg(cache, &q, l, &lang_name, &q, "", names) {
            layers.push((lang_name, q));
        }
    }
    layers
}

/// The highlighter's event stream folded into coloured pieces per line;
/// `None` when the stream carries an error.
fn spans_per_line(
    events: impl Iterator<Item = Result<HighlightEvent, tree_sitter_highlight::Error>>,
    src: &str,
    syn: &Syntax,
) -> Option<Vec<LineSpans>> {
    let mut lines: Vec<LineSpans> = vec![vec![]];
    let mut stack: Vec<Color> = vec![];
    for ev in events {
        match ev.ok()? {
            HighlightEvent::HighlightStart(h) => {
                stack.push(HL.get(h.0).map(|(_, r)| syn.of(*r)).unwrap_or(syn.variable));
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let color = stack.last().copied().unwrap_or(syn.variable);
                push_source(&mut lines, src.get(start..end).unwrap_or(""), color);
            }
        }
    }
    Some(lines)
}

/// One highlighted run of source onto the current line — `lines` always
/// holds one — and a new line per newline in it.
fn push_source(lines: &mut Vec<LineSpans>, text: &str, color: Color) {
    for (i, piece) in text.split('\n').enumerate() {
        if i > 0 {
            lines.push(vec![]);
        }
        if !piece.is_empty() {
            let current = lines.last_mut().expect("a current line");
            current.push((piece.to_string(), color));
        }
    }
}
