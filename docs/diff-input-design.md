# Diff-input semantics — design

**Status:** design (approved approach: pure engine, no git in core).
**Problem:** a modified file supplied as a bare unified `diff` gets *positional*
ordering only — no tree-sitter semantics (category/enclosing/defines/uses),
because a default diff lacks the full new-file content tree-sitter needs.

## Goal / non-goals
- **Goal:** a modified file reaches *full semantics* whenever the full new
  content is derivable, without ordo shelling to git or becoming a diff tool.
- **Non-goal:** git access inside the core. Repo→blob resolution stays in
  consumers/wrappers (gitplay, `scripts/`). ordo remains pure stdin→stdout.
- **Non-goal:** trusting partial reconstruction — a syntax tree built from a
  file with missing middle chunks yields wrong node ranges; refuse instead.

## Root cause
`patch.rs::parse_file_diff` reconstructs full `new` only for *added* files
(`--- /dev/null` → all `+` lines = whole file). Modified files keep `new = None`
→ `analyze` parses `""` → every hunk `other`. This is the ceiling to lift.

## Approach — four input tiers (each derives full `new` where possible)
| Input | Handling | Result |
|---|---|---|
| `{path, old, new}` | today | full semantics |
| `{path, old, diff}` | **L1**: apply `diff` to `old` → `new`, then normal `compute_hunks(old,new)` path | full |
| `{path, diff}`, full-context | **L2**: single hunk from line 1 ⇒ reconstruct `old`=ctx+removed, `new`=ctx+added → normal path | full |
| `{path, diff}`, partial-context | **L3**: positional (status quo) **+ stderr warning + output marker** | positional, but honest |

### L1 — `old` + `diff` → apply
- Add `diffy` crate. Parse the file diff, `diffy::apply(old, &patch) → new`.
- On success: discard the diff's coarse hunks, run the proven `compute_hunks(old,new)`
  + `analyze(new)` path (identical output to the old/new tier — golden-equivalent).
- On apply error (context mismatch / fuzzy patch): fall back to L3 (positional + warn),
  never panic.

### L2 — full-context diff only (OPT-IN)
- **Why not auto-detect:** a single hunk starting at line 1 does NOT imply the
  diff spans the whole file — a `-U3` change at the *top* of a large file looks
  identical but omits the tail, so blind reconstruction silently truncates →
  wrong tree. There is no reliable pure-diff signal for "reaches EOF".
- **Opt-in instead:** the caller asserts completeness — `full_context: true`
  (JSON option) / `ordo review --full-context` — meaning "generated with
  `git diff -U100000`". We additionally require a **single hunk starting at old
  line 1** (structural check on top of the promise). Then `old` = context+removed,
  `new` = context+added → normal `compute_hunks(old,new)` path, zero git.
- Default (no flag) → positional (L3). Multi-hunk even under the flag → refuse →
  L3 (belt and suspenders).

### L3 — partial diff only (unchanged behavior, made honest)
- Keep positional hunks from the `@@` headers.
- Emit `ordo: <path>: modified-file diff lacks full context — positional order
  only (pass `git diff -U100000` or use old/new for semantics)` to **stderr**.
- Output marker: add optional `"degraded": true` on the file object so
  programmatic consumers can detect it (additive; no schema bump).

## Contract impact
- Input gains the `{old, diff}` combo and richer `{diff}` handling — **additive,
  backward-compatible**, no `schema` bump.
- Output gains optional `files[].degraded` (absent = false).

## CLI (`ordo review`)
- Benefits automatically: full-context patch → full semantics; else warns.
- Doc: `git diff -U100000 | ordo review`, or use the JSON old/new API. The
  repo-aware convenience (base blob = old, worktree = new) ships as
  `scripts/ordo-commit`, **not** in the binary.

## Tests
- L1: `{old, diff}` output equals the `{old, new}` golden for the same change.
- L2: full-context modified-file diff → def-before-use holds (real semantics).
- L3: partial diff → positional + warning emitted + `degraded` set.
- Apply failure (corrupted context) → graceful positional fallback, no panic.
- Edge cases: no-newline-at-eof, CRLF, deletion-only hunk.

## Risks
- `diffy` parsing of headerless single-file diffs — validate; prepend a synthetic
  `--- a/…`/`+++ b/…` if needed.
- Preserve trailing-newline / CRLF through reconstruction (diffy handles apply;
  L2 reconstruction must not normalize).
- Fuzzy/rejected patches → always degrade, never wrong output.
