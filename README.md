# ordo

<p>
  <a href="https://github.com/FoamScience/ordo/actions/workflows/ci.yml"><img src="https://github.com/FoamScience/ordo/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
  <a href="https://crates.io/crates/ordo-diff"><img src="https://img.shields.io/crates/v/ordo-diff" alt="ordo-diff on crates.io"></a>
  <!-- ordo:begin langs-badge -->
  <img src="https://img.shields.io/badge/languages-29-5fd4c0" alt="29 supported languages">
  <!-- ordo:end langs-badge -->
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
</p>

Review a code change in reading order: definitions before their uses.

<picture>
  <source srcset="assets/readme/ordo.avif" type="image/avif">
  <img src="assets/readme/reviewer-dark.png" alt="A 51-second tour of ordo: a cobra commit whose call to postRun comes before its definition in git order is reordered so the definition is read first, with a def-to-use arc and one plain sentence per hunk; on a Vue commit the import is dimmed, --hunch moves the canSetRef logic ahead of 14 test hunks by attention, the verdict shows readability going from cognitive 29 to 37, and a catalog note flags an any type; then the real reviewer in a browser tab, the same order as JSON from ordo-engine, and the install command.">
</picture>

- **Reading order.** Each definition comes before the code that uses it, across
  files, parsed with tree-sitter ([languages](docs/languages.md)).
- **A reason per hunk.** One plain sentence: `adds parse_cfg, used in
  main.py`, `renames foo → bar`.
- **Skip the noise.** Imports, generated and lock files, and pure formatting
  are dimmed.
- **A verdict on the change.** Feature, likely fix or refactor; whether
  behaviour is preserved; which function got harder to read (nesting 2 → 3).
- **Findings on what changed.** Built-in construct checks and your own rules,
  flagged only where this change introduces them.
- **[Hunch](docs/hunch.md)** (opt-in). A trained guess of what a reviewer can
  skip and what needs care first.
- **Local and scriptable.** Runs on your machine; `ordo-engine` gives the same
  analysis as JSON or as compact text for an LLM.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/FoamScience/ordo/main/install.sh | sh
# or
cargo install ordo-diff --features web
```

Prebuilt binaries for Linux, macOS and Windows are on the
[releases page](https://github.com/FoamScience/ordo/releases).

## Use

```sh
ordo                       # review the last commit in a browser tab
ordo zz                    # uncommitted work
ordo main...feat           # a branch
git diff | ordo-engine review   # the same analysis as JSON, no git needed
```

`j` next hunk, `]f` next finding, `x` mark read, `:help` for commands.

## Docs

- [Reviewer](docs/web.md): revisions, keys, commands, themes
- [Hunch](docs/hunch.md): the opt-in trained guess
- [Engine and JSON schema](docs/cli.md)
- [What the rationales and findings mean](docs/reviewing.md)
- [Your own rules](docs/rules.md)
- [Supported languages](docs/languages.md)
- [What ordo does not do](docs/ceilings.md)
- [Architecture](docs/architecture.md)

Pre-1.0; see the [changelog](CHANGELOG.md) before upgrading.

## License

[MIT](LICENSE)
