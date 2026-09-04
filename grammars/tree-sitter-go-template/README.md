# tree-sitter-go-template (in-tree)

Go template / Helm grammar, vendored because no tree-sitter grammar for it is
published to crates.io — the `gotmpl` / `gotpl` / `go-template` crates are
template *renderers*, which evaluate a template rather than hand back a syntax
tree with byte ranges. `extract::mask_template` needs the ranges.

- **Upstream**: https://github.com/ngalaiko/tree-sitter-go-template (`master`)
- **License**: MIT (see `LICENSE`) — same as ordo
- **ABI**: `LANGUAGE_VERSION 15`
- **Contents**: only the generated `src/parser.c` and the `src/tree_sitter`
  headers it includes, plus `grammar.js` for provenance. There is no external
  scanner. Compiled by `/build.rs`.

To update: replace `src/` and `grammar.js` from a fresh upstream checkout and
re-run the test suite. Nothing here is edited by hand.
