//! Per-language config: tree-sitter grammar + node-type sets, ported verbatim
//! from gitplay's validated `order.lua` classifier. Adding a language is one
//! entry here plus its grammar crate in Cargo.toml — no logic changes (markdown
//! is the one exception: it also needed a small, `prose`-gated naming path in
//! extract.rs and order.rs, since a heading has no identifier to name a def by).
//! Tier-1: python, xonsh, javascript, typescript, tsx, go, c, cpp, java, lua,
//! markdown.
use tree_sitter::{Language, Parser, Tree};

pub struct LangSpec {
    pub name: &'static str,
    pub language: fn() -> Language,
    /// node types that introduce an import
    pub imports: &'static [&'static str],
    /// call names that introduce a *named block* rather than a definition —
    /// `describe("…", () => …)` and friends. A test suite is the container a
    /// reviewer navigates by, but it is a call, not a declaration, so it is
    /// tracked separately from `defs` (see `extract::test_block_label`).
    /// Matched on the callee's first segment, so `test.serial` and `it.only`
    /// count too. Empty for languages with no such convention.
    pub test_blocks: &'static [&'static str],
    /// node types that introduce a definition (fn / type / class / …)
    pub defs: &'static [&'static str],
    /// node types that are a *named member* of an enclosing definition — an enum
    /// variant, a struct field, an object property. Drives the detail layer
    /// (P15): what a hunk added to / removed from / changed in its container.
    pub members: &'static [&'static str],
    /// prose, not code: a def is a section, not a function/class. Switches the
    /// rationale wording (adds/edits/removes X) to say "section X" and lets a
    /// nested def's enclosing scope resolve to its parent rather than itself.
    pub prose: bool,
    /// node types that bind a name without being a definition (local
    /// variable / assignment target) — drives the "adds local X, used at …"
    /// rationale wording. Verified against each grammar's node-types.json.
    pub locals: &'static [&'static str],
}

fn py() -> Language {
    tree_sitter_python::LANGUAGE.into()
}
fn xonsh() -> Language {
    tree_sitter_xonsh::LANGUAGE.into()
}
fn js() -> Language {
    tree_sitter_javascript::LANGUAGE.into()
}
fn rs() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}
fn ts() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
fn tsx() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}
fn go() -> Language {
    tree_sitter_go::LANGUAGE.into()
}
fn c() -> Language {
    tree_sitter_c::LANGUAGE.into()
}
fn cpp() -> Language {
    tree_sitter_cpp::LANGUAGE.into()
}
fn java() -> Language {
    tree_sitter_java::LANGUAGE.into()
}
fn lua() -> Language {
    tree_sitter_lua::LANGUAGE.into()
}
// block grammar only (headings, sections, prose) — the crate's separate
// inline grammar (link/emphasis text) is out of scope, see README.
fn md() -> Language {
    tree_sitter_md::LANGUAGE.into()
}

static SPECS: &[LangSpec] = &[
    LangSpec {
        name: "python",
        language: py,
        test_blocks: &[],
        imports: &[
            "import_statement",
            "import_from_statement",
            "future_import_statement",
        ],
        defs: &[
            "function_definition",
            "class_definition",
            "decorated_definition",
        ],
        members: &["pair", "keyword_argument"],
        prose: false,
        locals: &["assignment"],
    },
    // a python superset: every node kind python's entry names exists in this
    // grammar too (checked against its node-types.json), plus the shell forms.
    LangSpec {
        name: "xonsh",
        language: xonsh,
        test_blocks: &[],
        imports: &[
            "import_statement",
            "import_from_statement",
            "future_import_statement",
        ],
        defs: &[
            "function_definition",
            "class_definition",
            "decorated_definition",
        ],
        members: &["pair", "keyword_argument"],
        prose: false,
        // `env_assignment` is xonsh's own: `$FOO = …` binds a name no python
        // `assignment` node covers
        locals: &["assignment", "env_assignment"],
    },
    LangSpec {
        name: "javascript",
        language: js,
        test_blocks: &["describe", "it", "test", "context", "suite", "bench"],
        imports: &["import_statement"],
        defs: &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "method_definition",
            // named arrow/function expressions (`const foo = () => {}`,
            // `const foo = function () {}`) — anonymous when unbound: a def
            // node whose `node_name` returns None (no assignment/pair/…
            // ancestor within 3 levels) is transparent in extract.rs's
            // `walk`, so an inline callback (`arr.map(x => x*2)`) contributes
            // no def entry and floods nothing. Verified against
            // tree-sitter-javascript-0.23.1's node-types.json.
            "arrow_function",
            "function_expression",
            "generator_function",
        ],
        members: &["pair", "field_definition", "method_definition"],
        prose: false,
        locals: &["variable_declarator"],
    },
    LangSpec {
        name: "rust",
        language: rs,
        test_blocks: &["test"],
        imports: &["use_declaration", "extern_crate_declaration"],
        defs: &[
            "function_item",
            "struct_item",
            "enum_item",
            "trait_item",
            "impl_item",
            "mod_item",
            "type_item",
            "macro_definition",
            "const_item",
            "static_item",
        ],
        members: &["enum_variant", "field_declaration"],
        prose: false,
        locals: &["let_declaration"],
    },
    LangSpec {
        name: "typescript",
        language: ts,
        test_blocks: &["describe", "it", "test", "context", "suite", "bench"],
        imports: &["import_statement"],
        defs: &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "abstract_class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
            "enum_declaration",
            // see javascript's comment: same grammar shapes, verified
            // against tree-sitter-typescript-0.23.2's node-types.json.
            "arrow_function",
            "function_expression",
            "generator_function",
        ],
        members: &[
            "enum_assignment",
            "property_signature",
            "public_field_definition",
            "method_signature",
            "pair",
        ],
        prose: false,
        locals: &["variable_declarator"],
    },
    LangSpec {
        name: "tsx",
        language: tsx,
        test_blocks: &["describe", "it", "test", "context", "suite", "bench"],
        imports: &["import_statement"],
        defs: &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
            "enum_declaration",
            // see javascript's comment: same grammar shapes, verified
            // against tree-sitter-typescript-0.23.2's node-types.json.
            "arrow_function",
            "function_expression",
            "generator_function",
        ],
        members: &[
            "enum_assignment",
            "property_signature",
            "public_field_definition",
            "method_signature",
            "pair",
        ],
        prose: false,
        locals: &["variable_declarator"],
    },
    LangSpec {
        name: "go",
        language: go,
        test_blocks: &[],
        imports: &["import_declaration", "import_spec"],
        defs: &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
        ],
        members: &["field_declaration", "const_spec", "var_spec", "type_spec"],
        prose: false,
        locals: &["short_var_declaration", "var_spec"],
    },
    LangSpec {
        name: "c",
        language: c,
        test_blocks: &[],
        imports: &["preproc_include"],
        defs: &[
            "function_definition",
            "struct_specifier",
            "enum_specifier",
            "union_specifier",
            // a macro is a definition: `#define CPOOL_LOCK(...)` names something
            // the rest of the file uses, so a hunk inside its body belongs to it
            // rather than to the file at large. Both kinds carry a `name` field
            // (verified against tree-sitter-{c,cpp}-0.23.4's node-types.json).
            "preproc_def",
            "preproc_function_def",
        ],
        members: &["field_declaration", "enumerator"],
        prose: false,
        locals: &["declaration"],
    },
    LangSpec {
        name: "cpp",
        language: cpp,
        test_blocks: &[],
        imports: &["preproc_include"],
        defs: &[
            "function_definition",
            "class_specifier",
            "struct_specifier",
            "enum_specifier",
            "namespace_definition",
            "template_declaration",
            // a macro is a definition: `#define CPOOL_LOCK(...)` names something
            // the rest of the file uses, so a hunk inside its body belongs to it
            // rather than to the file at large. Both kinds carry a `name` field
            // (verified against tree-sitter-{c,cpp}-0.23.4's node-types.json).
            "preproc_def",
            "preproc_function_def",
        ],
        members: &["field_declaration", "enumerator"],
        prose: false,
        locals: &["declaration"],
    },
    LangSpec {
        name: "java",
        language: java,
        test_blocks: &[],
        imports: &["import_declaration"],
        defs: &[
            "method_declaration",
            "constructor_declaration",
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "field_declaration",
        ],
        // field_declaration is a def (below), not a member: a java field is
        // commonly referenced by bare name within the class (a static final
        // constant, or an instance field read without `this.`), so it needs
        // def→use edges the way a rust const/static does. Leaving it in
        // `members` too made P15 report "adds/changes X in X" for a hunk
        // that IS the field's own def — self-referential once the field
        // becomes its own enclosing definition.
        members: &["enum_constant"],
        prose: false,
        locals: &["local_variable_declaration"],
    },
    LangSpec {
        // lua: `require()` is a call, not a distinct import node → no imports
        name: "lua",
        language: lua,
        test_blocks: &["describe", "it", "test", "context", "pending"],
        imports: &[],
        defs: &["function_declaration", "function_definition"],
        members: &["field"],
        prose: false,
        // `local x = …` parses as `variable_declaration` wrapping an
        // `assignment_statement`/`variable_list` — the name sits several
        // levels down (see extract.rs's lua-specific binding walk), not
        // reachable via a single field the way other grammars' locals are.
        locals: &["variable_declaration", "assignment_statement"],
    },
    LangSpec {
        // markdown: a def is a `section` (heading + its content, nested by
        // heading level via the grammar's own tree shape) — no imports, no
        // uses (link targets are out of scope, see README). `section` doing
        // double duty as both def and member lets a new subsection register
        // as a member of its parent for the P15 detail layer.
        name: "markdown",
        language: md,
        test_blocks: &[],
        imports: &[],
        defs: &["section"],
        members: &["section"],
        prose: true,
        locals: &[],
    },
];

/// Resolve a path to a language spec by file extension, or None when
/// unsupported (caller then falls back to file order).
pub fn for_path(path: &str) -> Option<&'static LangSpec> {
    let ext = path.rsplit('.').next()?;
    let name = match ext {
        "py" | "pyi" => "python",
        "xsh" | "xonsh" | "xonshrc" => "xonsh",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "c" | "h" => "c",
        // `.C`/`.H` are C++ by the GNU convention gcc follows, and the house
        // style of OpenFOAM and much older scientific C++. Case matters here:
        // `.c` is C, `.C` is not, so the extension is never lower-cased.
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "C" | "H" => "cpp",
        "java" => "java",
        "lua" => "lua",
        "md" | "markdown" => "markdown",
        _ => return None,
    };
    SPECS.iter().find(|s| s.name == name)
}

/// Resolve a markdown fence's info string (```python, ```rs, ```C++) to a
/// spec — the language *injected* into a prose file. Only names this crate has
/// a grammar for resolve; a `console`, `json` or `diff` fence has no structure
/// to read and returns None rather than being guessed at.
pub fn for_lang_name(name: &str) -> Option<&'static LangSpec> {
    // an info string may carry attributes after the language (```py title=x)
    let word = name.trim().split([' ', ',', '{', ':']).next()?.trim();
    let lower = word.to_ascii_lowercase();
    let canonical = match lower.as_str() {
        "py" | "python" | "python3" => "python",
        "xsh" | "xonsh" => "xonsh",
        "js" | "javascript" | "node" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "typescript" => "typescript",
        "tsx" => "tsx",
        "rs" | "rust" => "rust",
        "go" | "golang" => "go",
        "c" => "c",
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "lua" => "lua",
        "md" | "markdown" => "markdown",
        _ => return None,
    };
    SPECS.iter().find(|s| s.name == canonical)
}

impl LangSpec {
    pub fn is_import(&self, kind: &str) -> bool {
        self.imports.contains(&kind)
    }
    pub fn is_def(&self, kind: &str) -> bool {
        self.defs.contains(&kind)
    }
    pub fn is_member(&self, kind: &str) -> bool {
        self.members.contains(&kind)
    }
    pub fn is_local(&self, kind: &str) -> bool {
        self.locals.contains(&kind)
    }
}

/// Identifier node kinds treated as symbol references (uses) and as declaration
/// names. Broad on purpose — resolution is approximate per design non-goals.
pub const IDENT_KINDS: &[&str] = &[
    "identifier",
    "type_identifier",
    "field_identifier",
    "property_identifier",
    "shorthand_property_identifier",
    "shorthand_property_identifier_pattern",
    "constant",
];

/// How a qualified enclosing name joins its parts. Code nests through a dot
/// (`App.handle`, the language's own notation); prose nests through an arrow
/// (`Install > From source`), since a dot reads like a code path and a heading
/// path is not one.
pub fn scope_sep(spec: &LangSpec) -> &'static str {
    if spec.prose {
        " > "
    } else {
        "."
    }
}

pub fn is_ident(kind: &str) -> bool {
    IDENT_KINDS.contains(&kind)
}

/// Does this path look like a test file? (tests/ dir, test_*, *_test, *_spec)
pub fn is_test_path(p: &str) -> bool {
    let name = p.rsplit('/').next().unwrap_or(p);
    p.contains("/tests/")
        || p.starts_with("tests/")
        || p.contains("/test/")
        || p.starts_with("test/")
        || name.starts_with("test_")
        || name.contains("_test.")
        || name.contains("_spec.")
        || name.contains(".test.")
        || name.contains(".spec.")
}

/// Generated / vendored / lockfile paths whose hunks are noise to a reviewer.
pub fn is_generated_path(p: &str) -> bool {
    let name = p.rsplit('/').next().unwrap_or(p);
    matches!(
        name,
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "Cargo.lock"
            | "go.sum"
            | "poetry.lock"
            | "Gemfile.lock"
            | "composer.lock"
            | "flake.lock"
            | "uv.lock"
    ) || name.starts_with("_generated.")
        || name.contains(".generated.")
        || name.ends_with("_generated.go")
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".map")
        || name.ends_with(".pb.go")
        || name.ends_with("_pb2.py")
        || p.contains("/generated/")
        || p.contains("/vendor/")
        || p.contains("/node_modules/")
}

/// Is this def node a *type* (class/struct/enum/interface/…) rather than a
/// function? Node kinds are distinctive enough to judge language-agnostically.
/// Used to word signature vs type changes (#4).
pub fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "type_declaration"
            | "struct_specifier"
            | "enum_specifier"
            | "union_specifier"
            | "class_specifier"
    )
}

/// Parse `content` with this spec's grammar. `None` when the grammar refuses
/// to load or the parse fails — callers degrade rather than abort.
pub(crate) fn parse(spec: &LangSpec, content: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&(spec.language)()).ok()?;
    parser.parse(content, None)
}
