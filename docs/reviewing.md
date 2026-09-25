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

### Comments on lines

A point narrower than a symbol goes on its lines. In the code pane, `v` starts
a selection at the cursor and moving extends it; `c` opens `:comment` on the
selection, or on the cursor's line with none, already holding the text when
those lines have a comment, so the same key edits it. `:comment` with no text
deletes, `gc` / `gC` walk the review's comments in file and line order, and
`:comments` lists them. A commented line carries `●` in the gutter, and the why
pane shows the comments inside the selected hunk.

Unlike a note, a comment stays on its lines. When an edit above moves them, it
finds them again by a hash of their text and moves along; when the lines
themselves change, it keeps its place and says so — `(lines changed since)` —
rather than vanishing. Comments live beside the notes, in
`${XDG_CACHE_HOME:-~/.cache}/ordo/comments/<repo>.json`, which records the
lines' hash, never their text.

### Handing the review back

When an agent wrote the change, the review goes back to it as one prompt:
every note, anchored to the file and the new-side line range of its hunk, and
every line comment on its own lines.
`:yank` puts it on the clipboard through the terminal (OSC 52, so it works over
ssh), `:send` pipes it to the command in `$ORDO_SEND`, and either one with
`all` adds ordo's own notes and findings — the contract notes above, the catalog
— for when you agree with them and would rather not retype them:

```
Review of HEAD~1..HEAD (2 points). Each is anchored to a file and a line range
of the changed code; address them, or say why not.

## api.py:12-18
the retry path here needs a bounded backoff
- ordo: fetch became async; 1 call never awaits it, so it never runs (cli.py:L4)
```

Notes stay after sending; a note is yours until you clear it.

### Reviewing work that is still changing

An agent keeps editing while you read. `ordo --watch zz` (or `main...zz`)
notices: every two seconds it fingerprints the working tree — `git status`, the
size and mtime of each changed file, and `HEAD` — and once the tree has held
still for the debounce (1500 ms, `--watch-debounce <ms>`), the status line
says `stale · 3 files changed · r reloads`. No file-watcher library: polling
has no per-platform backend and misses nothing on a network or overlay
filesystem.

`r` reads the change again and keeps your place. The selected hunk is found
again by its symbol and file — through a rename too, by the engine's ledger —
or, when it is gone, the next one in the reading order takes its place. Scroll
and cursor stay where they were relative to it, and the filters, the text
search and the jump stack come along. The status line then says what moved:
`reloaded · 2 changed · 1 reordered · 1 gone`. `:e` keeps your place the same
way.

`--watch=auto` reloads by itself, but never while a prompt, a popup, the
command bar or the config is open, nor within three seconds of a key. Two
things are only ever reported:
- **A commit** empties `zz` of what was committed. The status line says so,
  and suggests `:e main...zz`, which keeps the committed work in view.
- **A rebase or branch switch** moves the base under the review, which a
  reload would silently re-anchor.

`watch = "hint"` in `tui.toml` turns it on for every review; `:watch on`,
`off` and `auto` change it live. A committed revision cannot drift, so there
it says `nothing to watch`.

### Waves

An agent works in turns, and a turn is a natural unit to review. `ordo wave`
records the working tree as the next wave, a commit chained to the previous
one under `refs/worktree/ordo/waves/`. It builds the snapshot through a scratch
copy of the index, so your index, branches and GitButler workspace are never
touched, and a turn that changed nothing records nothing. The first wave is
the starting point:

```sh
ordo wave                  # before the agent starts: wave/0
ordo wave -m "add retry"   # after each turn: wave/1, wave/2, …
ordo wave/1..wave/2        # review one turn
ordo wave/0..wave/last     # everything so far
ordo wave --list           # what is recorded; --clear forgets it
```

Reviewing a range of waves, each hunk carries the wave that last changed its
lines (`wave 2` in the why pane): `git blame` along the chain, and for a
removed line the wave after the last one that still had it. Against the
working tree (`wave/0..zz`), edits no wave has recorded yet count as the next.

`:only-wave 3` narrows the view to what wave 3 changed, while the engine
still orders the whole range, so the def→use graph stays whole. A dep line
into another wave stays, saying which wave and whether you reviewed it
(`dep ← parse_rev · wave 1 ✓`); `K` previews it, `gd` switches to that wave
and goes there, and `C-o` comes back to where you were. `:only-wave last` is
the newest, `:only-wave all` lifts it, and `:audit` counts what it hides.

`:wave` records one from inside a review. The refs are per worktree, so two
linked worktrees keep separate chains. herdr-ordo records a wave each time the
agent goes from working back to idle, opt in with `WAVES=1`.

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

### Contracts that changed without a line saying so

Some edits read fine line by line and still break every caller, silently. Only
a diff sees them, because each needs the old side of a definition next to the
new one and the calls that were written against the old:

```
api.py:L1  changes signature of fetch
  fetch became async; 2 calls never await it, so it never runs (cli.py:L4, cli.py:L5)
  fetch's parameters went from (u, retries) to (u, timeout, retries); 1 call left as it was passes 2 or more by position, which now land on different parameters (cli.py:L3)
```

| the change | what goes wrong, quietly |
| --- | --- |
| `def f` → `async def f` | a call not awaited gets a coroutine: never runs, always truthy |
| `@property` dropped | `obj.x` is now a bound method: `if obj.x:` is always true |
| a parameter inserted or reordered mid-signature | calls left as they were hand values to the wrong parameters |
| a parameter renamed or removed | a call still passing it by name raises when that line runs |
| a default changed | every call leaving it out changes behaviour, with no line in the diff |
| a base method's parameters changed | overrides still taking the old ones |
| a new `@abstractmethod` | subclasses that do not define it can no longer be made |
| `raise KeyError` → `raise MissingKey` | handlers catching only the old type |
| a `with` or an `await` removed | the body lost its lock, transaction or wait |
| an enum member's value changed | stored or sent copies of the old value stop mapping back |
| a symbol renamed or moved | `mock.patch("pkg.cfg.old")`, entry points and other strings still naming it |
| a test drops assertions or gains a skip | next to an edit of the code it exercises: asked, not judged |
| an added import | it closes an import cycle among the changed files |
| a function added to two files | nearly verbatim: the next fix reaches one |

Python and JS/TS only, where these shapes exist; elsewhere the walk costs time
and finds nothing. Every check speaks only when it can name the calls, and
matches a function by bare name and a method only through `self.`/`this.` in its
own file.

**Other repositories.** A script's `main` changed, and its only caller is in the
repository next door, which imports it. The reviewer hands such files to the
engine as `consumers` — read only as callers, never ordered or shown as hunks —
and the notes above name their calls, as `../pipeline/driver.py:L3`. It finds
them two ways: repositories listed in a rules file, relative to that file, and
sibling repositories whose `pyproject.toml`, `uv.lock`, `requirements.txt` or
`package.json` points back at this one by relative path:

```toml
# <repo>/.ordo/rules.toml
consumers = ["../../pipeline"]
```

Only tracked files that mention a changed module (`script.run`) are read, at
most 200 and 8 MB. Through the engine directly it is `consumers` on the input.

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
`findings` — one list for everything anyone noticed, tagged with its `source`
(`catalog`, `rule` or `analyzer`) and a `level` of `note`, `warn` or `verdict`.
The catalog's own entries are advanced-construct guidance with an escalation
ladder, at `verdict` level when a downgrade is concretely warranted:

```
registry.py:L2  metaclass (catalog) ⚠
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
concretely wrong. Most of the catalog is **data** — `rulesets/catalog/*.toml`,
written in the same `[[rule]]` grammar as any ruleset and compiled into the
engine (`src/catalog.rs`), so a construct can be read, copied into your own
rules and reworded. It is on unless you turn it off — `--no-catalog` for a run,
`catalog = false` in a rules file for good, `disable = ["goto"]` for one entry
(see [rules.md](rules.md)). What the rule language cannot state stays as a
walker in `src/advisories.rs`, whose module doc lists each one and why:

| lang | advisory (ladder) | ⚠ verdict (concretely wrong) |
|---|---|---|
| python | metaclass, `eval`/`exec`, dynamic `type()`, `__del__`, `os.system`, `pickle`, `__getattribute__`, `suppress(Exception)` | mutable-default-arg, bare/empty-`except`, `assert`-validation, register-only metaclass, `__eq__` w/o `__hash__`, `subprocess(shell=True)`, blocking-call-in-async, `lru_cache`-on-method, SQL f-string, TLS `verify=False`, unsafe `yaml.load`, fire-and-forget task, half context-manager |
| rust | `unsafe`, `mem::transmute` | `static mut` |
| js/ts | `eval`, `any` | `with`, empty-`catch` |
| go | `unsafe`, `reflect`, `panic` (non-test) | — |
| c | `goto` | `strcpy`/`sprintf`/`gets`/`scanf` (buffer overflow) |
| c++ | (all of c) raw `new`/`delete`, `malloc`/`free`, C-style/`reinterpret`/`const`/`dynamic` cast, function-like macro, `using namespace std`, `volatile`, `[&]` capture, `memcpy` family, `system`/`exec*`, `alloca`, non-reentrant runtime, catch-by-value | unsafe string fns, `using namespace std` in a header, throw in destructor/`noexcept`, `setjmp`/`longjmp`, `operator&&`/`\|\|`/`,` overload |
| java | reflection (`setAccessible`) | empty-`catch` |
| any | hardcoded version in a string, absolute path, bare number in a comparison or an argument | — |

`assert`, `panic` and every hardcoding rule fire only outside test files — a
test's hardcoded path or expected number is the fixture. `0`, `1`, `2` and `-1`
are never called magic.

</details>

