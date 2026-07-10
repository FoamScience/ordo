//! Per-language config: tree-sitter grammar + node-type sets, ported verbatim
//! from gitplay's validated `order.lua` classifier. Adding a language is one
//! entry here plus its grammar crate in Cargo.toml — no logic changes.
//! Tier-1: python, javascript, typescript, tsx, go, c, cpp, java, lua.
use tree_sitter::Language;

pub struct LangSpec {
    pub name: &'static str,
    pub language: fn() -> Language,
    /// node types that introduce an import
    pub imports: &'static [&'static str],
    /// node types that introduce a definition (fn / type / class / …)
    pub defs: &'static [&'static str],
}

fn py() -> Language {
    tree_sitter_python::LANGUAGE.into()
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

static SPECS: &[LangSpec] = &[
    LangSpec {
        name: "python",
        language: py,
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
    },
    LangSpec {
        name: "javascript",
        language: js,
        imports: &["import_statement"],
        defs: &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "method_definition",
        ],
    },
    LangSpec {
        name: "rust",
        language: rs,
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
        ],
    },
    LangSpec {
        name: "typescript",
        language: ts,
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
        ],
    },
    LangSpec {
        name: "tsx",
        language: tsx,
        imports: &["import_statement"],
        defs: &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
            "enum_declaration",
        ],
    },
    LangSpec {
        name: "go",
        language: go,
        imports: &["import_declaration", "import_spec"],
        defs: &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
        ],
    },
    LangSpec {
        name: "c",
        language: c,
        imports: &["preproc_include"],
        defs: &[
            "function_definition",
            "struct_specifier",
            "enum_specifier",
            "union_specifier",
        ],
    },
    LangSpec {
        name: "cpp",
        language: cpp,
        imports: &["preproc_include"],
        defs: &[
            "function_definition",
            "class_specifier",
            "struct_specifier",
            "enum_specifier",
            "namespace_definition",
            "template_declaration",
        ],
    },
    LangSpec {
        name: "java",
        language: java,
        imports: &["import_declaration"],
        defs: &[
            "method_declaration",
            "constructor_declaration",
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
        ],
    },
    LangSpec {
        // lua: `require()` is a call, not a distinct import node → no imports
        name: "lua",
        language: lua,
        imports: &[],
        defs: &["function_declaration", "function_definition"],
    },
];

/// Resolve a path to a language spec by file extension, or None when
/// unsupported (caller then falls back to file order).
pub fn for_path(path: &str) -> Option<&'static LangSpec> {
    let ext = path.rsplit('.').next()?;
    let name = match ext {
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "java" => "java",
        "lua" => "lua",
        _ => return None,
    };
    SPECS.iter().find(|s| s.name == name)
}

impl LangSpec {
    pub fn is_import(&self, kind: &str) -> bool {
        self.imports.contains(&kind)
    }
    pub fn is_def(&self, kind: &str) -> bool {
        self.defs.contains(&kind)
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

pub fn is_ident(kind: &str) -> bool {
    IDENT_KINDS.contains(&kind)
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
    ) || name.ends_with(".min.js")
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
