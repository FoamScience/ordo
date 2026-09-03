# Document languages — css, html, and the single-file component

Status: design (P22.4). Nothing here is built yet. Every node kind named below
was read off the real grammar, not the docs — the probes are in the tables.

## The problem, stated honestly

The walk assumes a *definition* is a construct the language itself names, and a
*use* is an identifier the language itself resolves against that name. Every one
of the 21 shipped languages fits, including the three that are not code: a
markdown section is named by its heading, a yaml key by its key, a jinja macro
by its opening statement. Approximate resolution works there because the
approximation is only "which `parse_cfg`?", never "is this a name at all?".

CSS, HTML and an SFC each break a different half of that.

| | breaks |
| --- | --- |
| **css** | a rule set *is* named — by a selector, which is punctuation-shaped and matches nothing any other grammar produces. But its `class_name` wraps a plain `identifier`, a kind already in the global `IDENT_KINDS`, so the naive entry floods the shared symbol table with bare words like `title` and `root` |
| **html** | almost nothing in it is named. An `id` is; a `class` is a whitespace-delimited *string*, not a token; a `<div>` has no name at all |
| **vue / svelte** | one file, three languages, and the `<script>` block is the only one holding real definitions — which is precisely the thing the injection contract refuses to record |

## Grammar availability, verified

Built against `tree-sitter = "0.25"` in a scratch crate and parsed real files.

| crate | version | ABI | verdict |
| --- | --- | --- | --- |
| `tree-sitter-css` | 0.25.0, MIT, `tree-sitter/` org | `tree-sitter-language` 0.1.8, no `tree-sitter` dep | **take it** |
| `tree-sitter-html` | 0.23.2, MIT, `tree-sitter/` org | same | **take it** |
| `tree-sitter-svelte-ng` | 1.0.2, MIT, `tree-sitter-grammars/` org | same | **take it**, but only when Svelte earns it |
| `tree-sitter-vue` | 0.0.3 | pins **tree-sitter 0.20** | **reject** — `cargo tree -i tree-sitter@0.20.10` shows it pulling a second tree-sitter into the tree. Identical to the `tree-sitter-dockerfile` rejection |
| `tree-sitter-vue-updated`, `-next`, `octorus-…-vue3` | 0.1.0 each | — | **reject** — single-author forks with no track record, the `tree-sitter-dockerfile-updated` category |
| `tree-sitter-scss`, `tree-sitter-less` | — | — | **not needed**, see the SCSS ceiling below |

All three keepers export `HIGHLIGHTS_QUERY`, so the TUI's syntax pane is one
line per language.

The vendoring route (`build.rs` + `grammars/`) is available and is **not
needed**: nothing here requires an unpublished grammar.

---

## 1. What is a definition?

### css — the rule set, named by its whole selector list

A `rule_set` is what a reviewer navigates to, it can be named, and the name they
use for it is the selector as written. Not the individual selector: `.btn,
.btn-primary { … }` is one block you open once.

```
rule_set
  selectors            ".btn, .btn-primary"     ← the name
    class_selector → class_name → identifier
  block
    declaration → property_name + value
```

**A CSS name keeps its sigil.** `.btn`, `#nav`, `--brand`, `@keyframes spin`.
This is not cosmetic — it is the entire cross-language safety story, and section
2 turns on it. No code grammar produces an identifier beginning with `.`, `#` or
`--`, so a CSS symbol is lexically incapable of colliding with a Python
function, and the union symbol table stays honest for free.

| construct | node kind | is it a definition? |
| --- | --- | --- |
| rule set | `rule_set` | **yes** — named `.btn, .btn-primary` |
| `@keyframes spin` | `keyframes_statement` (child `keyframes_name`) | **yes** — named `@keyframes spin` |
| `--brand: #05f` | `declaration` whose `property_name` starts `--` | **yes** — named `--brand`. The one real symbol CSS has |
| `color: red` | `declaration` | no — a **member**, see section 3 |
| `@media (…)`, `@supports (…)` | `media_statement`, `supports_statement` | no — a **region**. `@media` is a condition under which rules apply, exactly what `#ifdef` is. `ContainerKind::Region` already means this and already refuses to seed edges |
| `@import "reset.css"` | `import_statement` | an **import**, named by the file it pulls in |
| `@font-face`, `@charset` | `at_rule` | no name, stays transparent. Don't build |
| a nested `rule_set` (`& .child`) | `rule_set` | yes, same as any other — CSS nesting parses fine in 0.25 |

A Region does not scope, so `.btn` inside `@media` is still `.btn` — which is
right: it *is* the same `.btn`, responsively overridden. The cost is that two
`Symbol` entries collide on `{name, kind, scope}`. That is the same collision
class already documented at `symbol_identity_key`; state it, don't fix it.

### html — an element with an `id`, and nothing else

An `id` is the only thing in an HTML document that is (a) named by a single
token, (b) navigable, (c) referenced by name — `href="#nav"`, `<label for>`,
`aria-labelledby`, `getElementById`. It is also unique per document *by spec*,
which is a stronger guarantee than any language ordo already handles.

So: `element` is a def; `node_name` returns `#` + the `id` attribute's value, or
**None** for the overwhelming majority of elements that have no id. None is not
a special case — `walk` already descends transparently through an unnamed def
(the anonymous-namespace / lambda path). A plain `<div>` therefore contributes
nothing at all, which is the correct amount.

**Everything else in HTML is not a definition and should not be made one.** A
`<div>` has no name; `div:nth-of-type(3)` is a path, not a name, and no reviewer
navigates by it. A tag name names an element *kind*, shared by four hundred
nodes in the file.

### vue / svelte — the file is not the definition site

An SFC declares one component, named by its **filename**, not by anything inside
it. `<script setup>` holds functions and refs; `<template>` holds markup;
`<style>` holds CSS rule sets. Section 4 is about which of those ordo should
record.

---

## 2. Is a class name in HTML a use of the CSS selector that defines it?

**No. Do not emit selector→class edges.** This is the headline call and it is a
refusal.

The README's ceiling — "name match with an optional cross-file union, no full
scope/type analysis" — is an approximation of *resolution*. It says: the grammar
told me this token is an identifier; I don't know which of the three `parse_cfg`
definitions it binds to, so I'll link them all. Every existing approximation has
that shape.

A selector→class edge would be a different kind of claim entirely:

**(a) The name is not a name.** `class="btn btn-primary tw-flex tw-p-2"` is one
`attribute_value` node holding one string — verified, the grammar does not
tokenize it. To make uses out of it, ordo would have to split a string literal on
whitespace and assert the pieces are identifiers. It does that nowhere else, for
good reason: the grammar is what makes a token a token, and here the grammar
declines to say so.

**(b) The names collide with code, and the collision is the common case.**
`card`, `title`, `row`, `active`, `hidden`, `container`, `header`, `content`,
`main`, `error` are class names *and* function and variable names in the same
repository. Under `cross_file: true` and name-only matching, `.title { }` in
`app.css` would link to a Python `title()`. That is not an approximation of the
truth. It is a false statement, and it degrades the ordering of the *other*
files, because everything is ordered against one union symbol table.

**(c) Cardinality is inverted.** A single Tailwind `<div class="flex items-center
gap-2 rounded-lg border p-4">` yields eight uses, resolving to definitions in a
generated stylesheet nobody reviews. The rationale becomes a wall.

**(d) The real relationship is usually invisible.** CSS modules resolve
`styles.card` to a hashed class; Vue's `scoped` appends `data-v-7ba5bd90`; BEM
names are composed at runtime through `clsx`; Tailwind's `@apply` inverts the
direction. The string in the markup frequently is not the string in the
stylesheet.

**(e) And the edge is the wrong direction for P2 anyway.** "Definitions before
their uses" exists because you cannot understand a call without knowing what it
does. You can understand `<div class="card">` perfectly well without reading
`.card { padding: 1rem }`. Presentation does not carry the semantics the
principle is about.

### The concrete rule

> **An edge into CSS is emitted only when the referencing token is one the CSS
> grammar itself produces.** In practice that is exactly one form:
> `var(--brand)` referencing `--brand: …`. Nothing outside a `.css` file can
> produce such a token, so nothing outside a `.css` file can link into one.

The *implementation* of that rule is the sigil convention from section 1: CSS
symbols are named `.btn` / `#nav` / `--brand` / `@keyframes spin`, and no other
grammar in the tree emits an identifier of that shape. The rule enforces itself
structurally rather than through a policy check — which is the same trick yaml
anchors use, and it leaves the door open: if html→css edges are ever wanted, the
html side simply starts emitting `.btn`, and nothing else has to change.

### The custom property is CSS's yaml anchor

`--brand: #05f` is a definition and `var(--brand)` is a use. Both sides are
unambiguous, both are single tokens the grammar produces, both are scoped to a
document, and neither can collide with anything. It is the one thing that lets a
CSS hunk be **ordered** rather than merely described:

```
tokens.css:2      adds --brand, used by .btn below
components.css:9  uses --brand, defined in tokens.css
```

Extraction is symmetric and keyed on the `--` prefix on both sides:

- def: `declaration` whose `property_name` text starts with `--`
- use: `plain_value` whose text starts with `--` (probe: `var(--brand)` →
  `call_expression{function_name "var", arguments{plain_value "--brand"}}`)

### The hazard that makes this urgent

In `tree-sitter-css`, `identifier` appears under exactly one parent — verified by
reading `node-types.json`, which lists `class_name` as the sole node with an
`identifier` child. And `class_name` is what the grammar uses for *both* a class
selector's name and a pseudo-class's name:

```
.a:is(.b, .c)
  pseudo_class_selector
    class_selector → class_name → identifier   "a"
    class_name                                  "is"      ← the pseudo-class
    arguments → class_selector → class_name → identifier "b", "c"
```

`identifier` is in the global `IDENT_KINDS`. So a css entry with no guard makes
`btn`, `card`, `title`, `hover`, `root`, `is` into bare `uses` — precisely the
false links argued against above, arriving through the back door.

**The guard is one line, and it has exact precedent** (nix's `attrpath`, which
returns early for the same reason — a name being selected is not a reference):

```rust
if spec.name == "css" && kind == "selectors" { return; }
```

The selector text is still read, by `node_name` on the parent `rule_set`, which
reads its children directly rather than through `walk`. Nothing is lost.

---

## 3. Is a CSS declaration a member?

**Yes, and it reads well.** A `declaration` is a member of its `rule_set` in
exactly the way a yaml key is a member of the key above it:

```
app.css:12  edits .btn
  details:
    - changes color in .btn
    - adds border-radius to .btn
    - removes padding from .btn
```

That is the detail layer doing its job on a file type where the container name
alone (`.btn`) genuinely does not say what happened.

Two small things stand in the way, both in shared code, both css-unique so
neither can change a shipped language:

**`member_name` cannot name a declaration.** It tries `name`/`key` fields, then
`declarator`, then the first identifier-ish direct child. A `declaration`'s
direct children are `property_name` and a value node; none is an ident kind, so
it returns `None`. Fix: check for a direct `property_name` child. `property_name`
exists in no other shipped grammar (checked against every `node-types.json` in
the tree).

**`collect_bodies` has no body to split on.** A `rule_set` has no `body` or
`value` field, so `header` falls back to the *whole node text* — meaning every
edit to any declaration reads `changes signature of .btn`, and a renamed selector
with an identical block never matches as a rename because `whole` still contains
the old selector. Fix: when a def has neither field, take a `block` child as the
body. Gate it on `rule_set` — `block` on its own collides with go, java, lua,
python, rust, cmake and xonsh.

That second fix is worth flagging: it is a *cheaper and better* answer than the
`has_signature(kind)` follow-up parked under P22.1, because it fixes rename
matching too, and because it touches nothing outside CSS. `has_signature` stays
parked.

**Not yet:** a nested `rule_set` as a member of its parent (`data: true` plus
`rule_set` in `members`). CSS nesting is real and the machinery would carry it,
but "adds .child to .parent" is a thin win for a construct that is still rare in
shipped stylesheets. Keep `data: false` at first.

---

## 4. Single-file components

### Vue is HTML. Svelte is not.

This is the most useful thing the probes turned up, and it is decided by one
character.

`tree-sitter-html` parses a hostile `.vue` file — `<script setup lang="ts">`,
self-closing components (`<Child … />`), `v-for`, `:key`, `@click`, `{{ … }}`,
`<slot name="foot" />`, `<style module lang="scss">` — with **zero error
nodes**. It works because Vue puts every expression inside a *quoted attribute*
or inside `{{ }}` text, and html tokenizes both.

Svelte puts expressions in bare `{ … }`, where a `>` closes a tag:

| construct | html parses? |
| --- | --- |
| `{#each items as it}…{/each}` | ✓ |
| `{#if a}…{:else}…{/if}` | ✓ |
| `<svelte:head>`, `<Child {x} />`, `bind:value={it.v}` | ✓ |
| `{#if x > 1}` | **✗** |
| `<button on:click={() => f()}>` | **✗** |

An arrow function in an event handler is on half the components in a real
Svelte project. So:

- **`.vue` → `tree-sitter-html`.** No new grammar, no vendoring, no fork.
- **`.svelte` → `tree-sitter-svelte-ng` 1.0.2**, when Svelte is done at all.

### Should an SFC's `<script>` contribute definitions?

**Not yet — record uses only, exactly as a markdown fence does.** The argument
is a cost, and the cost is concrete rather than aesthetic.

`inject_fence` writes into `c.uses` and nothing else. Recording defs means also
writing `def_rows`, `decls`, `sym_decls`, `defs` (with real start/end rows),
`member_rows`, `local_binds`, `bound`, `import_rows` and `import_decls` — all
shifted by the block's row offset. `walk` reads `node.start_position().row`
directly at roughly twenty sites, so this is a genuinely new concept: **a
sub-tree whose rows are not file rows.**

And that is only half of it. `analyze` is one of **ten** whole-file parse entry
points in `extract.rs`:

```
local_names  member_rows  top_level_bindings  symbol_rows  symbol_bodies
import_row_set  import_statements  field_initializers  mask_template  template_uses
```

Eight of those are *old-side* collectors feeding add-vs-edit, rename, move,
signature-change and import-move wording. Teach only `analyze` about injected
definitions and the old side never sees them — so **every function in every SFC
reads as newly added on every single commit**, and rename detection is
permanently wrong. Doing it right means all ten route through one offset-aware
sub-parse primitive.

That is a large change justified by nothing yet observed, which is the definition
of the thing this project's style refuses to build.

### The upgrade path, recorded rather than built

There is a much cheaper route to *real* definitions, and it should be written
down so nobody reinvents the expensive one:

> Set `for_path(".vue") → typescript` and `template_lang(".vue") → an html-based
> `Template` whose `literal` is the `<script>` block's `raw_text`. Masking runs
> in `lib.rs::mask_templates` **before everything else**, rewriting `old` and
> `new` at identical byte, row and column offsets — so all ten entry points see
> valid TypeScript for free, with correct positions. This is exactly what
> already happens for `values.yaml.j2`.

Its cost is the mirror image: `<template>` and `<style>` become blank, so a
markup-only hunk has no container and only the masked-row un-noising to save it
— and in a real `.vue` diff, markup is usually the majority. It trades "the
script is real" for "the template is invisible".

It also needs the one genuinely new concept in this whole design: `Template.literal`
is a flat list of node kinds, and this needs *"`raw_text`, but only under a
`script_element`"* — a predicate rather than a list. Do not add that predicate on
speculation. Add it the day someone reports that def-level SFC review matters.

### `<style>` is deliberately not injected

Injection is **uses only**. A `<style>` block's content is *definitions* — rule
sets and custom properties. Injecting it would therefore contribute nothing
useful, and would contribute something harmful: `collect_injected_uses` harvests
every `is_ident` node, which in CSS means every `class_name → identifier`, i.e.
exactly the flood section 2 exists to prevent.

So `<script>` is injected and `<style>` is not, and the reason is the injection
contract itself rather than a special case.

A `<style>` hunk still gets a name, via `region_label`: `script_element` and
`style_element` become regions named by their tag and their distinguishing
attributes — `<script setup>`, `<style scoped>`, `<script context="module">`,
`<style module lang="scss">`. That is what a reviewer calls those blocks, and it
turns `edits code` into `edits <style scoped>`. Both kinds are unique to
html + svelte-ng among shipped grammars.

---

## 5. Reused vs. new

### Reused, unchanged

| machinery | used for |
| --- | --- |
| `ContainerKind::Region` + `region_label` | `@media` / `@supports`; `<script>` / `<style>` blocks. Both are "a container that holds code and declares nothing", which is the doc comment verbatim |
| unnamed-def transparency in `walk` | every HTML element without an `id`. No guard needed — `node_name` returns `None` and the walk descends |
| the yaml-anchor pattern (a `matches!` arm writing `decls` + `uses`) | `--brand` / `var(--brand)` |
| the nix `attrpath` early return | suppressing `selectors` |
| `inject_fence` / `collect_injected_uses` | `<script>` blocks — a sibling entry point, same body |
| `lang::for_lang_name` | resolving `lang="ts"` on a `<script>` tag. Already understands `ts`/`js`/`tsx` |
| `Template` / `mask_template` | **nothing here.** Recorded as the SFC upgrade path only |
| `prose` / `data` flags | **neither**, at first. `data: true` is the follow-up for CSS nesting members |

### New — and it is very little

1. A naming path for `rule_set` and `keyframes_statement` in `node_name_inner`,
   plus a `node_name` early return so `tidy_ident` does not strip the spaces out
   of `.card .title` (turning it into a different selector). This is the second
   entry on that early-return list; markdown's `section` is the first, for the
   same reason.
2. A naming path for `element` (the `id` attribute).
3. `member_name` learning `property_name`.
4. `collect_bodies` taking a `block` child as a body when there is no `body` or
   `value` field, gated to `rule_set`.
5. `inject_element` — a sibling of `inject_fence` for `script_element`.

There is **no new `LangSpec` field, no new `ContainerKind`, and no new flag.**
Every kind these touch (`rule_set`, `selectors`, `keyframes_statement`,
`property_name`, `plain_value`, `element`, `script_element`, `style_element`,
`media_statement`, `supports_statement`) was checked against every shipped
grammar's `node-types.json` and appears in none of them.

### Risk to already-shipped languages — read this part

**One item, and it is severe if missed.**

> **The css `identifier` leak.** `identifier` is in the global `IDENT_KINDS`, and
> in `tree-sitter-css` it appears only under `class_name` — which the grammar
> uses for both class selectors and pseudo-classes. Without the `selectors`
> guard, a css entry emits bare `uses` for `btn`, `card`, `title`, `active`,
> `root`, `hover`, `is`, `container`, `main`. Those enter the **union symbol
> table every other file is ordered against**, so a stylesheet would start
> drawing def→use edges to Python functions and TypeScript classes sharing a
> name. This does not merely make CSS wrong; it degrades the ordering of
> languages that already work.
>
> Mitigation: the one-line `selectors` return. It must land in the same commit
> as the css entry, and it wants a test named after it.

Lesser items:

- **`collect_bodies` must stay gated.** A language-agnostic body fallback would
  change wording for rust `const_item`, cmake `set()` and make variables, all of
  which have golden files.
- **Svelte's `if_statement` is in `CONTROL_KINDS`.** A `{#if}` block increments
  `nest_depth`, so a heavily-templated component can pick up a spurious
  "deeply nested" structural note. Cosmetic; note it, do not chase it.
- **`.min.css` is already in `is_generated_path`.** Nothing to do; mentioned so
  nobody adds it twice.

---

## 6. Recommended order

| # | step | size | why here |
| --- | --- | --- | --- |
| 1 | **css** — registry entry, the `selectors` guard, `rule_set` / `keyframes_statement` naming, `--`/`var()` def→use, `@media` and `@supports` as regions, `declaration` as member, `member_name` + `collect_bodies` fixes | ~80 lines + one golden fixture | Self-contained, no other language touches it, and it is the only one of the three with a real def→use pair. It also gets the dangerous change (the guard) done first, under test |
| 2 | **html + vue** — one registry entry serving `.html` / `.htm` / `.vue`, `element`-by-`id` naming, `<script>`/`<style>` as regions, `inject_element` for `<script>` | ~75 lines + fixtures for both extensions | Depends on nothing from step 1. Vue arrives free with html — it is the same grammar and the same entry |
| 3 | **svelte** — `tree-sitter-svelte-ng` 1.0.2, a registry entry mirroring html's | ~25 lines | A new dependency for one extension. Do it only once steps 1–2 are shipped and someone actually reviews Svelte |

Do **not** ship html as an inert entry ahead of step 2. None of html's node
kinds are in `IDENT_KINDS`, so a bare entry parses cleanly and extracts
absolutely nothing — which flips `unsupported: false` on a file that was in fact
not read. Today's `unsupported: true` is the more honest output. HTML earns its
entry when injection and `#id` land with it.

### Don't build this yet

- **selector→class edges**, in either direction, including the "just for CSS
  modules" version — section 2 is the whole argument
- **`.scss` and `.less`** through the css grammar. A real SCSS file produces
  **13 error nodes** and a LESS file **5** (probed: `$brand`, `@mixin`,
  `@include`, `&__title`, `@extend`, `@use`). Feeding them to the css grammar is
  the `ssh_config`-into-ini mistake the README already names. They stay
  `unsupported: true` until `tree-sitter-scss` earns its own entry
- **SFC `<script>` definitions** — section 4, including the masking route
- **`<link rel="stylesheet">` / `<script src>` as imports** — genuinely useful,
  but `import_like` is keyed on node kind and this needs an attribute predicate
- **`animation: spin` as a use of `@keyframes spin`** — requires reading
  `plain_value` conditioned on the sibling `property_name`, and buys one edge
  per stylesheet
- **CSS nesting as members** (`data: true` + `rule_set` in `members`)
- **plain HTML elements as containers** — a `<div>` has no name
- **Svelte `{expr}` uses** — `svelte_raw_text` is an opaque blob, the same shape
  ERB's `code` has, and ERB's answer (no uses, honestly) applies unchanged
- **Vue's own grammar** — revisit only if a maintained crate builds against a
  current tree-sitter

## Ceilings this adds

- **CSS names carry their sigil, and that is the security boundary.** `.btn`,
  `#nav`, `--brand`, `@keyframes spin`. Nothing outside a stylesheet can
  reference a CSS symbol, by construction rather than by policy. A class name in
  markup is not a use of the selector that defines it, and ordo says so rather
  than guessing.
- **`--custom-property` / `var()` is the only def→use pair CSS has.** Everything
  else in a stylesheet is described, not ordered — the same position markdown
  and most config formats are in.
- **A `.vue` file is read as HTML.** Its `<script>` contributes uses; it
  contributes no definitions, so a component's own functions never appear in
  `defines` or `symbols`. `<style>` is not injected, deliberately.
- **An SFC's `<style>` is not parsed as CSS.** It is named as a region and left
  alone, because the injection contract is uses-only and a stylesheet is
  definitions.
- **`.scss` / `.less` stay unsupported.** The css grammar produces a pile of
  errors on both; a wrong tree is worse than no tree.
- **Two `.btn` rule sets in one file are one symbol.** A `@media` region does not
  scope, so a responsive override shares its base rule's identity — the same
  collision class already documented at `symbol_identity_key`.
