# Configuration — one declaration, everything else generated

Status: design. Nothing here is built yet.

## The problem, as the code has it today

Three config surfaces, three mechanisms, none of them the source of truth for the
others:

| surface | declared in | read by | generated from |
| --- | --- | --- | --- |
| `~/.config/ordo/tui.toml` (`preset`, `theme`, `[binds]`, `[theme]` roles) | `KeyConfig`, a hand-rolled line parser (`parse_key_config`) | `ordo` | `init_config`, hand-written prose + two tables (`keymap`, `THEME_ROLES`) |
| `rules.toml` / presets | `ordo::model::Rule` + `RuleToml` (real TOML since P21) | `ordo` | nothing — `docs/rules.md` is hand-kept |
| engine `Options` (`strategy`, `cross_file`, …) | `ordo::model::Options`, serde | `ordo-engine order --json`, `ordo` | `schema/v1.json`, hand-kept and frozen |

And a fourth that isn't configurable at all: the engine's tuning constants —
`LARGE_LINES = 60`, `DEEP_NESTING = 4`, `MANY_PARAMS = 6` (`extract.rs`),
`MAX_RATIONALE = 240`, `CONTAINER_BUDGET = 60` (`order.rs`), `PAIR_THRESHOLD`,
`MAX_PAIRS` (`refine.rs`).

Two consequences worth naming because they are bugs, not just untidiness:

1. **`tui.toml` as generated is not valid TOML.** `--init-config` writes `theme =
   "catppuccin-mocha"` at the top *and* a `[theme]` table below it — the same key
   as a string and as a table. The hand parser never noticed. The moment the file is
   read by a real TOML parser (as `rules.toml` now is), it fails to parse. Any
   redesign has to rename one of them.
2. **Docs and generated files drift by construction.** `docs/rules.md`'s condition
   table was hand-edited three times in one night. Nothing checks it against
   `When`.

## Goal

Every knob is declared **once**, in Rust, with its type, default and doc comment.
From that single declaration, generated and *tested against it*:

- the commented default config (`--init-config`);
- the docs page (`docs/config.md`);
- validation, including "unknown key `presett` — did you mean `preset`?";
- the engine's options schema, as a drift guard on the frozen `schema/v1.json`.

Layered sources with **provenance**: `:config` shows where every effective value came
from, the way `:rules` does for rules. The engine stays pure: it receives an
`Options`; the TUI owns files, env and flags.

## The reflection layer: `schemars`

Rust has no runtime reflection. The useful stand-in is a derive that turns a struct
into a data tree at compile time; the choice is which one.

| candidate | verdict |
| --- | --- |
| **`schemars` 1.2** (`JsonSchema` derive) | **Take it.** Doc comments become `description`, `#[serde(default)]` becomes `default` with the serialized value, `deny_unknown_fields` becomes `additionalProperties: false`, `preserve_order` keeps declaration order. serde-aligned, so every existing attribute is honoured. 436M downloads; MSRV 1.74. |
| `facet` 0.50-rc | real reflection, richer — and a release candidate with 600k downloads. Right idea, wrong year for a dependency in a tool other people install. |
| `documented` | docs only; no types, no defaults. Half the job. |
| a `config!` macro like `theme_roles!` | already the house pattern for one flat table; does not compose across nested sections or give a schema. Keep it for what it does. |

`schema_for!(TuiConfig)` yields a JSON tree: properties in declaration order, each
with `description`, `default`, `type`/`enum`. **That tree is the reflection.** One
walker over it renders TOML; another renders markdown; the deserializer already
validates against the same struct. Nothing is written twice.

In the engine crate `schemars` is **optional**, behind a `schema` feature that the
TUI and the tests enable — the default `ordo` build gains no dependency.

## The model

```rust
/// # ordo
/// Written by `ordo --init-config`; every value below is this build's default.
#[derive(Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct TuiConfig {
    /// Keystroke preset. One of the names `--keys` accepts.
    #[serde(default = "d_preset")]            // "vim"
    preset: String,
    /// Palette name. One of the names `--theme` accepts; `[colors]` overrides roles.
    #[serde(default = "d_theme")]             // "dark"
    theme: String,
    /// `"key" = "action"`; the action `none` removes a binding.
    #[serde(default)]
    #[schemars(schema_with = "binds_schema")]  // keys: any; values: enum of ACTION_NAMES
    binds: BTreeMap<String, String>,
    /// Per-role colour overrides, `#rrggbb`. Roles are the theme's fields.
    #[serde(default)]
    #[schemars(schema_with = "colors_schema")] // keys: enum of THEME_ROLES
    colors: BTreeMap<String, String>,
    /// What the engine is asked for. Same fields as `options` in `ordo-engine order --json`.
    #[serde(default)]
    engine: ordo::model::Options,
}
```

- `[theme]` (table) becomes **`[colors]`**. That is the fix for bug 1, and the only
  breaking rename. For one release the loader accepts a `[theme]` table, uses it,
  and reports `` `[theme]` is now `[colors]` `` as a problem — the same channel
  `:rules` uses.
- `[engine]` is `ordo::model::Options` **itself**, not a copy — one struct, one set of
  docs, the same names as the JSON API. `rules` is skipped in the file
  (`#[serde(skip)]` on the config side, or documented as "from rules.toml"); rules
  keep their own files and their own `include`/`disable` layering.
- The two dynamic tables (`binds`, `colors`) are the only places the generic walker
  defers to a runtime table, through `schema_with`: their *value* enumeration is
  `ACTION_NAMES` / `THEME_ROLES`, which already exist, and their default *rows* come
  from `keymap(preset)` / `Theme` — exactly what `init_config` does today, kept, just
  no longer surrounded by a hand-written template.

### Engine tuning becomes `Options`

```rust
pub struct Options {
    …existing fields…
    /// Thresholds the engine's structural notes use. Defaults are the values
    /// the corpus baseline was recorded with.
    #[serde(default)]
    pub tuning: Tuning,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Tuning {
    /// a definition longer than this gets a `large definition` note
    pub large_lines: usize,      // 60
    /// nested deeper than this many definitions gets a `deeply nested` note
    pub deep_nesting: usize,     // 4
    /// more parameters than this gets a `N params` note
    pub many_params: usize,      // 6
    /// a rationale is cut to this many characters
    pub max_rationale: usize,    // 240
}
```

Additive optional object on the frozen contract, so allowed; `schema/v1.json` gains
the entry by hand as the `frozen-contract` rule demands — and a test asserts
`schema_for!(Options)`'s property names are a subset of the frozen file's, so the two
can't drift silently. `refine.rs`'s `PAIR_THRESHOLD`/`MAX_PAIRS` stay constants: they
are algorithm internals, not review policy. Defaults equal today's constants, so the
corpus is unchanged by construction.

## Layering and provenance

```
defaults (the struct)                       ← schema_for!/Default
$XDG_CONFIG_HOME/ordo/tui.toml              ← this user
<repo>/.ordo/tui.toml                       ← this project (engine/limits mostly; keys and theme allowed)
$ORDO_TUI_KEYS, $ORDO_TUI_THEME             ← the two that exist today; no generic env scheme
--keys / --theme / --rules / --only-comments / --all    ← this run
```

Later wins, key by key. Implementation is the cheap one: parse each file to a
`toml::Table`, merge tables recursively (scalars replace, tables merge), then
deserialize the merged table **once** into `TuiConfig`. No second "all-`Option`
partial struct" to keep in step with the real one. Provenance is a side table
filled during the merge — `(key path, source)` — which is what `:config` prints:

```
preset            vim            ~/.config/ordo/tui.toml
theme             catppuccin-mocha   --theme
engine.strategy   comprehension  default
engine.tuning.many_params  3     .ordo/tui.toml
binds."C-w Left"  focus-list     default (preset vim)
```

Unknown keys are rejected by `deny_unknown_fields`; the error is rewritten with the
nearest known key at edit distance ≤ 2 from the schema's property list — ten lines,
no dependency.

## What gets generated, and what checks it

| artifact | generated by | checked by |
| --- | --- | --- |
| `--init-config` output | walk `schema_for!(TuiConfig)`: `# description` lines, then `# key = <default>` commented, `[section]` per nested object, dynamic rows for `binds`/`colors` | the existing round-trip test, generalized: uncomment everything → parse → equals `TuiConfig::default()` and reports no problems |
| `docs/config.md` | the same walk, markdown renderer: one table per section, key / default / description | a test that regenerates and diffs; `UPDATE_DOCS=1` rewrites, like the corpus baseline |
| `schema/v1.json` `options` section | **not** generated — it is the frozen contract | the subset test above |
| `docs/rules.md` condition table | same walk over `schema_for!(When)` | same diff test; the prose around it stays hand-written |

The last row is the quiet payoff: the rules reference stops being something a
person forgets to update.

## Sequence

1. **Foundation** — `schemars` (feature-gated in the engine, plain in the TUI);
   `TuiConfig` + `[colors]` rename with the `[theme]` compatibility path; the schema
   walker replacing `init_config`'s template; the round-trip test. The file people
   already have keeps working.
2. **Loading** — `toml` replaces `parse_key_config`; layering with provenance;
   `:config`; "did you mean".
3. **Engine tuning** — `Options.tuning`; constants become defaults; `schema/v1.json`
   entry; subset test; corpus run (must be identical).
4. **Docs** — `docs/config.md` and the `docs/rules.md` table generated; diff tests.
5. **Repo-level `.ordo/tui.toml`** — one more source in the same loader.

Each step ships on its own and leaves every existing file readable.

## Decisions to make before step 1

- **`[theme]` → `[colors]`**: rename now with a one-release alias (recommended — 0.x,
  and the current file is not valid TOML), or keep `[theme]` and rename the scalar
  to `palette` instead.
- **Which constants are policy**: the four above, or also `CONTAINER_BUDGET`,
  `HEADING_NAME_MAX`. Recommendation: the four; the rest are wording internals.
- **Repo-level `tui.toml`**: yes for `[engine]`, and either allow or ignore `preset`/
  `theme` there. Recommendation: allow — a repo that wants `defs-first` for reviews
  has a legitimate reason, and a key preset is harmless to override locally.
