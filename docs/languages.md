# Supported languages


<!-- ordo:begin langs -->
python, xonsh, javascript, rust, typescript, tsx, go, c, cpp, java, lua,
markdown, json, yaml, toml, ini, cmake, make, nix, bash, jinja, css, html,
svelte, gotmpl, erb
<!-- ordo:end langs -->

Adding one is usually a single registry entry in `src/lang.rs` plus its
grammar crate — no algorithm changes. Two shapes are exceptions:

**Markdown** — a heading has no identifier to name a def by, so a def is a
*section* (heading + content, nested by heading level) instead — `adds section
Usage`, `edits section Install`.

**Config formats** (json, yaml, toml) — a def is a key-value pair, named by its
key and nested through a dotted path, and each key is also a *member* of the key
above it, so the detail layer can say what changed inside a block:

```
docker-compose.yml:L3  edits image
  enclosing: services.web.image
  details: changes image in services.web
```

**ini** covers the config files shaped like it, matched by *filename* as well
as extension — `.gitconfig`, `.gitmodules`, `.git/config`, `.dvc/config`,
`.editorconfig`, `.npmrc`, `.hgrc`, `.flake8`, `.pylintrc`, `.coveragerc`, and
`.ini` / `.cfg` (`setup.cfg`, `tox.ini`, `pytest.ini`). A `.local` override
(`.dvc/config.local`) resolves as the file it overrides. A git subsection keeps
its quotes, because they are part of the name git gives it:

```
.gitconfig:L4  changes url
  enclosing: remote "origin".url
```

A toml `[table]` header is a container in its own right; a yaml sequence item
and a json array element carry no key, so they stay anonymous and their
contents nest under the nearest named key (a list entry's position is not part
of the path). Quoted and bare keys name the same thing (`"image"` == `image`).

Markdown is a rationale-quality improvement, not an ordering one: it has no
`uses` (link targets aren't parsed), so its hunks fall back to file order. So
does most config — with one exception. A **yaml anchor is a definition and its
alias is a use**, which is the one thing that lets a config hunk be *ordered*
rather than merely described:

```
database.yml:L4   adds adapter, base, used by dev below
database.yml:L11  edits dev
```

The merge key itself (`<<: *base`) is not a name anyone navigates by, so that
pair stays anonymous and only the alias in its value is read.

A multi-document yaml file (`---`) scopes each document by position, so two
k8s objects' top-level keys stay distinct — `document 1.spec.port` and
`document 2.spec.replicas` rather than two colliding `spec` paths. `document`
is the one container kind that *scopes*; every other region names only itself,
because `#ifdef X` is a fact about where a definition sits, not part of its
name. A single-document file is unaffected and keeps its bare key paths.

### cmake

Every cmake construct is a command, so what a node *is* lives in its identifier
rather than its node kind. `function` and `macro` define; `set` and `option`
name a variable; `include`, `find_package` and `add_subdirectory` are imports
naming what they pull in; every other command is transparent, so a `message()`
contributes no definition. `${VAR}` is a use, and a function's parameters are
bound so they can't be mistaken for one. `CMakeLists.txt` is matched by
filename — its `.txt` extension says nothing about it.

```
cmake/x.cmake:L1  adds helper, used by caller below
cmake/x.cmake:L6  uses helper, defined above
```

### make

A makefile already *is* a dependency graph, so ordo reads it as one: a rule is a
definition named by its target, and each prerequisite is a use of the target it
names. The reading order that falls out is the build order — variables, then the
rules nothing depends on, then the rules that depend on them.

```
Makefile:L2   adds SRCS, used by build below
Makefile:L9   changes build, build used by all above
Makefile:L6   changes all, used above
```

A special target (`.PHONY`, `.SUFFIXES`) names no recipe anyone navigates to, so
it stays anonymous — but the targets it lists are still read as uses of the real
rules, which is what a `.PHONY` line is. `include` names the makefiles it pulls
in. Matched by name (`Makefile`, `GNUmakefile`, `Makefile.am`) as well as by
`.mk`.

### nix

An attribute set is the language's main structure, so a `binding` is both a
definition and a member of the set above it — the same shape as the config
formats. A function is not a separate declaration (it is a lambda bound to an
attribute), so one node kind covers both. A dotted `meta.description = …` is
one name, a lambda's formals (`{ pkgs, lib, ... }:`) are bound rather than read
as references, and an `attrpath` never counts as a use — it is a name being
bound or selected, not a reference to something defined elsewhere.

nix spells an import as an ordinary application of a function named `import`,
so it is recognised by that name rather than by node kind. Because such an
import is nearly always bound (`overlay = import ./x.nix;`) the binding's name
leads the rationale — the import rows are still recorded, which is what an
`imports` glob in a rule matches on.

### bash

A shell command *is* a call, so `deploy prod` is a use of the function
`deploy` — which gives a script the same def→use ordering a code file gets.
Both spellings (`deploy() { … }` and `function deploy { … }`) share one node
kind. `source x.sh` and its POSIX form `. x.sh` are imports named by the script
they pull in, recognised by command name rather than node kind. `local`,
`readonly` and `declare` all wrap the same assignment node, so one entry covers
them. A positional parameter (`$1`) is never treated as a symbol.

```
deploy.sh:L2  adds import ./lib/common.sh
deploy.sh:L4  adds deploy, used by main below
deploy.sh:L9  uses deploy, defined above
```

Matched by `.sh`/`.bash` and by name for `.bashrc`, `.bash_profile`, `.profile`
and `.env` — a dotenv file is assignments, which is exactly what this grammar
reads, and `.env.local` resolves through the same variant strip as any other
config override.

### css

A rule set is a definition named by its **whole selector list, sigils kept** —
`.btn, .btn-primary`, `#nav a:hover`. That punctuation is the safety story, not
decoration: no code grammar emits an identifier starting with `.`, `#` or `--`,
so a css symbol is lexically incapable of colliding with a python function in
the cross-file union. A declaration is a member of its rule, `@media` and
`@supports` are regions (they are `#ifdef` in a different hat), and a selector
list is a *name* — never a set of references, so a stylesheet seeds no bare
`card` or `title` into the symbol table every other file is ordered against.

A **custom property is css's yaml anchor**: `--brand: #0af` defines a name and
`var(--brand)` uses it, which is the one thing that lets a css hunk be ordered
rather than merely described.

```
t.css:L3  adds --brand, used by .btn below
t.css:L8  uses --brand, defined above
            details: changes color in .btn
```

Deliberately **not** done: a class name in HTML is not treated as a use of the
selector that styles it. `class="btn tw-p-2 card"` is one un-tokenized string,
and names like `card`, `title`, `active` and `root` collide with real code
symbols — under a utility-class framework the false links would swamp the true
ones and make the ordering worse, not better. `.scss` and `.less` are not read
through this grammar either; they parse with errors, which is the same mistake
as feeding `ssh_config` to the ini grammar.

### html, and with it vue

Only an element carrying an **`id`** is a definition — that is the one handle a
stylesheet, a script or a fragment link addresses it by. Every other element
resolves to no name and stays transparent, so a page of anonymous `<div>`s
contributes nothing and a hunk inside one attributes to the nearest element
that *is* named.

```
page.html:L6  adds #foot
page.html:L3  edits #main
```

A **`.vue` single-file component needs no grammar of its own**: the html
grammar parses `<script setup lang="ts">`, `v-for`, `:key`, `@click`, `{{ }}`
and `<style module lang="scss">` with no error nodes, keeping the script and
style blocks as opaque text. (The published `tree-sitter-vue` pins tree-sitter
0.20 and could not be used regardless.)

An SFC's `<script>` block **is** injected — parsed with the js/ts grammar its
`lang` attribute names, and recorded as **uses only**, the same contract as a
markdown code fence. That is what lets a component join the def→use graph:

```
money.ts   adds formatPrice, used in Card.vue
Card.vue   uses formatPrice, defined in money.ts
```

Definitions are deliberately *not* taken from it. Recording them would mean a
sub-tree whose rows are not file rows, threaded through all ten of `extract`'s
parse entry points — eight of which are old-side collectors, so teaching only
`analyze` would make every function in every component read as newly added on
every commit. `<style>` is not injected at all: injection harvests every
identifier as a use, and a stylesheet's identifiers are its *definitions*, so it
would contribute nothing and would flood `uses` with exactly the `class_name`
leak the css selector guard exists to prevent.

Both blocks are still named the way a reviewer names them — `edits <script setup
lang="ts">`, `edits <style scoped>` — as regions rather than definitions.

**Svelte** has the same shape — `element`, `start_tag` and `attribute` are the
same kinds, so the id-naming path is reused verbatim — but it needs its own
grammar rather than riding on html's the way vue does: html cannot read a bare
`>` inside braces, and both `{#if n > 1}` and `on:click={() => pick()}` contain
one.

Its **block forms are named containers** — `{#if n > 1}`, `{#each items as it}`,
`{:else}`, plus `{#await}` and `{#key}` — written the way they appear in the
file. Regions, like `#ifdef`: they hold markup but declare nothing. The branch
is gated on the language because `if_statement` is a kind seven other shipped
grammars also produce. Markup inside a block attributes to that block rather
than to the enclosing element id, which is the tighter answer.

`{#snippet}` is the exception, and a real definition rather than a region:
`{#snippet row(x)}` declares a reusable named block and `{@render row(1)}` calls
it, which is the one def→use pair a component's markup has.

```
List.svelte:L1  adds row, used below
List.svelte:L7  uses row, defined above
```

### Templates

A `.j2` (also `.jinja`, `.jinja2`, `.tmpl`, `.tpl`) or a `.erb` / `.ejs` is
reviewed as **the format underneath it**. `values.yaml.j2` is yaml, `cfg.toml.j2` is toml, `app.py.j2` is
python — one `{% for %}` is enough to make a whole yaml document a parse error,
so the `{% … %}` statements and `{# … #}` comments are blanked out (space for
space, newlines kept) before the underlying grammar sees the file. Byte, row and
column offsets are unchanged, so every hunk still lines up with the file the
reviewer is looking at. An interpolation (`{{ … }}`, `<%= … %>`) is left in
place — it sits where a scalar does, and every format here already tolerates
one. Which node kinds are literal text and which are interpolations comes from
each templating grammar's own registry entry, so the pass belongs to no one
language:

```
templates/app.yml.j2:L3  adds port
  enclosing: services.{{s.name}}.port
```

The variables a template reads are recorded as **uses, never definitions** — a
template consumes what an inventory or a `group_vars` file sets, and defines
none of it. So the file that sets a variable sorts ahead of the template that
renders it:

```
group_vars/all.yml:L2      adds db_port, used in templates/app.yml.j2
templates/app.yml.j2:L3    adds port
```

Templating composes with the filename-matched formats above:
`.dvc/config.j2`, `.dvc/config.local.j2`, `.gitconfig.j2` and `setup.cfg.j2`
all resolve to ini. A hunk that touches *only* jinja is blank to the underlying
grammar, so it would read as "formatting only" — it is exempted from that, and
says what the statement reads instead (`uses prod` for an added `{% if prod %}`
guard).

**ERB / EJS** works the same way, with one honest difference: its `<% … %>`
bodies are a single opaque blob of ruby or javascript rather than parsed
identifiers, so an ERB template contributes no `uses` — and it cannot host a
format that has no grammar of its own. `config.yml.erb` is yaml;
`index.html.erb` stays `unsupported: true` rather than pretending to have been
read. That falls out of one flag on the grammar's entry, not a special case.

**Go templates, and with them Helm.** One pair of delimiters does both jobs
here — `{{ if … }}` is a statement and `{{ .Values.x }}` an interpolation — so
the two are told apart by node kind rather than by delimiter, which is exactly
what having a grammar buys over a scan. Helm is also the one exception to the
extension convention: a chart's templates carry *no* template extension at all,
so they are found by the directory Helm requires them to live in.

```
mychart/templates/deployment.yaml:L2  edits replicas
  enclosing: spec.replicas
mychart/templates/_helpers.tpl:L4     adds mychart.name
```

That directory rule is deliberately a loose heuristic: masking a file that
turns out to hold no template syntax blanks nothing and changes nothing, so a
`templates/` directory in a project that is not a chart costs exactly zero.
`{{ define "x" }}` and `{{ block "x" }}` are named blocks, so a `_helpers.tpl`
reads as structure rather than as text.

This is the one grammar ordo vendors rather than depends on — no crate
publishes a Go-template grammar for a current tree-sitter (the `gotmpl` /
`gotpl` crates are template *renderers*, which evaluate a template rather than
hand back a syntax tree). See
[`grammars/tree-sitter-go-template/`](grammars/tree-sitter-go-template/).

A template over a format that has *no* grammar (`nginx.conf.j2`,
`deploy.sh.j2`, a bare `foo.j2`) is parsed as jinja itself: `{% block x %}` and
`{% macro x() %}` are defs, and `{% include %}` / `{% extends %}` / `{% import %}`
are imports naming the template they pull in (`adds import tls.j2`). `{% for %}`
and `{% if %}` carry no name, so they stay transparent rather than contributing
an `<anonymous>` segment to a path.

