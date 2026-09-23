//! Per-language config: tree-sitter grammar + node-type sets, ported verbatim
//! from gitplay's validated `order.lua` classifier. Adding a language is one
//! entry here plus its grammar crate in Cargo.toml — no logic changes (markdown
//! is the one exception: it also needed a small, `prose`-gated naming path in
//! extract.rs and order.rs, since a heading has no identifier to name a def by).
//! Tier-1: python, xonsh, javascript, typescript, tsx, go, c, cpp, java, lua,
//! markdown. Config formats (json, yaml, toml) are a third shape alongside
//! code and prose — see the `data` flag.
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tree_sitter::{Language, Parser, Tree};

/// A templating grammar: what it wraps, and how much of it to blank out so
/// the *host* format underneath can be parsed. See `extract::mask_template`.
pub struct Template {
    /// node kinds holding the host format's own bytes
    pub literal: &'static [&'static str],
    /// node kinds left in place when masking — an interpolation sits where a
    /// scalar does, and every host format here already tolerates one
    pub interpolation: &'static [&'static str],
    /// Can this grammar stand alone as the host when the wrapped format has
    /// no grammar of its own? jinja can: its blocks, macros and includes are
    /// real structure. ERB cannot — its directives are opaque ruby text, so
    /// `index.html.erb` has nothing to read and stays honestly unsupported.
    pub standalone: bool,
}

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
    /// a data/config format (json, yaml, toml): structure is keys, not code.
    /// A key is both a definition and a member of the key above it, so an
    /// edit inside a block can say which keys changed. No uses, no imports —
    /// like `prose`, this improves rationale, not ordering.
    pub data: bool,
    /// set only for a templating grammar (jinja, ERB) — see `Template`
    pub template: Option<&'static Template>,
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
fn json() -> Language {
    tree_sitter_json::LANGUAGE.into()
}
fn yaml() -> Language {
    tree_sitter_yaml::LANGUAGE.into()
}
fn toml() -> Language {
    tree_sitter_toml_ng::LANGUAGE.into()
}
fn ini() -> Language {
    tree_sitter_ini::LANGUAGE.into()
}
fn cmake() -> Language {
    tree_sitter_cmake::LANGUAGE.into()
}
fn make() -> Language {
    tree_sitter_make::LANGUAGE.into()
}
fn nix() -> Language {
    tree_sitter_nix::LANGUAGE.into()
}
fn bash() -> Language {
    tree_sitter_bash::LANGUAGE.into()
}
fn css() -> Language {
    tree_sitter_css::LANGUAGE.into()
}
fn html() -> Language {
    tree_sitter_html::LANGUAGE.into()
}
fn svelte() -> Language {
    tree_sitter_svelte_ng::LANGUAGE.into()
}
// the crate still ships pre-0.25 bindings (a `language()` fn, no `LANGUAGE`
// constant); the grammar itself loads fine against tree-sitter 0.25.
fn jinja() -> Language {
    tree_sitter_jinja::language()
}
fn erb() -> Language {
    tree_sitter_embedded_template::LANGUAGE.into()
}
// The one grammar this crate vendors rather than depends on — no crate
// publishes a Go-template grammar for a current tree-sitter. Built by
// `build.rs`; see grammars/tree-sitter-go-template/README.md.
// the symbol `parser.c` actually exports — upstream's own Rust binding still
// names `tree_sitter_go_template`, which the generated parser no longer defines
extern "C" {
    fn tree_sitter_gotmpl() -> Language;
}
fn gotmpl() -> Language {
    unsafe { tree_sitter_gotmpl() }
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
        locals: &["short_var_declaration", "var_spec", "const_spec"],
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
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
        data: false,
        template: None,
        locals: &[],
    },
    // The three config formats below share one shape: a key-value pair is
    // both the definition of its key and a member of the key above it, so
    // `edits services.web` can list `adds ports, changes image`. Naming is
    // `node_name`'s config-key path in extract.rs (the `key` field for
    // json/yaml, the first `*_key` child for toml, whose grammar labels no
    // fields). A yaml sequence item and a json array element carry no key:
    // both stay anonymous and their contents nest under the nearest named
    // key, so a list entry's position is not part of the path.
    LangSpec {
        name: "json",
        language: json,
        test_blocks: &[],
        imports: &[],
        defs: &["pair"],
        members: &["pair"],
        prose: false,
        data: true,
        template: None,
        locals: &[],
    },
    LangSpec {
        name: "yaml",
        language: yaml,
        test_blocks: &[],
        imports: &[],
        // `flow_pair` is the inline form (`{a: 1}`); anchors and aliases are
        // real def/use pairs but are not read yet (see README ceilings).
        defs: &["block_mapping_pair", "flow_pair"],
        members: &["block_mapping_pair", "flow_pair"],
        prose: false,
        data: true,
        template: None,
        locals: &[],
    },
    LangSpec {
        name: "toml",
        language: toml,
        test_blocks: &[],
        imports: &[],
        // a `[table]` header names a container the pairs beneath it belong
        // to, so it is a def in its own right alongside the pairs.
        defs: &["table", "table_array_element", "pair"],
        members: &["table", "table_array_element", "pair"],
        prose: false,
        data: true,
        template: None,
        locals: &[],
    },
    // ini and the config files shaped like it — a `[section]` header and
    // `key = value` settings, both named by `node_name`'s config-key path.
    // git's `[remote "origin"]` subsection and dvc's `['remote "x"']` are
    // section text like any other, kept verbatim (quotes and space included)
    // because that is how the file names them.
    LangSpec {
        name: "ini",
        language: ini,
        test_blocks: &[],
        imports: &[],
        defs: &["section", "setting"],
        members: &["section", "setting"],
        prose: false,
        data: true,
        template: None,
        locals: &[],
    },
    // cmake: one node kind (`normal_command`) covers every command, so which
    // command a node *is* lives in its identifier, not its kind — see
    // `extract::cmake_command`. `normal_command` is listed as a def so that
    // `set()`/`option()` can be named; every other command resolves to no name
    // and is transparent, exactly as an anonymous def already is.
    LangSpec {
        name: "cmake",
        language: cmake,
        test_blocks: &[],
        imports: &[],
        defs: &["function_def", "macro_def", "normal_command"],
        members: &[],
        prose: false,
        data: false,
        template: None,
        locals: &[],
    },
    // make: a rule is a definition named by its target, and a prerequisite is
    // a *use* of another target — the dependency graph a makefile already is,
    // read straight off the tree. Targets, prerequisites and variable names
    // are all `word` nodes, a kind far too generic for IDENT_KINDS, so uses
    // are collected from the two parents that mean one (see `extract::walk`).
    LangSpec {
        name: "make",
        language: make,
        test_blocks: &[],
        imports: &["include_directive"],
        defs: &["rule", "variable_assignment"],
        members: &[],
        prose: false,
        data: false,
        template: None,
        locals: &[],
    },
    // nix: an attribute set is the language's main structure, so a `binding`
    // is both a definition and a member of the set above it — the same shape
    // as the config formats, which is why `data` is set. A function is not a
    // separate declaration here (it is a lambda bound to an attribute), so
    // `binding` covers both. `import ./x.nix` is an ordinary application
    // whose function happens to be named `import`; see `extract::import_like`.
    LangSpec {
        name: "nix",
        language: nix,
        test_blocks: &[],
        imports: &[],
        defs: &["binding"],
        members: &["binding"],
        prose: false,
        data: true,
        template: None,
        locals: &[],
    },
    // bash: `foo() { … }` and `function foo { … }` share one node kind, and a
    // command is a call — so `deploy main` is a use of the function `deploy`.
    // `source x.sh` / `. x.sh` are commands too, named rather than spelled as
    // a distinct kind (see `extract::import_like`). A command name is a bare
    // `word`, a kind make also uses, so it is read explicitly rather than
    // through IDENT_KINDS.
    LangSpec {
        name: "bash",
        language: bash,
        test_blocks: &[],
        imports: &[],
        defs: &["function_definition"],
        members: &[],
        prose: false,
        data: false,
        template: None,
        // `local x=1` / `readonly P=8080` wrap this in a `declaration_command`
        // the walk descends through, so the one kind covers both
        locals: &["variable_assignment"],
    },
    // jinja: the host language of a template whose *underlying* format has no
    // grammar (`nginx.conf.j2`, `deploy.sh.j2`, a bare `foo.j2`). When the
    // underlying format does have one — `values.yaml.j2` — that format is the
    // host instead and the jinja statements are masked out of it; see
    // `extract::mask_template`.
    LangSpec {
        name: "jinja",
        language: jinja,
        template: Some(&Template {
            literal: &["content"],
            interpolation: &["render_expression"],
            standalone: true,
        }),
        test_blocks: &[],
        // a template's dependencies are other templates
        imports: &["include_statement", "import_statement", "extends_statement"],
        // the two *named* blocks. `{% for %}` / `{% if %}` are containers too
        // but carry no name, so they stay transparent (`walk` descends through
        // an unnamed def) rather than contributing `<anonymous>` to a path.
        defs: &["block_block", "macro_block"],
        members: &[],
        prose: false,
        data: false,
        locals: &[],
    },
    // css: a rule set is a definition named by its *whole* selector list,
    // sigil included (`.btn, .btn-primary`, `#nav a:hover`). That punctuation
    // is the safety story for the cross-file union symbol table: no code
    // grammar emits an identifier starting with `.`, `#` or `--`, so a css
    // symbol is lexically incapable of colliding with a python function.
    // A `--custom-property` and its `var(--x)` are the one honest def→use pair
    // a stylesheet has (see `extract::walk`), the way a yaml anchor is.
    LangSpec {
        name: "css",
        language: css,
        test_blocks: &[],
        imports: &["import_statement"],
        defs: &["rule_set", "keyframes_statement"],
        members: &["declaration"],
        prose: false,
        data: false,
        template: None,
        locals: &[],
    },
    // html, and with it vue. Only an element carrying an `id` is a
    // definition — that is the one name a reviewer navigates to and other
    // things reference; every other element resolves to no name and stays
    // transparent, so a page of `<div>`s contributes nothing. A class is
    // deliberately *not* a use of the css that styles it (see
    // docs/document-languages-design.md).
    //
    // A `.vue` single-file component needs no grammar of its own: this one
    // parses `<script setup lang="ts">`, `v-for`, `:key`, `@click`, `{{ }}`
    // and `<style module lang="scss">` with no error nodes, keeping the
    // script and style blocks as opaque `raw_text`. The published
    // `tree-sitter-vue` pins tree-sitter 0.20 and could not be used anyway.
    LangSpec {
        name: "html",
        language: html,
        test_blocks: &[],
        imports: &[],
        defs: &["element"],
        members: &[],
        prose: false,
        data: false,
        template: None,
        locals: &[],
    },
    // svelte: the same shape as html — `element`, `start_tag`, `attribute`
    // are the same kinds, so the id-naming path is reused verbatim — but it
    // needs its own grammar rather than riding on html's the way vue does.
    // html breaks on a bare `>` inside braces, and both `{#if n > 1}` and
    // `on:click={() => pick()}` contain one. Its own block forms (`{#if}`,
    // `{#each}`) are left unnamed for now: they are containers worth naming,
    // but `if_statement` is a kind three other grammars here also produce,
    // so claiming it would need a language-gated branch.
    LangSpec {
        name: "svelte",
        language: svelte,
        test_blocks: &[],
        imports: &[],
        // `{#snippet row(x)}` is a real named block, and `{@render row(1)}`
        // calls it — the one def→use pair a component's markup has
        defs: &["element", "snippet_statement"],
        members: &[],
        prose: false,
        data: false,
        template: None,
        locals: &[],
    },
    // Go templates, and with them Helm. One pair of delimiters does both jobs
    // — `{{ if … }}` is a statement and `{{ .Values.x }}` an interpolation —
    // so the two are told apart by node kind rather than by delimiter, which
    // is exactly what having a grammar buys. A control action *contains* the
    // text it guards, the same shape jinja has, so the masking recursion is
    // unchanged. Vendored: see grammars/tree-sitter-go-template/README.md.
    LangSpec {
        name: "gotmpl",
        language: gotmpl,
        template: Some(&Template {
            literal: &["text"],
            interpolation: &["template_action"],
            standalone: true,
        }),
        test_blocks: &[],
        imports: &[],
        // `{{ define "mychart.labels" }}` in a Helm `_helpers.tpl` is a real
        // named block, and `{{ template "x" }}` / `{{ include "x" }}` use it
        defs: &["define_action", "block_action"],
        members: &[],
        prose: false,
        data: false,
        locals: &[],
    },
    // ERB / EJS. The host format is everything outside the directives:
    // `<%= … %>` stays in place like a jinja interpolation, `<% … %>` and
    // `<%# … %>` are blanked. Its `code` is one opaque blob — ruby or
    // javascript, neither of which this crate reads — so an ERB template
    // contributes no uses, and cannot host a format with no grammar of its own.
    LangSpec {
        name: "erb",
        language: erb,
        template: Some(&Template {
            literal: &["content"],
            interpolation: &["output_directive"],
            standalone: false,
        }),
        test_blocks: &[],
        imports: &[],
        defs: &[],
        members: &[],
        prose: false,
        data: false,
        locals: &[],
    },
];

/// Extensions that mark a file as a template *over* another format, and the
/// templating grammar each one names. Strip the extension and what remains
/// names the host language.
const TEMPLATE_EXTS: &[(&str, &str)] = &[
    ("j2", "jinja"),
    ("jinja", "jinja"),
    ("jinja2", "jinja"),
    ("erb", "erb"),
    ("ejs", "erb"),
    // `.tmpl` is Go's own spelling and `.tpl` is Helm's; neither was ever
    // jinja, they were mapped there only because nothing else read them
    ("tmpl", "gotmpl"),
    ("tpl", "gotmpl"),
    ("gotmpl", "gotmpl"),
];

/// Does the final extension itself mark a template, so that stripping it names
/// the host format? True for `.j2`/`.erb`/`.tpl`; false for a Helm template,
/// whose extension is the host format's own.
fn has_template_ext(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|e| TEMPLATE_EXTS.iter().any(|(x, _)| *x == e))
}

/// The templating grammar wrapping this file — the one whose own syntax is
/// masked out, not the host format underneath it.
pub fn template_lang(path: &str) -> Option<&'static LangSpec> {
    if let Some(ext) = path.rsplit('.').next() {
        if let Some((_, name)) = TEMPLATE_EXTS.iter().find(|(e, _)| *e == ext) {
            return SPECS.iter().find(|s| s.name == *name);
        }
    }
    // Helm is the exception to the whole extension convention: a chart's
    // templates carry no template extension at all — `templates/deployment.yaml`
    // is yaml with Go template actions written through it. Detected by the
    // directory Helm requires them to live in, which is a heuristic and is
    // meant to be a loose one: masking a file that turns out to hold no
    // template syntax blanks nothing and changes nothing, so a false positive
    // on some other project's `templates/` directory costs exactly zero.
    if is_helm_template(path) {
        return SPECS.iter().find(|s| s.name == "gotmpl");
    }
    None
}

fn is_helm_template(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("");
    matches!(ext, "yaml" | "yml")
        && (path.starts_with("templates/") || path.contains("/templates/"))
}

/// The spec a *template* is parsed with: the underlying format when it has a
/// grammar (`values.yaml.j2` → yaml), jinja itself otherwise (`foo.j2`,
/// `nginx.conf.j2`). Only ever called for a path `has_template_ext` accepts.
fn template_spec(path: &str) -> Option<&'static LangSpec> {
    let inner = path.rsplit_once('.').map(|(head, _)| head)?;
    // a second template extension (`a.j2.j2`) is not stripped again: one
    // level is what the convention means, and looping invites a path that is
    // nothing but extensions.
    if let Some(host) = for_path_plain(inner) {
        return Some(host);
    }
    // nothing underneath: the templating grammar hosts the file if it can
    // stand alone, otherwise the file is honestly unsupported
    template_lang(path).filter(|t| t.template.is_some_and(|t| t.standalone))
}

/// Resolve a path to a language spec by file extension, or None when
/// unsupported (caller then falls back to file order). A template extension
/// (`.j2` and friends) resolves to the format underneath it.
pub fn for_path(path: &str) -> Option<&'static LangSpec> {
    // only an extension that *marks* a template is stripped; a Helm template's
    // extension is the host format's own and stays
    if has_template_ext(path) {
        return template_spec(path);
    }
    for_path_plain(path)
}

/// Suffixes that mark a file as a *variant* of another: `.dvc/config.local`
/// overrides `.dvc/config` and is the same format. Stripped before resolving,
/// the way a template extension is.
const VARIANT_EXTS: &[&str] = &["local"];

/// Config files named by filename rather than extension — a gitconfig has no
/// extension at all, and `.dvc/config` shares its basename with half the files
/// on a disk, so that one is matched by the directory it sits in.
fn for_filename(path: &str, name: &str) -> Option<&'static LangSpec> {
    // cmake's entry point has a `.txt` extension that says nothing about it
    if name == "CMakeLists.txt" {
        return SPECS.iter().find(|s| s.name == "cmake");
    }
    // shell config and dotenv files carry no extension
    if matches!(name, ".bashrc" | ".bash_profile" | ".profile" | ".env") {
        return SPECS.iter().find(|s| s.name == "bash");
    }
    // a makefile is named, not extended
    if matches!(
        name,
        "Makefile" | "makefile" | "GNUmakefile" | "Makefile.am" | "Makefile.in"
    ) {
        return SPECS.iter().find(|s| s.name == "make");
    }
    let is_ini = matches!(
        name,
        ".gitconfig"
            | ".gitmodules"
            | ".editorconfig"
            | ".npmrc"
            | ".hgrc"
            | ".flake8"
            | ".pylintrc"
            | ".coveragerc"
    ) || path.ends_with(".git/config")
        || path.ends_with(".dvc/config");
    is_ini.then(|| SPECS.iter().find(|s| s.name == "ini"))?
}

fn for_path_plain(path: &str) -> Option<&'static LangSpec> {
    // resolve against the basename: a dotted *directory* (`.dvc/config`,
    // `.ssh/config`) otherwise hands the extension split a whole path segment
    let name = path.rsplit('/').next().unwrap_or(path);
    let (path, name) = match name.rsplit_once('.') {
        Some((head, v)) if !head.is_empty() && VARIANT_EXTS.contains(&v) => (
            path.strip_suffix(v).unwrap_or(path).trim_end_matches('.'),
            head,
        ),
        _ => (path, name),
    };
    if let Some(spec) = for_filename(path, name) {
        return Some(spec);
    }
    let ext = name.rsplit('.').next()?;
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
        "json" => "json",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "ini" | "cfg" => "ini",
        "cmake" => "cmake",
        "mk" | "mak" | "make" => "make",
        "nix" => "nix",
        "sh" | "bash" | "zsh" => "bash",
        "css" => "css",
        "html" | "htm" | "vue" => "html",
        "svelte" => "svelte",
        _ => return None,
    };
    SPECS.iter().find(|s| s.name == name)
}

/// Every registered language, in registry order — the docs generator's view
/// of this table (see `languages()` in the crate root).
pub fn all() -> &'static [LangSpec] {
    SPECS
}

/// Resolve a markdown fence's info string (```python, ```rs, ```C++) to a
/// spec — the language *injected* into a prose file. Only names this crate has
/// a grammar for resolve; a `console` or `diff` fence has no structure to read
/// and returns None rather than being guessed at.
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
        "json" => "json",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        // `conf` only as a fence: an author who writes ```conf means an
        // ini-shaped file, while a `.conf` *path* is as often nginx or systemd,
        // and resolving it would also steal `nginx.conf.j2` from jinja
        "ini" | "cfg" | "conf" | "dosini" => "ini",
        "cmake" => "cmake",
        "make" | "makefile" | "mk" => "make",
        "nix" => "nix",
        // not `console`: that fence is a shell *session* (`$ cmd` and its
        // output), not a script — see tests/injection.rs
        "sh" | "bash" | "shell" | "zsh" => "bash",
        // Anything else is tried as a file extension. The two tables used to be
        // written out separately and had drifted: `conf` and `zsh` resolved
        // from a fence but not from a path, and `html`, `css`, `vue` and
        // `svelte` the other way round, each grammar present the whole time.
        _ => return for_path_plain(&format!("x.{lower}")),
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
    // cmake `${SOURCES}` — the only grammar here with a bare `variable` kind
    "variable",
    // bash `$APP_DIR` and the left of an assignment
    "variable_name",
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

/// Is this identifier the member half of an access — `element.angle`, `p->len`?
/// Still a use (the rationale says what the hunk touched), but never evidence
/// that the *definition* lives in another changed file: the name belongs to
/// whatever object it hangs off, and a same-named export elsewhere is a
/// coincidence. The shorthand kinds stay out: `{angle}` really does reference
/// the variable `angle`.
pub fn is_member_ident(kind: &str) -> bool {
    matches!(kind, "property_identifier" | "field_identifier")
}

/// Does this path look like a test file? (tests/ dir, test_*, *_test, *_spec)
pub fn is_test_path(p: &str) -> bool {
    // slash-wrapped, so `d` matches inside the path and `d[1..]` at its start
    const TEST_DIRS: &[&str] = &["/tests/", "/test/"];
    const NAME_PARTS: &[&str] = &["_test.", "_spec.", ".test.", ".spec."];
    let name = p.rsplit('/').next().unwrap_or(p);
    TEST_DIRS
        .iter()
        .any(|d| p.contains(d) || p.starts_with(&d[1..]))
        || name.starts_with("test_")
        || NAME_PARTS.iter().any(|s| name.contains(s))
}

/// Generated / vendored / lockfile paths whose hunks are noise to a reviewer.
pub fn is_generated_path(p: &str) -> bool {
    const LOCKFILES: &[&str] = &[
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "Cargo.lock",
        "go.sum",
        "poetry.lock",
        "Gemfile.lock",
        "composer.lock",
        "flake.lock",
        "uv.lock",
    ];
    const SUFFIXES: &[&str] = &[
        "_generated.go",
        ".min.js",
        ".min.css",
        ".map",
        ".pb.go",
        "_pb2.py",
    ];
    const GENERATED_DIRS: &[&str] = &["/generated/", "/vendor/", "/node_modules/"];
    let name = p.rsplit('/').next().unwrap_or(p);
    LOCKFILES.contains(&name)
        || name.starts_with("_generated.")
        || name.contains(".generated.")
        || SUFFIXES.iter().any(|s| name.ends_with(s))
        || GENERATED_DIRS.iter().any(|d| p.contains(d))
}

/// What a parameter contributes to a definition's arity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// must be passed
    Required,
    /// carries a default, so it may be omitted
    Optional,
    /// `*args`, `**kw`, `...rest` — makes the upper bound unbounded, and is
    /// why a definition holding one is not arity-checked at all
    Variadic,
}

/// Classify one parameter node. A positive list per non-required kind, so an
/// unfamiliar kind reads as `Required` — which is the *conservative* direction
/// only for the lower bound, and is why `signatures` refuses any definition
/// carrying a kind it does not recognise as variadic.
pub fn param_kind(kind: &str) -> ParamKind {
    match kind {
        "list_splat_pattern"
        | "dictionary_splat_pattern"
        | "variadic_parameter"
        | "rest_pattern"
        | "rest_parameter"
        | "spread_parameter" => ParamKind::Variadic,
        "default_parameter"
        | "typed_default_parameter"
        | "assignment_pattern"
        | "optional_parameter"
        | "optional_typed_parameter" => ParamKind::Optional,
        _ => ParamKind::Required,
    }
}

/// Does a definition of this kind *have* a signature — is it callable, with a
/// parameter list a change can alter? `changes signature of X` is the wording
/// for those; everything else that already existed and was touched on its own
/// declaration line simply `changes`. A cmake `set()`, a make variable, a yaml
/// key and a rust `const` are all values, not calls, and a value has no
/// signature to change.
///
/// A positive list rather than an exclusion: a kind this does not name gets
/// the weaker, always-true wording, so a language added later reads acceptably
/// before anyone thinks about it.
pub fn has_signature(kind: &str) -> bool {
    matches!(
        kind,
        // python, c, cpp, lua, bash all spell it this way; nix has no such kind
        "function_definition"
            | "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
            | "method_signature"
            | "method_declaration"
            | "constructor_declaration"
            | "function_item"
            | "macro_definition"
            | "preproc_function_def"
            // cmake `function(f a b)` / `macro(m a)`; its `normal_command`
            // (a `set()`) is deliberately absent
            | "function_def"
            | "macro_def"
            // a jinja macro takes parameters; a `{% block %}` does not
            | "macro_block"
    )
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

/// Memoized (language, content) -> Tree cache.
///
/// `run` makes several passes over *every* changed file — hunk semantics,
/// symbol facts, change detection, the rules engine — and each one re-parses
/// the same two strings (the old and new side). The passes are the outer loop
/// and the files the inner one, so a file's tree is wanted again only after
/// every other file has been through.
///
/// This used to be four slots that cleared themselves when full, on the
/// assumption that the access pattern was "same string many times in a row,
/// then move on". Measured on an 8-file C changeset it was not: 16 distinct
/// contents were parsed 56 times, every repeat a full re-parse.
///
/// So: least-recently-used, bounded by the source bytes held rather than an
/// entry count, so one enormous file cannot evict everything else on its own.
/// `Tree::clone` is a refcount bump (`ts_tree_copy`), not a deep copy.
///
/// The budget counts source because that is what a caller can reason about;
/// the trees measured ~10x it (230 KiB of C held across an 8-file changeset
/// moved peak RSS 12.5 -> 14.9 MB). 4 MiB therefore trades roughly 40 MB of
/// peak memory for not re-parsing, and holds a 500-file review whole.
const TREE_CACHE_BYTES: usize = 4 * 1024 * 1024;

struct CacheEntry {
    lang: &'static str,
    hash: u64,
    len: usize,
    tree: Tree,
}

thread_local! {
    static PARSER: RefCell<(Parser, &'static str)> = RefCell::new((Parser::new(), ""));
    /// how many real parses have happened — a cache that silently stops
    /// caching is otherwise invisible, since every call still returns a
    /// correct tree. Read by `tree_cache_tests`.
    #[cfg(test)]
    static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TREE_CACHE: RefCell<Vec<CacheEntry>> = const { RefCell::new(Vec::new()) };
}

/// Drop every cached tree.
///
/// The cache exists to serve the passes of one `run`; past that it is just
/// retained memory, and the `ordo` client calls `run` again on every reload.
/// `run` clears it on the way out so a finished review does not hold 4 MiB of
/// source worth of trees for the life of the process.
pub(crate) fn forget_trees() {
    TREE_CACHE.with(|c| c.borrow_mut().clear());
}

/// Parse `content` with this spec's grammar. `None` when the grammar refuses
/// to load or the parse fails — callers degrade rather than abort.
pub(crate) fn parse(spec: &LangSpec, content: &str) -> Option<Tree> {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let hash = hasher.finish();

    // A hit moves the entry to the back, so the front is always the least
    // recently used. Hash + length is the collision guard: a 64-bit hash match
    // alone is already astronomically unlikely to be wrong, and pairing it with
    // the length costs nothing extra to check.
    let cached = TREE_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let at = c
            .iter()
            .position(|e| e.lang == spec.name && e.hash == hash && e.len == content.len())?;
        let entry = c.remove(at);
        let tree = entry.tree.clone();
        c.push(entry);
        Some(tree)
    });
    if let Some(tree) = cached {
        return Some(tree);
    }

    #[cfg(test)]
    PARSE_COUNT.with(|c| c.set(c.get() + 1));
    let tree = PARSER.with(|p| {
        let mut p = p.borrow_mut();
        let (parser, last_lang) = &mut *p;
        if *last_lang != spec.name {
            parser.set_language(&(spec.language)()).ok()?;
            *last_lang = spec.name;
        }
        parser.parse(content, None)
    })?;

    TREE_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.push(CacheEntry {
            lang: spec.name,
            hash,
            len: content.len(),
            tree: tree.clone(),
        });
        // drop from the front (least recently used) until the budget holds.
        // The just-parsed entry always stays, even if it alone exceeds the
        // budget: evicting it would guarantee a re-parse on the next pass,
        // which is the behaviour this cache exists to prevent.
        let mut held: usize = c.iter().map(|e| e.len).sum();
        while held > TREE_CACHE_BYTES && c.len() > 1 {
            held -= c.remove(0).len;
        }
    });

    Some(tree)
}

#[cfg(test)]
mod tree_cache_tests {
    use super::*;

    /// A cache that stops caching is invisible: every call still returns a
    /// correct tree, only slower. These count parses directly so the failure
    /// mode this cache exists to prevent is observable.
    fn parses(spec: &LangSpec, contents: &[&str]) -> usize {
        forget_trees();
        let before = PARSE_COUNT.with(|c| c.get());
        for c in contents {
            assert!(parse(spec, c).is_some(), "{c} parses");
        }
        PARSE_COUNT.with(|c| c.get()) - before
    }

    fn c_spec() -> &'static LangSpec {
        all().iter().find(|s| s.name == "c").expect("c grammar")
    }

    #[test]
    fn the_same_content_is_parsed_once_however_often_it_is_asked_for() {
        let spec = c_spec();
        let src = "int main(void) { return 0; }\n";
        assert_eq!(parses(spec, &[src, src, src, src, src]), 1);
    }

    /// The access pattern `run` actually has: several passes over every file,
    /// so a file's tree is wanted again only after every other file. This is
    /// what the old four-slot clear-on-full cache got wrong.
    #[test]
    fn a_pass_over_many_files_does_not_evict_the_file_it_started_with() {
        let spec = c_spec();
        let files: Vec<String> = (0..12)
            .map(|i| format!("int f{i}(void) {{ return {i}; }}\n"))
            .collect();
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        // three passes over all twelve, the shape `run` has
        let mut seq: Vec<&str> = vec![];
        for _ in 0..3 {
            seq.extend(&refs);
        }
        assert_eq!(parses(spec, &seq), files.len());
    }

    #[test]
    fn the_budget_evicts_the_least_recently_used_and_keeps_the_newest() {
        let spec = c_spec();
        // each body is over half the budget, so only one fits beside another
        let big: Vec<String> = (0..3)
            .map(|i| {
                let pad = "// pad\n".repeat(TREE_CACHE_BYTES / 2 / 7);
                format!("{pad}int g{i}(void) {{ return {i}; }}\n")
            })
            .collect();
        forget_trees();
        for b in &big {
            assert!(parse(spec, b).is_some());
        }
        let held: usize = TREE_CACHE.with(|c| c.borrow().iter().map(|e| e.len).sum());
        assert!(held <= TREE_CACHE_BYTES, "budget held: {held}");
        // the most recent survives; the oldest was evicted
        let newest = TREE_CACHE.with(|c| c.borrow().last().map(|e| e.len));
        assert_eq!(newest, Some(big[2].len()));
        forget_trees();
    }

    /// One file larger than the whole budget still caches, or the passes over
    /// it re-parse it every time — the worst case, not a degenerate one.
    #[test]
    fn a_single_file_over_budget_is_still_cached() {
        let spec = c_spec();
        let huge = format!(
            "{}int h(void) {{ return 0; }}\n",
            "// pad\n".repeat(TREE_CACHE_BYTES / 7 + 16)
        );
        assert!(huge.len() > TREE_CACHE_BYTES);
        assert_eq!(parses(spec, &[&huge, &huge, &huge]), 1);
        forget_trees();
    }

    #[test]
    fn forgetting_drops_everything() {
        let spec = c_spec();
        let src = "int k(void) { return 1; }\n";
        assert_eq!(parses(spec, &[src, src]), 1);
        forget_trees();
        assert_eq!(TREE_CACHE.with(|c| c.borrow().len()), 0);
        // and the next ask really re-parses
        assert_eq!(parses(spec, &[src]), 1);
    }
}
