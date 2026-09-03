# Reviewing rules

Your own conventions, applied to the change you are reviewing.

A rule is **data**, not a plugin: globs and a tree-sitter query, matched against
facts the engine already computes. Nothing is executed, so the same input still
produces the same output — which is what lets rules influence the *order*
without anyone having to trust them.

## Where rules live

| file | for |
| --- | --- |
| `${XDG_CONFIG_HOME:-~/.config}/ordo/rules.toml` | your own preferences, everywhere |
| `<repo>/.ordo/rules.toml` | this project's conventions, checked in |

Both apply. Yours are read first, the repo's second, so on a hunk they both
match the project's rule reads last. A repo file being checked in also means CI
and a reviewer see the same rules.

The **engine reads neither**. `ordo::run` takes rules in `Options.rules`; a
client (`ordo-tui`) collects the files and passes them in. That keeps
`ordo order --json` a function of its arguments.

## Presets: opting in, overriding, disabling

Nothing is on by default. The rulesets under `rulesets/` are bundled into
`ordo-tui`, and a file opts in by name:

```toml
# <repo>/.ordo/rules.toml
include = ["go-uber-guide", "./team.toml"]   # a bundled preset, or a path relative to this file
disable = ["raw-loop", "*-size"]             # by name; globs allowed

[[rule]]                    # same name as an included rule → replaces it, in place
name = "three-arguments"
lang = "go"
max-params = 4
note = "more than 4 arguments"

[[rule]]                    # a new name → extends
name = "no-cgo"
lang = "go"
imports = "C"
warn = "cgo needs a design review"
```

Three things decide what runs:

1. **Definitions layer in order** — each file's `include`s first, then its own
   rules; your file, then the repo's, then `--rules`. A rule whose `name` already
   exists replaces the earlier one, so "make it a note", "raise the limit",
   "narrow it with `path-not`" are all the same move: define it again.
2. **Disables win, whoever wrote them.** `disable` lists from every file are
   applied after everything is layered, so you can silence a rule the repo
   includes, and the repo can silence one you include.
3. **A name defined twice in one file is a problem**, not a silent last-wins.

`--rules <preset-or-file>` layers one more source last. `:rules` in the TUI shows
where the active rules came from, what was replaced, what was disabled — a
silenced rule looks exactly like a convention nobody breaks, so the silencing is
never silent.

## A rule

```toml
[[rule]]
name = "security-first"          # how it identifies itself in the review
path = "src/security/**"         # a condition
note = "security-sensitive path" # what to say
priority = 100                   # and where to put it
```

### Conditions

Every condition given must hold. A rule with no conditions matches every hunk —
occasionally what you want, otherwise a mistake its name should make obvious.

| key | matches |
| --- | --- |
| `path` | glob against the file path |
| `lang` | `python`, `cpp`, `markdown`, … as `src/lang.rs` names them |
| `category` | `import` · `definition` · `other` |
| `enclosing-kind` | `definition` · `test` · `region` · `binding` · `call` · `preamble` · `front-matter` |
| `defines` / `uses` / `imports` | glob against any name the hunk defines, uses or imports |
| `noise-when` / `comment` | the engine's own classification |
| `query` / `query-file` | a tree-sitter query (below) |
| `kind` / `with` / `without` / `text` / `text-not` | a node shape the hunk introduces (below) |
| `path-not` | glob the file path must *not* match — third-party code, a framework carve-out |
| `max-params` / `max-lines` / `max-nesting` / `max-file-lines` | a limit something the hunk introduces exceeds (below) |
| `recursive` | a definition starting in the hunk calls itself |
| `container-with` / `container-without` | glob against the members of the container the hunk defines into (below) |
| `member-uninitialized` | the hunk adds a data member nothing in the change initializes |

### Actions

| key | does |
| --- | --- |
| `note` | says something on the hunk |
| `warn` | says it at warning level — `⚠` in the reading order |
| `noise` | marks the hunk skippable, like generated code |
| `priority` | sorts it earlier (see the guarantee below) |

## Ordering influence, and its limit

`priority` replaces the *file-position tiebreaker*, and nothing else. Groups are
still ordered by the def→use graph first; priority only chooses among the groups
the graph has already freed.

So this reorders two independent files:

```toml
[[rule]]
name = "security-first"
path = "src/security/**"
priority = 100
```

…and this one does **not** get what it asks for, if `main.py` uses a helper that
`util.py` defines:

```toml
[[rule]]
name = "main-first"
path = "main.py"
priority = 1000     # util.py still comes first
```

A preference cannot pull a use ahead of its definition. That is P2, and it is
not negotiable by config — there is a test named after it.

## Shape rules

Most conventions are "this change introduces X" — a `typedef`, a `protected:`, a
data member with no initializer. That is a table entry, not a query:

```toml
[[rule]]
name = "no-typedef"
lang = "cpp"
kind = "type_definition"
note = "prefer `using X = Y`"

[[rule]]
name = "uninitialized-member"
lang = "cpp"
kind = "field_declaration"
without = "default_value"
warn = "initialize at declaration"

[[rule]]
name = "virtual-destructor"
lang = "cpp"
kind = "declaration"
with = "function_declarator"
without = "virtual"
text = "~"
warn = "a destructor in a hierarchy is virtual"
```

- `kind` — the node kind(s) the hunk introduces; a string or a list.
- `with` / `without` — what the node's direct children must include / must lack.
  Each entry is a node kind (`init_declarator`), a **field name** (`default_value`)
  or a **keyword token** (`virtual`, `static`, `override`). The last one matters:
  keywords are anonymous in the syntax tree and a query anchor cannot see them.
- `text` / `text-not` — a regex the node's own text must match / must not match.

`without` is how you say *absence*. `(field_declaration declarator: (field_identifier) .)`
expresses the same thing as a query, but nobody should have to know that.

## Limits

```toml
[[rule]]
name = "small-functions"
max-lines = 42
max-params = 3
note = "split it, or pass a struct"

[[rule]]
name = "flat-control-flow"
max-nesting = 2
warn = "return early"

[[rule]]
name = "file-size"
max-file-lines = 2000
note = "this change pushed the file past 2000 lines"
```

`max-lines` and `max-params` measure a definition the hunk *starts*; `max-nesting` is
the deepest `if`/`for`/`while`/`match`/`try` any row of the hunk sits inside.
`max-file-lines` fires on the hunks of a file that *crossed* the limit in this change —
not on every edit to a file that was already over it.

## Relationships

```toml
[[rule]]
name = "equals-needs-hashcode"
lang = "java"
defines = "equals"
container-without = "hashCode"
warn = "override hashCode with equals"

[[rule]]
name = "no-recursion"
recursive = true
warn = "no direct recursion (Power of Ten, rule 1)"
```

`container-with` / `container-without` look at the members — methods, fields,
variants — of the container the hunk's definition lives in. `recursive` is a
definition that names itself in its own body.

```toml
[[rule]]
name = "initialize-members"
member-uninitialized = true
warn = "no constructor in this change initializes this member"
```

`member-uninitialized` is decided across the **whole change**, not one hunk: a member
added in a C++ header is fine if a constructor's initializer list in the `.cpp` — or an
in-class initializer, or a Java `this.x = …` — names it, and if that constructor was
updated, its file is in the diff. A member the old side already had is not this
change's to answer for. C++ and Java only; C structs have no constructors to
initialize in. The engine also records each such member as a `notes` entry
(`uninitialized member m_x`), rule or no rule.

## Query rules

For a convention about code *shape* rather than about paths and names, when a
`kind` rule can't say it — a relationship *between* nodes. The query
is tree-sitter's own syntax, and only rows **inside the hunk** count:

```toml
[[rule]]
name = "prefer-pathlib"
lang = "python"
query-file = "rules/prefer-pathlib.scm"
warn = "prefer pathlib.Path over os.path.* in new code"
```

```scheme
; .ordo/rules/prefer-pathlib.scm
((call
   function: (attribute
     object: (attribute object: (identifier) @mod attribute: (identifier) @sub)
     attribute: (identifier) @fn)) @call
 (#eq? @mod "os")
 (#eq? @sub "path"))
```

**This is the difference between a review signal and a linter backlog.** ruff's
`flake8-use-pathlib` flags all 400 pre-existing `os.path.join` calls in a legacy
codebase, which is why people disable it. ordo flags the one *this change
introduces*, on the hunk you are reading, in the reading order. A rule whose
pattern is already in the file, untouched, does not fire.

### Two things to know

**Predicates go inside the pattern.** A sibling `(#eq? @mod "os")` parses fine
and silently matches everything:

```scheme
; wrong — matches every call
(call function: (attribute object: (identifier) @mod)) @call
(#eq? @mod "os")

; right
((call function: (attribute object: (identifier) @mod)) @call
 (#eq? @mod "os"))
```

**Queries can't resolve names.** `os.path.join(a, b)` matches the query above;
`from os.path import join` followed by a bare `join(a, b)` does not, because a
query sees an identifier and cannot know where it came from. Pair it with an
`imports` condition, which the engine *does* know:

```toml
[[rule]]
name = "prefer-pathlib-aliased"
lang = "python"
imports = "join"
warn = "os.path.join imported directly — prefer pathlib.Path"
```

## When a rule can't work

A glob or query that doesn't compile is reported in `Output.problems` (and
printed by `ordo-tui`), never silently dropped: a rule that never fires looks
exactly like a convention nobody breaks.
