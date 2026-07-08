# ordo

A standalone, language-agnostic engine that reorders the **hunks** of a code
change into a **comprehension-optimized** reading order, with per-hunk semantic
metadata and a human rationale. One core, many consumers (editors, CLIs, CI,
review bots) — all speaking one JSON contract.

Grounded in Baum et al. (ICSME 2017): **P1** keep related changes together,
**P2** definitions before their uses, **P3** grouping is the dominant win.

Ordering is a comprehension *aid*, not a correctness fix — treat it as such.

## What it does

Per file, it parses the *new* content once with tree-sitter and, for each hunk:
classifies it (`import` / `definition` / `other`), finds its enclosing
definition, and extracts the symbols it **defines** and **uses**. Then it groups
hunks by enclosing definition (P1), derives **def→use** edges between groups
(P2, across files when enabled), and topologically sorts them — ties and cycles
broken by file position, so output is deterministic and a strict permutation of
the input (nothing is dropped).

## Install

```sh
cargo install --path .          # from source (Rust)
npm  install -g @ordo/cli       # node wrapper (vendors a prebuilt binary)
pip  install ordo               # python wrapper (vendors a prebuilt binary)
brew install elwardi/tap/ordo   # macOS
```

The npm/pypi packages are thin wrappers around one prebuilt binary (the
ruff/esbuild pattern). Set `ORDO_BIN=/path/to/ordo` to point them at a local
build.

## CLI

```sh
ordo order --json < input.json > output.json
ordo review path/to.patch          # or: git diff | ordo review
```

Input / output are frozen as **schema v1** (`schema/v1.json`):

```jsonc
// input
{ "changes": [ { "path": "src/a.py", "old": "…", "new": "…" } ],
  "options": { "strategy": "comprehension", "cross_file": true } }
```

`strategy` is `comprehension` (default) | `defs-first` | `file`.
A change may instead carry a `diff` (unified/git). See the ceiling below.

Output carries the global `order`, per-file `hunks` (with `category`,
`enclosing`, `defines`, `uses`, `group`, `order_index`, `rationale`), the
`groups`, and the def→use `edges`.

## Library API

```js
const { order, review } = require("@ordo/cli");
const out = order({ changes: [{ path: "a.py", old, new }] });
```

```python
from ordo import order, review
out = order({"changes": [{"path": "a.py", "old": old, "new": new}]})
```

## Supported languages

python, javascript, typescript, tsx, go, c, cpp, java, lua. Adding one is a
single registry entry in `src/lang.rs` plus its grammar crate — no algorithm
changes.

## Ceilings (by design)

- **Symbol resolution is approximate** — name match with an optional cross-file
  union, no full scope/type analysis. `cross_file` is a toggle.
- **`diff` input**: a context-limited patch has no full new-file content, so
  *modified* files yield positional hunks; additions reconstruct fully. For full
  semantics on modified files, pass `old`/`new`.

## Development

```sh
cargo test                     # unit + property + golden + cross-file
UPDATE_GOLDEN=1 cargo test     # regenerate tests/golden/*/expected.json
```

Design: [`docs/`](docs) / the ordering-engine design outline. Roadmap and status
in [`TASKS.md`](TASKS.md).
