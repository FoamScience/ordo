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
- [x] `ordo order --json` (stdin→stdout)
- [x] `ordo review [patch]` subcommand (arg or stdin → JSON)
- [x] Emit versioned output (`schema` field = 1)
- [x] Unified-diff input parsing (`change.diff`) — `src/patch.rs`; full semantics for additions, positional for modified files (ceiling documented)
- [x] Golden-file snapshot harness (`tests/golden/`, `UPDATE_GOLDEN=1` to regen) — py/rust/crossfile cases

## P4 — cross-file resolution  ✅
- [x] Union symbol tables across files (name match, approximate) — global orderer
- [x] Order files+hunks so defs precede uses across changeset (global Kahn topo)
- [x] `cross_file` toggle (edges restricted to same file when off)
- [x] Cross-file fixtures (`tests/crossfile.rs`)

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
