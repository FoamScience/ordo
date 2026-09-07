# CLI & schema v1

The engine CLI (`ordo-engine`) and the frozen JSON contract every consumer
speaks. For the interactive reviewer see [tui.md](tui.md).


```sh
ordo-engine order --json < input.json > output.json
ordo-engine pack  --json < input.json                 # compact LLM-ready review context
ordo-engine review path/to.patch                      # or: git diff | ordo-engine review
git diff -U100000 | ordo-engine review --full-context  # modified files get full semantics
ordo-engine order --only-comments --json < input.json  # only comment/docstring hunks
```

Input / output are frozen as **schema v1** (`schema/v1.json`):

```jsonc
// input
{ "changes": [ { "path": "src/a.py", "old": "…", "new": "…" } ],
  "options": { "strategy": "comprehension", "cross_file": true } }
```

`strategy` is `comprehension` (default) | `defs-first` | `file`. A change may
instead carry a `diff` (unified/git) — see [Ceilings](ceilings.md).

Output carries the global `order`, per-file `hunks` (with `category`,
`enclosing`, `defines`, `uses`, `group`, `order_index`, `rationale`, `details`,
`symbols`, `noise` for skippable formatting/generated hunks, and `comment` for
comment/docstring-only hunks), the `groups`, the def→use `edges`, and the
`clusters` shown above. `ordo-engine pack` renders all of it as compact review
context.

`options.only_comments` (`--only-comments` on `ordo-engine order`/`ordo-engine pack`) drops
every non-comment hunk before ordering, so `order`/`groups`/`edges`/`clusters`
cover only comment/docstring changes — a lightweight pass over documentation
edits without the noise of the surrounding code.

Each file entry also carries `dropped`: the hunks removed before ordering
(pure imports, and non-comment hunks under `only_comments`) with the range each
covered. `hunks` + `dropped` is exactly what the diff produced, so "did ordo
miss one?" is a subtraction rather than a guess. The corpus suite checks the
stronger property against git itself: every line `git diff -U0` calls changed
must fall inside some hunk, kept or dropped.

Not every hunk sits in a definition, and the ones that don't used to say only
`change`. A hunk is now attributed to whatever really holds it, and
`enclosing_kind` says what that is when it isn't a plain definition:

<!-- ordo:begin container-kinds -->
| `enclosing_kind` | what holds the hunk | example `enclosing` |
| --- | --- | --- |
| *(omitted)* | a definition — a function, class, macro, … | `parse_cfg` |
| `test` | a named block: `describe`/`it`/`test`, or a rust test macro | `describe "cli" > it "parses flags"` |
| `region` | conditional compilation | `#ifdef CURL_DISABLE_HTTP` |
| `preamble` | prose before a document's first heading | `preamble` |
| `front-matter` | a document's `---` metadata block | `front matter` |
| `document` | one `---` document of a multi-document yaml file | `document 2` |
| `binding` | a file-scope binding whose multi-line value holds the hunk | `ALLOWED_IMPORTS` |
| `call` | a file-scope call whose multi-line arguments hold it | `execa('unicorns')` |
<!-- ordo:end container-kinds -->

Only a definition is a symbol: a region name is never looked up, never enters
`defines` or `symbols`, and never seeds a def→use edge. `#ifdef CURL_DISABLE_HTTP`
*tests* that macro rather than defining it.

`symbols` gives each definition a name + tree-sitter node `kind` + enclosing
`scope`, so a consumer can tell whether the same name across two commits is the
*same* symbol — a method `run` on class `A` and a module-level function `run`
share a name but differ in kind and/or scope, so they're different symbols.


## Library API

```js
const { order, review } = require("@ordo/cli");
const out = order({ changes: [{ path: "a.py", old, new }] });
```

```python
from ordo import order, review
out = order({"changes": [{"path": "a.py", "old": old, "new": new}]})
```

