# ordo — implementation tasklist

Change-ordering engine. Design: `gitplay.nvim/docs/ordering-engine-design.md`.
Prior art in that repo: `scratchpad/HUNK_ORDER_RESEARCH.md`, `scratchpad/hunk_order_proto.lua`.

Working name `ordo` — TBD. Rust core.

## P0 — decisions + schema freeze  ✅
- [x] Confirm name (`ordo` placeholder) — kept `ordo`
- [x] Confirm core language — Rust
- [x] Decide: engine owns diff computation, or only consumes diffs? → **owns** (computes hunks from old/new via `similar`); unified-diff *input* deferred to P3
- [x] Freeze JSON schema v1 (input + output from design §4) → `schema/v1.json`

## P1 — Rust core: parse + semantic extraction (single file)  ✅
- [x] `cargo init`, wire tree-sitter (0.25) + grammars — **python, javascript, typescript, tsx, go, c, cpp, java, lua** (all tier-1; lua via tree-sitter-lua 0.5 after bumping core to 0.25)
- [x] Line-hunks: compute via `similar` (accept unified diff = P3)
- [x] Per-hunk category: definition/import node starts in hunk → that, else "other"
- [x] Enclosing definition: innermost def node containing hunk (qualified name)
- [x] defines[] / uses[] from def/import + identifier nodes
- [x] Per-language config: node-type sets (ported from order.lua), name extraction, identifier types
- [x] Unit tests on single-file metadata (`tests/basic.rs`)

## P2 — ordering (single file)  ✅
- [x] Group hunks by enclosing def (P1)
- [x] def→use edges between groups; imports first (P2)
- [x] Topological sort (Kahn); break ties/cycles by file position (deterministic)
- [x] Strategies: comprehension | defs-first | file
- [x] Golden/property tests: permutation, def-before-use, determinism

## P3 — CLI + contract  ✅
- [x] `ordo-engine order --json` (stdin→stdout)
- [x] `ordo-engine review [patch]` subcommand (arg or stdin → JSON)
- [x] Emit versioned output (`schema` field = 1)
- [x] Unified-diff input parsing (`change.diff`) — `src/patch.rs`; full semantics for additions, positional for modified files (ceiling documented)
- [x] Golden-file snapshot harness (`tests/golden/`, `UPDATE_GOLDEN=1` to regen) — py/rust/crossfile cases

## P4 — cross-file resolution  ✅
- [x] Union symbol tables across files (name match, approximate) — global orderer
- [x] Order files+hunks so defs precede uses across changeset (global Kahn topo)
- [x] `cross_file` toggle (edges restricted to same file when off)
- [x] Cross-file fixtures (`tests/crossfile.rs`)

## P9 — diff-input full semantics  ✅ (design: `docs/diff-input-design.md`) — pure engine, no git in core

### P9.0 — prep
- [x] `cargo add diffy`
- [x] Spike: confirm `diffy::Patch::from_str` parses a headerless single-file diff (our `change.diff`); if not, prepend synthetic `--- a/<p>` / `+++ b/<p>` before parsing
- [x] `patch.rs`: helper `apply(old: &str, diff: &str) -> Option<String>` (diffy parse + apply; `None` on any parse/apply error)

### P9.1 — L1: `{path, old, diff}` → apply
- [x] `lib.rs build_change`: match arm `(Some(old), None, Some(diff))` → `patch::apply(old, diff)` → `Some(new)` ⇒ `(compute_hunks(old,&new), new)` (reuse the proven old/new path); `None` ⇒ fall through to L3
- [x] Test: `{old, diff}` output byte-equals the `{old, new}` result for the same modification
- [x] Test: corrupted/fuzzy diff (context mismatch) → graceful positional fallback, no panic

### P9.2 — L2: `{path, diff}` full-context → reconstruct both sides (OPT-IN — auto-detect is unsafe, see design)
- [x] `model.rs Options`: add `full_context: bool` (default false); `main.rs`: `ordo-engine review --full-context` flag
- [x] `patch.rs parse_file_diff(diff, full_context)`: when `full_context` AND single hunk starting at old line 1 → reconstruct `old` (ctx+removed) **and** `new` (ctx+added); else old/new = None
- [x] `ParsedFile`: carry optional `old`; `build_change`/`from_diff` use `compute_hunks(old,new)` when both present
- [x] Preserve trailing-newline / no-normalization in reconstruction
- [x] Test: full-context modified-file diff + flag → def-before-use holds (real semantics)
- [x] Test: same diff WITHOUT the flag → positional (L3); multi-hunk WITH flag → still positional

### P9.3 — L3: partial `{diff}` → positional but honest
- [x] `model.rs FileOut`: add `#[serde(default, skip_serializing_if = "is_false")] degraded: bool`
- [x] `lib.rs`: diff-only + not reconstructable ⇒ set `degraded = true` + `eprintln!` one-line warning naming the file (suggest `-U100000` / old-new API)
- [x] Thread `degraded` from `build_change` through to the emitted `FileOut`
- [x] Test: partial diff → positional order + `degraded == true` + warning on stderr

### P9.4 — CLI, contract, docs
- [x] Verify `ordo-engine review` picks all of this up (full-context patch → full semantics; else warns) — add a `review` test
- [x] `schema/v1.json`: document `{old, diff}` input combo + optional output `files[].degraded` (no `schema` bump — additive)
- [x] README: "`git diff -U100000 | ordo-engine review` for full semantics" + note the old/new API is always full
- [x] Promote the dogfood wrapper to `scripts/ordo-commit` (repo-read stays OUT of the binary)

### P9.5 — wrap
- [x] `cargo test` green (incl. new cases + regen goldens if any), `cargo fmt --check`
- [x] Re-run gitplay suite (unaffected — it uses old/new) to confirm no regression
- [x] Tick P9 items + update README ceilings section (diff modified-file no longer silently positional)

### polish (non-blocking)
- [x] rationale wording: import-category hunk that *also* defines fns now lists only import-introduced names (`HunkSem.imports`); reads "import" when the import node declares no names (e.g. go `import "fmt"`).

## P5 — distribution  ✅ (infra in place; publish needs a tag/credentials)
- [x] Prebuilt binaries per platform — `.github/workflows/release.yml` (5 targets, on `v*` tag)
- [x] npm wrapper vendoring binary + thin JS API — `packaging/npm/` (order/review tested via ORDO_BIN)
- [x] pypi wrapper vendoring binary + Python API — `packaging/pypi/` (order/review tested via ORDO_BIN)
- [x] cargo crate (`cargo install --path .`); brew formula (`packaging/brew/ordo.rb`, sha256 TBD at release)
- [x] release profile (lto/strip); CI (`.github/workflows/ci.yml`: fmt + build + test)

## P6 — gitplay integration  ✅ (already wired in gitplay; VERIFIED against this binary)
- [x] gitplay shells to `ordo` via `vim.system` — `lua/gitplay/ordo.lua` (layered `available` check, cached in `session.lua`)
- [x] Engine output drives ordering (`rank_fn` by new-range overlap); two-layer fallback → built-in `order.lua` classifier → file order
- [x] Graceful fallback when `ordo` absent (`s.ordo_ok`, `has_semantics` gate in `resolve_order_opts`)
- [x] Integration tests: `tests/suites/ordo_spec.lua` — **98/98 gitplay tests pass** with this binary present (incl. live def→use e2e + full python session)
- config: `hunk_order = "file"|"defs-first"|"comprehension"`, `ordo = { cmd = "ordo" }` (`config.lua`)
- open extension point: `cross_file` hardcoded `false` in `ordo.lua` — file-level ordering would hook at `play_commit`/`s.files` (part of P7)

## P7 — gitplay review mode  ◑ (in gitplay.nvim; no commits — working tree only)
- [x] Rationale "why" line in hunks pane — ordo `rationale` → `ordo.rationale_fn` → `planner` `hk.why` → `hunks.lua`; opt-in via `show_rationale` config (default off).
- [x] `:GitPlay review [range]`, static (non-animated) layout — `session.start_review` reuses the pipeline with `force="instant"` (no typing), forces comprehension order + `show_rationale`. Dispatch in `init.lua` + completion in `plugin/gitplay.lua`.
- [x] Cross-file def→use ordering for multi-file review — `ordo.order_files` (one `cross_file=true` call, all files; file order = first appearance in global order); `session.reorder_files_crossfile` batch-resolves blobs + stably reorders `s.files` in review mode. Tested (gitplay 105/105).
- [x] Opt-in (config-gated; review mode explicit)

**All P7 done.** gitplay.nvim changes are in the working tree, uncommitted (per no-commit rule) — files: `lua/gitplay/{ordo,session,hunks,planner,config,init}.lua`, `plugin/gitplay.lua`, `tests/suites/ordo_spec.lua`.

## P8 — broaden  ◑
- [x] Docs — `README.md` (install, CLI, contract, API, ceilings)
- [x] More languages — all tier-1 done incl. lua
- [ ] VS Code / CI / GitHub Action consumers — open-ended future broadening; build when a real consumer needs it (YAGNI until then)

## Properties to hold (test invariants)
- output is a permutation of all hunks (nothing lost)
- deterministic
- cycle-safe
- def-before-use wherever a def→use edge is derivable

## P10 — rationale augmentation (patterns #1–#7)  ✅

Foundation for all: make `rationale_for` **file-aware** — pass `paths` into
`order::order_all`, give it group→file + `gdef` so it can name provenance and
pick above/below vs "in <path>".

- [x] **#1 cross-file def→use provenance** — use side: "uses `foo`, defined in `a.py` (this change)"; def side: "defines `foo`, used in `b.py`". Drop "below" across files.
- [x] **#2 within-file bidirectional** — also speak the use side: "uses `helper` defined above" (today only the def side talks). Above/below by source position.
- [x] **#3 add vs edit a definition** — "adds `foo`" (def node starts in hunk) vs "edits `foo` body" (hunk inside an existing def). Data already present.
- [x] **#4 signature / type change** — hunk touches a def *header* or a type/struct → "changes signature of `foo` (N uses)" / "changes type `Foo`".
- [x] **#5 import add/remove** — "adds import os" / "removes `X`" (needs old-side import diff, not just new-side).
- [x] **#6 test ↔ code link** — path heuristic (tests/, _test, _spec): "tests `foo` (a.py)".
- [x] **#7 rename / deletion** — "renames `foo`→`bar`" (old↔new def matching), "removes `foo`". Highest cost, last.

## P11 — rationale polish

- [x] **collapse nested `<anonymous>`** in enclosing paths (`M.start_pick.<anonymous>.<anonymous>` → `M.start_pick.<anonymous>`) — dedupe consecutive anon in the def stack.
- [x] **rename beyond 1:1** — match removed↔added defs by normalized body (`extract::symbol_bodies`) + lone-pair 1:1 fallback for body-changed renames.
- [~] **signature-change use counts** — **skipped**: the count only sees changed hunks (0 when callers unchanged → misleading); provenance (`used by X below`) already conveys "it's used".

## P12 — competitive features  ✅ (borrow rivals' strengths into ordering+rationale)

- [ ] **#1 move detection** — a def whose (normalized) body leaves old file A and reappears in new file B → `moves foo from a.py` on B's def hunk, and `moves foo to b.py` (not "removes") on A's deletion hunk. Cross-file extension of P11.2's body matching. Rivals: difftastic/git only do single-file-pair / whole-file renames.
- [ ] **#2 noise / skippable** — formatting-only hunk (old-slice ≡ new-slice after whitespace/token normalization) and generated/lockfile paths → output `noise: true` + rationale `formatting only`. Lets consumers collapse/de-prioritize (GitHub's generated-file collapse, but per-hunk).
- [ ] **#3 PR-split suggestion** — connected components of the group def→use graph → emit independent clusters ("splits into N independent parts"). Output-only, from existing `edges`. Nobody does this deterministically.


## P13 — structural smells (native, from ordo's own AST — no style rules, no deps)

Change-shape signals as `notes`, not judgments. Language-agnostic thresholds.

- [x] **P13.1 per-hunk def smells** — a def introduced in a hunk that is large (≥60 lines), deeply nested (≥4 ancestors), or param-heavy (≥6 params) → `hunks[].notes[]` (`large definition (120 lines)`, `deeply nested (depth 4)`, `7 params`). Data: DefRec span/depth + params node count.
- [ ] **P13.2 changeset notes** — `Output.notes[]`: `code changed but no test touched` (code file changed, no test file in changeset), `path: N hunks (high churn)` (≥10 hunks). Needs `is_test_path` shared (move to lang.rs).
- [x] **P13.3 surface** — fold notes into `ordo-engine pack`; document `hunks[].notes` + `notes` in schema/v1.json + README.

## P14 — advanced-construct advisor (curated catalog, not a linter)

Detect powerful/overusable constructs, attach an escalation-ladder advisory, and
a downgrade **verdict** only where a body-inspection signal backs it.
`src/advisories.rs` — deterministic tree-sitter detection; `hunks[].advisories`.

- [x] **P14.1 framework + python metaclass** — detect `class(metaclass=)` / `class(type)`; ladder (descriptor → `__init_subclass__` → class decorator → metaclass); ⚠ verdict when a metaclass-definition overrides only `__init_subclass__`-able behavior (no `__new__`/`__prepare__`/`__call__`). Surfaced in `ordo-engine pack`, schema, README.
- [x] **catalog expansion (batch 1)** — py mutable-default-arg + bare-except (verdicts) + eval/exec; rust unsafe + transmute; js/ts eval + with (verdict); go unsafe + reflect. Tested (`p14_catalog`).
- [x] **catalog expansion (batch 2)** — path-aware advisor; +c/cpp/java coverage. py assert-validation/dynamic-type/empty-except; rust static-mut; js/ts any/empty-catch; go panic; c/cpp goto + reinterpret_cast; java empty-catch + reflection. Tested (`p14_batch2`).

## P15 — ordo reviewer (feature-gated bin, engine stays pure)  ✅
- [x] `[[bin]] ordo` behind `tui` feature (ratatui optional; default build unaffected)
- [x] git layer (shell) → `ordo::run` → ratatui review in comprehension order
- [x] reading-order list (⚠ advisories, dimmed noise) + detail pane (rationale, notes, def→use edges, advisory ladders); j/k/g/G/q nav
- [x] diff-body view (colored old→new, capped) + mark-reviewed (x, ✓, n/N progress)
- [ ] follow-ups: jump-along-edge (gd), detail-pane scroll, working-tree/range revs

## P16 — relocation / extraction detection  ✅
- [x] `symbol_bodies` also returns each def's substantial body lines
- [x] an added def whose body-lines overlap a still-present old def (≥3 shared, ≥50%) → "adds X, extracted from Y" (catches extractions where the body was also edited and the source name is reused — exact rename/move miss these). Tested (`p16_extraction_from_present_def`).

## P17 — containers: every hunk belongs to something  ✅

A hunk outside any definition used to carry the bare rationale `change`. 372 of
them across the 10-repo corpus (47k hunks); the cause was singular — nothing in
`defs` held them — so the fix was to widen what counts as a container without
widening what counts as a *definition*. `enclosing_kind` (schema, optional) says
which: a region name is never a symbol, never enters `defines`, never seeds an
edge.

- [x] **macros are definitions** — `preproc_def`/`preproc_function_def` (c/cpp);
      a macro's `value` is its body, so a body edit stops reading as a signature
      change (the same bug hit rust `const_item`/`static_item`)
- [x] **test blocks** — `describe`/`it`/`test`/`context`/`suite`/`bench` in
      js/ts/tsx and lua (busted), rust test macros (`rgtest!`); nested labels
      join with ` > `, the rationale names the innermost
- [x] **regions** — `#ifdef`/`#ifndef`/`#if`, markdown preamble and front matter
- [x] **bindings** — a file-scope binding whose multi-line value holds the hunk,
      with its literal's elements as detail-layer members
- [x] **calls** — a file-scope call whose multi-line arguments hold the hunk
- [x] **re-exports are bookkeeping** — `export * from`, `export {} `; NOT
      `export default <value>`, which fills `value` rather than `declaration`
- [x] **whitespace** — a blank-line-only hunk is formatting noise; an import
      line that moved leaving a blank behind is too
- [x] **switched-off code** — `comments out code` / `uncomments code` (exact
      match after stripping markers), `replaces N lines with a comment` when it
      is documentation arriving where code left
- [x] **removed file-scope bindings** are named, not counted as lines
- [x] language injection — a markdown fence is parsed with its own grammar, and
      contributes *uses only*: a sample documents an API, it does not define it

Result: 372 → 0 bare `change` across the corpus, with the invariants and the
git line-coverage check holding.

## P18 — reviewer polish  ✅
- [x] intra-line refinement (`ordo::refine`) — leaf-level LCS, only the part of
      a changed line that differs is tinted
- [x] `:audit` — every hunk and file not on screen, charged to what removed it
- [x] fold the reading order by group (`za`/`zo`/`zc`/`zR`/`zM`, `C-k` chords)
- [x] `Esc` clears a search before it quits; `K` finds the line's symbol and
      names the function a parameter belongs to
- [x] configurable keybind presets (`~/.config/ordo/tui.toml`) — preset choice
      plus per-key add/replace/remove, validated against the action table
- [x] themes — 14 truecolor palettes (catppuccin, tokyonight, gruvbox, nord,
      dracula, solarized) beside the two terminal-palette ones, `:theme` to swap
      live, and every role overridable in `[theme]`. Rounded pane borders; the
      whole palette lives in `Theme`, nothing hardcoded at a call site

## P19 — reviewing rules  ✅

The caller's own conventions, as data. No plugin runtime: a rule is globs plus a
tree-sitter query, matched deterministically against facts the engine already
computes, so ordering influence is safe to hand to a config file.

- [x] **facts** — `path`, `lang`, `category`, `enclosing-kind`,
      `defines`/`uses`/`imports`, `noise`, `comment`; ANDed, globs throughout
- [x] **queries** — tree-sitter source (inline or `query-file`), matched only on
      rows *inside the hunk*: a review signal, not a lint backlog
- [x] **actions** — `note`, `warn`, `noise`, `priority`
- [x] **ordering influence that cannot break P2** — priority replaces the
      file-position tiebreaker among groups the graph has already freed
- [x] **two scopes** — `~/.config/ordo/rules.toml` then `<repo>/.ordo/rules.toml`
- [x] **engine reads nothing** — rules arrive in `Options.rules`; the client
      collects the files
- [x] **a rule that cannot work says so** — bad glob, bad query, a query that
      compiles for no language in the change → `Output.problems`
- [x] ordo's own rules in `.ordo/` — contract, purity, invariants, wording

## P20 — an import is noise, not nothing  ✅

Reported from a real review: an added `from ppump.diagnostics import degrade`
was invisible, while the loguru import it replaced was reported as removed. One
asymmetry, two symptoms — a *moved* import read as a deletion with no
counterpart.

- [x] a pure-import hunk is kept and marked `noise` instead of dropped; it never
      seeds an edge, and no longer leads the reading order either (forty dimmed
      rows ahead of the change is not a reading order — `priority` in a rule can
      put them back on top for anyone who wants that)
- [x] imports group together (`same scope: imports`) rather than joining the
      top-level group, which had cost the `file` strategy its positional promise
- [x] an import statement's *bound* names, not every identifier in it:
      `from ppump.diagnostics import degrade` binds `degrade`, so add-vs-change
      is decided on the right evidence (python; other grammars already bind one
      name per statement)
- [x] a name is attributed to every row of its statement, so a hunk touching the
      tail of a multi-line import list still has names to report
- [x] `moves import pg` when the same statement existed in the old file — a
      reordered import block is not a pile of edits
- [x] `DropReason::Import` is gone: `only_comments` is now the only thing that
      drops a hunk

## P21 — rules as a table, and published guidelines as rulesets  ✅

Started from one C++ guideline set (janwilmans) and the question "what would it
take to enforce this?". The answer was mostly *engine* work: nearly every
guideline is "this hunk introduces shape X" or "…exceeds limit N", and both are
facts the engine can state once, for every language, so a rule is a table entry
rather than a tree-sitter query.

- [x] **shapes** — `kind` / `with` / `without` / `text` / `text-not`: the node
      kinds a hunk introduces, what their children must have or lack. A child is
      a node kind, a *field name* (`default_value`) or a *keyword token*
      (`virtual`) — the last is what a query anchor can never see, so absence is
      a config field now instead of `(field_declaration declarator: (_) .)`
- [x] **limits** — `max-params` / `max-lines` / `max-nesting` / `max-file-lines`,
      on facts the engine already computed for its own notes (which were `const`
      before); `max-file-lines` fires on the hunks of a file this *change*
      pushed past the limit, not every edit to a file already over it
- [x] `path-not` — third-party code, a framework carve-out
- [x] `recursive` — a definition that names itself, from the def→use graph
- [x] `container-with` / `container-without` — the members of the container a
      hunk defines into (`equals` without `hashCode`)
- [x] `member-uninitialized` — decided across the **whole change**: a member
      added in the header is fine if the `.cpp` constructor's initializer list
      names it, because if that constructor changed its file is in the diff
- [x] a C/C++ `#include` and a go `import` bind a name — `imports = "boost/**"`
      was dead for them, and the include line also claimed the row after it
- [x] C/C++ parameter counting followed the wrong field; the `N params` note had
      never fired for them
- [x] `ordo` reads rules as real TOML (arrays, multi-line queries) and
      `--rules <file>` layers a set on top of your own
- [x] **rulesets/** — ten published guideline sets as rules, each verified by a
      harness that fails on any engine problem *and* on any rule that never
      fires on its sample: C++ Default Guidelines, Uber Go, Google Python §2,
      Rust API Guidelines, Power of Ten, Effective Java, clean-code TypeScript,
      Airbnb JavaScript, Lua, markdown
- [ ] parked: `:split` — propose clusters, let the reviewer merge them, emit
      `but` commands; uncommitted work only, GitButler first
