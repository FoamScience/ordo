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
client (`ordo`) collects the files and passes them in. That keeps
`ordo-engine order --json` a function of its arguments.

## Presets: opting in, overriding, disabling

Nothing is on by default. The rulesets under `rulesets/` are bundled into
`ordo`, and a file opts in by name:

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

## Shipped rulesets

Published guideline sets, as rules, under [`../rulesets/`](../rulesets/) — each
one verified against a sample in which every rule fires. They are bundled into
the `ordo` binary and **off by default**; `include = ["<name>"]` opts in.

<!-- ordo:begin rulesets -->
| preset | source |
| --- | --- |
| `c-power-of-ten` | NASA/JPL "The Power of 10: Rules for Developing Safety-Critical Code" |
| `cpp-default-guidelines` | Jan Wilmans' C++ Default Guidelines — https://github.com/janwilmans/guidelines |
| `go-uber-guide` | Uber Go Style Guide — https://github.com/uber-go/guide (style.md) |
| `java-effective-java` | Effective Java (Joshua Bloch), 3rd edition — the construct-level items |
| `javascript-airbnb` | airbnb/javascript — https://github.com/airbnb/javascript |
| `lua-style-guide` | Lua style — Olivine Labs (https://github.com/Olivine-Labs/lua-style-guide) |
| `markdown` | Markdown — two rules pulled from markdownlint's list |
| `python-google-style` | Google Python Style Guide — https://google.github.io/styleguide/pyguide.html |
| `rust-api-guidelines` | Rust API Guidelines checklist — https://rust-lang.github.io/api-guidelines/checklist.html |
| `typescript-clean-code` | labs42io/clean-code-typescript — https://github.com/labs42io/clean-code-typescript |
<!-- ordo:end rulesets -->

Each file's header says what it deliberately leaves out — style that belongs to
a formatter, lints a linter already owns, and anything needing dataflow.

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

<!-- ordo:begin rule-conditions -->
| key | matches |
| --- | --- |
| `path` | glob against the file path |
| `path-not` | glob the file path must *not* match — third-party code, a framework carve-out |
| `lang` | `python`, `cpp`, `markdown`, … as `src/lang.rs` names them |
| `category` | `import` · `definition` · `other` |
| `enclosing-kind` | what holds the hunk — see the table in [cli.md](cli.md) |
| `defines` | glob against any name the hunk defines |
| `uses` | glob against any name the hunk uses |
| `imports` | glob against any name the hunk imports |
| `noise-when` | the engine's own noise classification (`true` / `false`) |
| `comment` | the hunk is comment/docstring-only |
| `query` | a tree-sitter query, inline (below) |
| `query-file` | a tree-sitter query, read from a file relative to the rules file |
| `kind` | a node kind the hunk introduces (below) |
| `with` | …whose direct children include each of these |
| `without` | …and none of these — absence, as a table entry |
| `text` | …and whose text matches this regex |
| `text-not` | …and whose text does not match this regex |
| `max-params` | a definition the hunk introduces takes more parameters (below) |
| `max-lines` | …is longer than this |
| `max-nesting` | …sits deeper in control flow than this |
| `max-file-lines` | this change pushed the file past this many lines |
| `recursive` | a definition starting in the hunk calls itself |
| `container-with` | glob against the members of the container the hunk defines into (below) |
| `container-without` | …the same, negated |
| `member-uninitialized` | the hunk adds a data member nothing in this change initializes |
<!-- ordo:end rule-conditions -->

### Actions

<!-- ordo:begin rule-actions -->
| key | does |
| --- | --- |
| `name` | how the rule identifies itself in the review — required |
| `note` | says something on the hunk |
| `warn` | says it at warning level — `⚠` in the reading order |
| `noise` | marks the hunk skippable, like generated code |
| `priority` | sorts it earlier (see the guarantee below) |
<!-- ordo:end rule-actions -->

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
printed by `ordo`), never silently dropped: a rule that never fires looks
exactly like a convention nobody breaks.
