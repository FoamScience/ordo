<p align="center">
  <img src="assets/readme/hero.svg" width="100%" alt="ordo — a code change, read in the order a human should read it. Right: a reviewer pane from a real ordo run listing five hunks in comprehension order, the changed definition first, with the rationale “changes signature of edit_files”.">
</p>

<p align="center">
  <a href="https://github.com/FoamScience/ordo/actions/workflows/ci.yml"><img src="https://github.com/FoamScience/ordo/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
  <img src="https://img.shields.io/badge/schema-v1_frozen-5fd4c0" alt="schema v1, frozen">
  <!-- ordo:begin langs-badge -->
  <img src="https://img.shields.io/badge/languages-26-5fd4c0" alt="26 supported languages">
  <!-- ordo:end langs-badge -->

**Diffs arrive in file order. Nobody reads them that way.**

`ordo` is a standalone, language-agnostic engine that reorders the **hunks** of
a code change into a **comprehension-optimized** reading order, with per-hunk
semantic metadata and a human-readable rationale. One core, many consumers —
editors, CLIs, CI, review bots — all speaking one frozen JSON contract.

Grounded in Baum, Schneider & Bacchelli (ICSME 2017): **P1** keep related
changes together, **P2** definitions before their uses, **P3** grouping is the
dominant win. Ordering is a comprehension *aid*, not a correctness fix — treat
it as such.

## What you get

- **A reading order**, not a file listing — a deterministic permutation of the
  change's hunks, definitions ahead of their uses.
- **A rationale per hunk** — one plain sentence saying why it is where it is
  and what it did: `adds parse_cfg, used in main.py`, `changes signature of
  parse`, `renames foo → bar`, `removes 31 lines`.
- **A detail layer** — what the hunk did to its container's members: which enum
  variant, struct field, object property or call keyword actually moved.
- **Clusters** — the change's independent parts. One cluster means the change
  is atomic; several is a candidate PR split.
- **Advisories** — curated, deterministic guidance on advanced constructs
  (metaclasses, `unsafe`, raw `new`/`delete`, shell injection), with an
  escalation ladder and a verdict only when the pattern is concretely wrong.
- **Noise flags** — generated, lock and pure-formatting hunks marked so a
  client can dim or drop them.
- **Full accounting** — every hunk the diff produced is either in the reading
  order or recorded in `dropped` with the reason, so a consumer can prove
  nothing went missing silently.
- **Your own rules** — conventions as data (path/name facts plus tree-sitter
  queries) that annotate, flag, or move a hunk earlier in the reading order,
  without ever overriding definitions-before-uses.
- **Intra-line refinement** — when a removed and an added line are the same line
  edited, only the part that changed is highlighted, over grammar leaves rather
  than characters: adding a parameter reads as adding that parameter.
- **A reviewer TUI** — `ordo`, a first-party client that shells to git and
  renders the whole thing in the terminal.

## Why the order is principled

<p align="center">
  <img src="assets/readme/defuse.svg" width="100%" alt="Real ordo output on a two-file change: main.py uses helper(), util.py defines it. Diffed order lists main.py first; ordo's comprehension order puts util.py's definition first, linked to its use by a def-to-use edge.">
</p>

Per file, `ordo` parses both sides of the diff with tree-sitter and, for each
hunk: classifies it (`import` / `definition` / `other`), finds its enclosing
definition, and extracts the symbols it **defines** and **uses** (imported
names and local bindings are neither, so they can't seed spurious edges). Pure
import hunks are marked `noise` and grouped together, the rest are grouped by
enclosing definition (**P1**),
linked by **def→use** edges between groups (**P2**, across files when
`cross_file` is on), and topologically sorted — ties and cycles broken by file
position, so output is deterministic and a permutation of the non-import
hunks.


## Install

```sh
cargo install --path .          # from source: `ordo` (the reviewer) and `ordo-engine` (the JSON CLI)
npm  install -g @ordo/cli       # node wrapper (vendors a prebuilt binary)
pip  install ordo               # python wrapper (vendors a prebuilt binary)
```

The npm/pypi packages are thin wrappers around one prebuilt binary (the
ruff/esbuild pattern). Set `ORDO_BIN=/path/to/ordo-engine` to point them at a local
build.


## Quickstart

```sh
ordo HEAD                                   # review the last commit, in the terminal
ordo main...feature 'src/*'                 # a branch, Rust sources only
git diff | ordo-engine review               # the JSON contract, from a patch
ordo-engine pack --json < input.json        # compact LLM-ready review context
```

`ordo help` lists the documentation topics and `ordo help <topic>` prints one —
the pages below, embedded in the binary, so they always match the version you
are running.

`ordo` is the interactive reviewer; `ordo-engine` is the pure JSON CLI every
other consumer speaks to. Neither is required by the other.

## Documentation

| | |
| --- | --- |
| [`docs/cli.md`](docs/cli.md) | `ordo-engine`, schema v1, the `enclosing_kind` table, the library API |
| [`docs/tui.md`](docs/tui.md) | the reviewer: revisions, filters, keybindings, command bar, themes |
| [`docs/reviewing.md`](docs/reviewing.md) | the detail layer, rationale patterns, advisories, the change ledger |
| [`docs/rules.md`](docs/rules.md) | conventions as data — conditions, limits, queries, shipped rulesets |
| [`docs/languages.md`](docs/languages.md) | every supported language and the shape it is read in |
| [`docs/ceilings.md`](docs/ceilings.md) | what `ordo` deliberately does not do |

Design notes — how a decision was reached rather than what it is — also live in
[`docs/`](docs), suffixed `-design.md`.

## Ceilings, in brief

`ordo` states its own limits rather than guessing past them: symbol resolution
is name-matching, not type analysis; a context-limited `diff` of a modified
file stays positional and is flagged `degraded`; a file with no tree-sitter
grammar is flagged `unsupported`; the engine has no filtering policy of its
own. The full list is [`docs/ceilings.md`](docs/ceilings.md).

## Development

```sh
cargo test                     # unit + property + golden + cross-file
UPDATE_GOLDEN=1 cargo test     # regenerate tests/golden/*/expected.json
UPDATE_DOCS=1   cargo test     # regenerate the marked blocks in README/docs
```

Blocks fenced by a pair of `ordo:begin <key>` / `ordo:end <key>` HTML
comments are generated from the code that owns them — the language registry, the command
table, the keymaps, the theme list, the bundled rulesets. A test fails when a
committed block no longer matches, so the docs cannot quietly go stale.

License: MIT.
