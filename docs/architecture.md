# Architecture

Where the seams are, for someone changing the code. The other docs describe
what ordo does; this one says where each thing lives and what it may depend on.

The one rule that shapes everything else: **the engine is a function of its
arguments.** `ordo::run` takes an `Input` and returns an `Output`. It shells to
nothing, reads no files, and knows nothing about git. Everything that touches a
repository, a config file or a terminal lives in the client.

## The seams

| tier | section | lives in |
| --- | --- | --- |
| **A — engine** | A1 pipeline core | `src/lib.rs` |
| | A2 syntax layer | `src/lang.rs`, `src/extract.rs`, `grammars/` |
| | A3 narration layer | `src/order.rs`, `src/advisories.rs`, `src/contract.rs`, `rulesets/catalog/` |
| **B — contract** | B1 contract + input | `src/model.rs`, `schema/v2.json`, `src/main.rs`, `src/patch.rs` |
| | B2 rules + rulesets | `src/rules.rs`, `src/catalog.rs`, `rulesets/`, `scripts/ruleset-check.py` |
| **C — client** | C1 data layer | `src/bin/ordo/main.rs` (args, load), `src/bin/ordo/findings.rs`, `src/bin/ordo/git.rs`, `src/bin/ordo/consumers.rs`, `src/bin/ordo/marks.rs` |
| | C2 UI layer | `src/bin/ordo/main.rs` (app state, event loop), `src/bin/ordo/keys.rs`, `src/bin/ordo/rules.rs`, `src/bin/ordo/config.rs`, `src/bin/ordo/highlight.rs`, `src/bin/ordo/code_view.rs`, `src/bin/ordo/history.rs`, `src/bin/ordo/search.rs`, `src/bin/ordo/editor.rs`, `src/bin/ordo/draw.rs`, `src/bin/ordo/commands.rs`, plus `src/refine.rs` |
| **D — supporting** | D1 test + corpus | `tests/`, `benches/`, `corpus/`, `.github/workflows/ci.yml` |
| | D2 docs system | `docs/`, the generated blocks in `src/bin/ordo/docs.rs` |

Out of scope: `target/`, `packaging/`, `assets/`.

The client is one binary crate under `src/bin/ordo/`: `src/bin/ordo/main.rs` holds the
arguments, the loader, the app state and the event loop, and each `mod` file
is one concern with a `// ---- name` banner at its top. The module files are
the addressable unit — an earlier version of this map cited line ranges in a
single 15k-line file and they were stale within a fortnight.

## What depends on what

```
main.rs ─────────────────┐
                         ├──> lib.rs ──> extract.rs ──> lang.rs ──> grammars/
bin/ordo.rs (client) ────┘       │                          ▲
  git, config, terminal          ├──> order.rs              │
                                 ├──> rules.rs ─────────────┘
                                 │      ▲
                                 │      └── catalog.rs ──> catalog.generated.json
                                 └──> advisories.rs
```

Nothing in tier A may reach for tier C. The client is a consumer of the engine
like any other, which is what keeps `ordo-engine order --json` honest.

## One fact, many copies

The recurring defect behind most of the review pass was the same shape: a fact
the code already knows once, written down again somewhere else, and the two
drifting apart. Every drift found was between two copies of one thing.

The surviving copies, and what holds them together:

| the fact | copies | what catches drift |
| --- | --- | --- |
| the rule schema | `model::When` (26 fields), `RuleToml` in the client, `schema/v2.json` (25 declared + the `unknown` catch-all) | `tests/schema.rs`, `rule_toml_keys`, `every_documented_rule_condition_is_one_the_engine_reads` |
| the extension table | `src/lang.rs`, the client's highlighter | nothing yet |
| the construct catalog | `rulesets/catalog/*.toml` → `src/catalog.generated.json` | `the_compiled_catalog_matches_the_toml_it_is_built_from`, `tests/catalog.rs` |

Two that used to be on this list are gone:

- **rationale prose** was written in both `src/lib.rs` and `src/order.rs`; it now lives
  only in `src/order.rs` (`src/lib.rs` builds no phrases at all).
- **construct detection** was two mechanisms — a hardcoded walker per construct
  beside rules-as-data. 34 of them are data now; the walkers that remain in
  `src/advisories.rs` are the ones the rule language genuinely cannot state,
  and its module doc lists each one with the reason.

`docs/rules.md`'s rule tables are no longer a copy either: they are generated
from `RULE_DOC` and drift-tested, which is the pattern to reach for when a
fourth copy of something is tempting.

## Where to put a new fact

- **A shape the engine detects** → a rule in `rulesets/catalog/`, not a walker.
  Only write Rust when the rule language cannot express it, and say why in the
  `src/advisories.rs` module doc.
- **Something the caller configures** → `model::Options` or a `[[rule]]` key,
  which means touching all three schema copies above; the drift tests will tell
  you if you miss one.
- **Something derived from a repository** → the client. If the engine would
  have to shell out to know it, it belongs on the other side of the seam.
- **A number in a doc** → generate it. `UPDATE_DOCS=1 cargo test --bin ordo
  docs::` rewrites every generated block.
