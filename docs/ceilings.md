# Ceilings (by design)


`ordo` states its own limits rather than guessing past them:

- **Symbol resolution is approximate** — name match with an optional
  cross-file union, no full scope/type analysis. `cross_file` is a toggle.
- **`diff` input** reaches full semantics whenever full new content is
  derivable: `new` given, `old`+`diff` (applied), an added file, or a
  caller-asserted full-context patch (`full_context` / `--full-context`, e.g.
  `git diff -U100000`). A bare context-limited diff of a *modified* file stays
  positional and is flagged `degraded: true` (no silent guessing — a partial
  diff can't be reconstructed without truncating the file). See
  [`diff-input-design.md`](diff-input-design.md).
- **A template's format comes from its own path, not its destination** — one
  extension is stripped and what remains must name a format on its own.
  `.dvc/config.j2` is ini; a template kept somewhere else under a name its
  target never has (`templates/dvc-config.j2`) is jinja, because nothing in the
  path says otherwise.
- **A template is read as written, not as rendered** — a `.j2` is analyzed as
  the one document its source text spells out. A `{% for %}` that emits a key
  per host contributes that key once, under the literal `{{ … }}` it is named
  by; a `{% if %}`-guarded block sits at whatever indentation the source gives
  it, which in yaml is the branch's own nesting, not the enclosing key's.
- **`ssh_config` is not ini** — `~/.ssh/config` is `Host` blocks and
  space-separated directives, not sections and `key = value`, and no
  tree-sitter grammar for it is published to crates.io. It stays unsupported
  rather than being fed to the ini grammar, which reads the whole file as one
  error.
- **Unsupported languages** — a file whose extension has no tree-sitter
  grammar (`.gif`, `.ttf`, `.astro`, `.css`, …) gets no structural analysis at
  all and is flagged `unsupported: true` on its file entry. Distinct from
  `degraded`: `degraded` means a grammar exists but only a context-limited
  diff was available; `unsupported` means there is no grammar to begin with.
  Either, both, or neither can be true for a given file.
- **The engine has no filtering policy of its own** — it orders exactly the
  changes it is handed and has no opinion about which files belong in a
  review. *Path* filtering (globs, skipping generated/lock files) is entirely
  client-side: `ordo` has it, `ordo-engine order`/`ordo-engine review` deliberately do
  not. A caller sends the set it wants ordered.

  One deliberate exception: `options.only_comments` (`--only-comments`) *is*
  honoured by the engine, dropping non-comment hunks **before** grouping, the
  the one thing the engine drops — filtering the finished `Output`
  would leave `order`/`groups`/`edges`/`clusters` referring to hunks no longer
  in `files`. So the engine applies a selection the caller *states*; it never
  invents one.

  Note `ordo`'s `--only-comments` does NOT use the engine flag: it asks
  for every hunk and filters the view, so `:only-comments` can toggle back off
  with something to reveal.

  Because the client filters and the engine does not, `:audit` is what ties the
  two together: it charges every hunk not on screen to the thing that removed
  it — view filter, engine drop, or a file never sent at all — and says so
  outright when a hidden hunk matches no known reason.
- **Rationale heuristics** — rename detection is 1:1 per file (a file that
  renames *and* adds/removes other defs falls back to `adds`/`removes`);
  removals attach by old-line overlap (precise for isolated deletions).

