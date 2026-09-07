# Detail layer, rationale patterns & advisories


<details>
<summary><b>Detail layer</b> — what a hunk did to its container's members</summary>

Where the rationale names the *container* a hunk touched, `details` says what
it did to that container's members — an enum variant, a struct field, an
object property.
Members on both sides are compared by their own text, not just their name, so
a member that merely shares a line with the real change is not reported as
changed. A container the hunk introduces wholesale stays silent — the
rationale already says `adds type Fresh`. Lists cap at three names plus a
count.

```
rationale: edits main
details:
  - adds help to ap.add_argument("--sample")
  - removes required from ap.add_argument("--sample")
  - adds default, help, and nargs to ap.add_argument("--samples")
```

Two `add_argument(...)` calls in the same function are attributed separately,
never conflated under the enclosing `main`.

<!-- ordo:begin members -->
| language | member node kinds |
| --- | --- |
| python / xonsh | `pair`, `keyword_argument` |
| javascript | `pair`, `field_definition`, `method_definition` |
| rust | `enum_variant`, `field_declaration` |
| typescript / tsx | `enum_assignment`, `property_signature`, `public_field_definition`, `method_signature`, `pair` |
| go | `field_declaration`, `const_spec`, `var_spec`, `type_spec` |
| c / cpp | `field_declaration`, `enumerator` |
| java | `enum_constant` |
| lua | `field` |
| markdown | `section` |
| json | `pair` |
| yaml | `block_mapping_pair`, `flow_pair` |
| toml | `table`, `table_array_element`, `pair` |
| ini | `section`, `setting` |
| nix | `binding` |
| css | `declaration` |
<!-- ordo:end members -->

These are node kinds, not concepts: a config format's member is its
key-value pair, so a nested key is a member of the key above it, and
markdown's `section` makes a subsection a member of its parent heading.

</details>

<details>
<summary><b>Rationale patterns</b> — one line per hunk, from comparing both sides</summary>

| Pattern | Example |
|---|---|
| cross-file def→use | `uses parse_cfg, defined in config.py` · `adds parse_cfg, used in main.py` |
| within-file order | `uses helper, defined above` · `adds helper, used by run below` |
| add vs edit | `adds helper` (new) · `edits run` (body of an existing def) |
| signature / type | `changes signature of parse` · `changes type Config` · `adds type Config` |
| value changed | `changes SOURCES` — a cmake `set()`, a make variable, a yaml key: a value has no signature |
| test ↔ code | `tests parse_cfg (config.py)` |
| rename / delete | `renames foo → bar` · `removes old_helper` · `removes import sys` |
| body deletion | `removes 31 lines` (a deletion inside a def, no symbol removed) |
| move / extract | `moves foo from a.py` · `adds read_input, extracted from order` |
| container | `edits it "parses flags"` · `edits #ifdef CURL_DISABLE_HTTP` · `edits ALLOWED_IMPORTS` |
| switched off | `comments out code in run` · `uncomments code in run` |
| import | `adds import degrade` · `changes import c, d, e` · `moves import pg` · `removes import logger` |

An import hunk is **noise, not nothing**: it never leads the reading order and
never seeds an edge, but it stays visible (dimmed) where the diff put it, and
says which import arrived, changed or moved. Dropping it outright made a new
dependency invisible, and made a *moved* import read as a deletion with no
counterpart.

Cross-file lines only appear when the changeset is sent as one call with
`cross_file: true`.

### From this review to every future one

Flag a hunk you never want to see the shape of again, and `:rule` drafts the
rule for it:

```toml
# paste into .ordo/rules.toml, then delete the conditions that were
# incidental — every line below is a fact about the hunk you flagged.

[[rule]]
name = "no-function-definition"
lang = "python"
kind = ["function_definition"]
max-params = 6
warn = "TODO: say why this shape is unwanted"
```

Every condition is a structural fact the engine already recorded about that
hunk, evaluated later by the same rules engine — no model in the path, and the
draft is reproducible from the hunk alone. Limits come out one below what the
hunk measured, so the rule fires on the thing that prompted it.

It is deliberately a *draft*, and deliberately over-specific: deleting a
condition that turned out to be incidental is easy, inventing the one that
mattered is not. A hunk that declares no symbol produces a rule with no `kind`
to match on, and the draft says so rather than quietly being broad.

### What a rejection would strand

Before pushing back on a hunk, the graph can say what else loses its footing:

```
· rejecting this strands 3 hunks (b.py:L4, c.py:L9, d.py:L2)
```

Transitive, not just the direct dependents — that is the whole point. Pushing
back on a leaf when the root is the problem sends the author round the loop
twice, so this changes how feedback gets sequenced: reject the root, and say so.
Reported in reading order, which is the order the fixes would be made in.
Dependency cycles are real (mutual recursion is one), and terminate on the
visited set rather than hanging.

### Since you last looked

After a force-push every tool replays the whole diff. ordo instead diffs two of
its **own runs** — `:delta`:

```
since you last looked
  3 new
  2 changed
  4 unchanged but reordered
  1 gone

  moved: api.py:L40   uses fetch, defined above
```

That third number is the one nothing else offers. A hunk can be **byte-identical
and still need re-reading**, because what it depends on changed and it now sits
somewhere else in the order. A diff sees nothing there, so the hunk looks
untouched while the reason to read it moved.

A hunk's identity across runs is its symbol and its file — not its content and
not the revision, since those are what the delta is measuring. The snapshot
lives in `${XDG_CACHE_HOME:-~/.cache}/ordo/runs/<repo>.json` and is overwritten
each time the review opens, so a delta always answers "since I last looked"
rather than "since some fixed point". On a first run there is nothing to
compare against, and it says so instead of calling everything new.

### Reviewing in the wrong order

Marking a hunk reviewed while the definition it depends on is still outstanding
means a call was approved before its callee. That is the one review-order
mistake the graph can prove, so it says so:

```
⚠ marked reviewed, but depends on unreviewed api.py:L4
```

It only fires on a hunk you have actually approved — an unreviewed hunk with
unreviewed dependencies is just work still to do, not a mistake.

Alongside it, the reviewer counts **edge coverage** as well as hunk coverage:

```
 HEAD — 8/10 reviewed · 4/9 edges · vim
```

Every tool reports how many hunks were ticked. An edge with *both* ends
reviewed is the number that tracks whether the relationship between two places
was actually checked — which is the thing a reading order exists to make
possible, and it is routinely much lower. Only edges whose ends are both in the
current view count, so filtering the review never makes the number look better
than it is.

### Notes that survive a rebase

A review note is anchored to a **symbol** — name, tree-sitter kind and
enclosing scope — not to `path:line`:

```
:note the retry path here needs a bounded backoff
```

A line anchor rots on the first rebase. This one survives a rebase, a move to
another file, an edit to the body, *and* a rename — the change ledger already
carries the name a symbol had before, so the note is migrated onto the new
identity when the rename is detected. This is the piece of infrastructure ordo
has that a line-oriented forge structurally cannot build.

The key is deliberately the exact opposite of a reviewed mark's. A mark folds
in the revision, the path and a hash of both sides of the hunk, because a stale
"already reviewed" is worse than a lost one. A note folds in none of them,
because it is a thought about the symbol rather than about one version of it.
A hunk that declares no symbol cannot be annotated at all — anchoring to the
enclosing name would silently drift onto whatever moved there next.

Notes live beside the marks, in
`${XDG_CACHE_HOME:-~/.cache}/ordo/notes/<repo>.json`, and `:note` with no text
clears the one on the selected symbol.

### The change ledger

One line per **symbol**, not per hunk — a forty-hunk diff read before any hunk
is opened:

| symbol | change | used by |
| --- | --- | --- |
| `fetch` | signature | 2 hunks |
| `backoff` | renamed from `old_helper` | — |
| `main` | body | — |
| `old_fetch` | removed | — |

```json
"ledger": [
  { "name": "fetch", "kind": "function_definition", "path": "api.py",
    "at": "h1", "change": "signature", "used_by": ["h2", "h3"] }
]
```

`ordo-engine pack` leads with it, between the changeset notes and the reading
order — the executive summary before the hunks. In the reviewer, **the ledger
is the default view**: the list is a list of symbols, with the hunks that
changed each one folded underneath. `:mode hunks` switches back to the flat
reading order, `:mode` alone toggles. A hunk that changes nothing nameable — an
import, a formatting fix — sits under `no symbol changed` rather than
disappearing.

Every entry carries `at`, the hunk it is anchored to, because a ledger line you
cannot jump to is not actionable.

Entries follow the reading order of the hunk that defines them, so the ledger
and the hunk list tell the same story in the same sequence. Every field is a
projection of what the engine already computed — `symbols`, the per-file status
maps, and each hunk's `uses` — so this is a second view of the change, not a
second analysis of it.

Two ceilings worth stating. A **removed** symbol carries no `kind`, because the
node that would answer no longer exists; and a symbol is only reported removed
when some hunk actually deletes its lines, since a caller who sends `old` + a
context-limited `diff` gives the engine no new-side symbol set to compare
against. The **fan-in** counts uses *within the change*, which is the honest
scope: those are the call sites the author touched.

### Call sites that did not follow the signature

`changes signature of fetch` is the easy sentence. The one that matters is
whether the calls agree — and after a signature change ordo has both the new
parameter list and every call to it in the change:

```
api.py:L1  changes signature of fetch
  1 of 2 call sites in this change do not pass 2 arguments to fetch (cli.py:L4)
```

Change the function, update most callers, miss one: that is the most common way
an edit goes wrong, and it is decidable from the tree.

Deliberately narrow, because a false "wrong number of arguments" is worse than
a missed one. It stays silent for a **variadic** definition (`*args` makes the
upper bound meaningless), a **method** (the receiver is passed implicitly, so
counting the two against each other would report every method call as short by
one), a **qualified callee** (`obj.f(...)` may be passing a receiver) and a call
using **keyword arguments** (passing by name says nothing about positional
arity). An optional parameter widens the accepted range rather than narrowing
it. Arity only, never types — and only callers *in the change*, which is the
honest scope: those are the ones the author touched.

### The rename that did not finish

Rename detection already says `renames parse_cfg → load_cfg`. The question it
leaves open is whether the old name still appears anywhere:

```
cfg.py:L1  renames parse_cfg → load_cfg
  parse_cfg still used at main.py:L6 after the rename to load_cfg
```

Searched across the **whole new content** of every changed file, not just its
hunks — a reference on a line nobody touched is exactly the one that gets
missed. Only identifiers count, so the old name surviving in a string or a
comment says nothing, and it stays silent when the old name still defines
something in the change, since then it is a name that legitimately still exists
rather than an orphaned reference.

**Ceiling:** files *in the change* only. A caller in a file the author never
opened is invisible to the pure engine — finding that one needs repo access,
which is the reviewer's job rather than the engine's.

The changeset as a whole carries `notes` too — facts about the shape of the
change, never judgments about it:

```json
"notes": [
  "code changed but no test touched",
  "src/parser.py: 14 hunks (high churn)"
]
```

A sharper sibling of the first one: a test file *was* touched, but what the
change wrote there references none of the definitions the change altered.

```json
"notes": [
  "tests/test_api.py touched, but none of its uses reference the 1 changed def"
]
```

That is the shape of a test which exercises something adjacent to the thing
that moved. It reads the hunks of the test file rather than its whole content,
because untouched tests are existing coverage rather than part of this change.
The two test notes are mutually exclusive by construction — a changeset either
touched no test at all, or touched one that missed.

"Code" means any supported language that is neither prose nor a config format,
so a docs-only or CI-config-only change never reports an untouched test suite.
It does include css and html: a stylesheet-only change is still reported, since
tightening that would need a notion of "language people write tests for" the
registry does not have. Both notes lead the `ordo-engine pack` output, ahead of
the reading order.

Hunks also carry structural `notes` (large/deeply-nested/param-heavy defs) and
**advisories** — advanced-construct guidance with an escalation ladder, and a
`verdict` when a downgrade is concretely warranted:

```
registry.py:L2  metaclass ⚠
  metaclass — 90% of the time the wrong tool. Lightest sufficient step:
  1. configure one attribute → __set_name__ (descriptor)
  2. react to subclassing → __init_subclass__
  3. replace the class after it's built → class decorator
  4. rewrite the class as built / control instances → metaclass
  ⚠ this metaclass overrides only __init__ — __init_subclass__ likely suffices.
```

</details>

<details>
<summary><b>Advisory catalog</b> — a curated list, not a style linter</summary>

Detection is deterministic tree-sitter, verdicts fire only when the pattern is
concretely wrong (`src/advisories.rs`):

| lang | advisory (ladder) | ⚠ verdict (concretely wrong) |
|---|---|---|
| python | metaclass, `eval`/`exec`, dynamic `type()`, `__del__`, `os.system`, `pickle`, `__getattribute__`, `suppress(Exception)` | mutable-default-arg, bare/empty-`except`, `assert`-validation, register-only metaclass, `__eq__` w/o `__hash__`, `subprocess(shell=True)`, blocking-call-in-async, `lru_cache`-on-method, SQL f-string, TLS `verify=False`, unsafe `yaml.load`, fire-and-forget task, half context-manager |
| rust | `unsafe`, `mem::transmute` | `static mut` |
| js/ts | `eval`, `any` | `with`, empty-`catch` |
| go | `unsafe`, `reflect`, `panic` (non-test) | — |
| c | `goto` | `strcpy`/`sprintf`/`gets`/`scanf` (buffer overflow) |
| c++ | (all of c) raw `new`/`delete`, `malloc`/`free`, C-style/`reinterpret`/`const`/`dynamic` cast, function-like macro, `using namespace std`, `volatile`, `[&]` capture, `memcpy` family, `system`/`exec*`, `alloca`, non-reentrant runtime, catch-by-value | unsafe string fns, `using namespace std` in a header, throw in destructor/`noexcept`, `setjmp`/`longjmp`, `operator&&`/`\|\|`/`,` overload |
| java | reflection (`setAccessible`) | empty-`catch` |

`assert`/`panic` fire only outside test files.

</details>

