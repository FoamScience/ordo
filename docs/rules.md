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

## Query rules

For a convention about code *shape* rather than about paths and names. The query
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
