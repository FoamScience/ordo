# Reviewer TUI (`ordo`)


An interactive terminal reviewer — a first-party *client* of the engine, kept
out of the pure default build behind the `tui` feature:

```sh
cargo run --bin ordo -- <rev> [<glob>...]   # rev defaults to HEAD
ordo help [<topic>]                         # this page, and the others, offline
```

`ordo help` lists the topics; `ordo help <topic>` prints one, paged through
`$PAGER` when there is a terminal to page for. The pages are compiled into the
binary, so they can never describe a different version than the one running.

It owns git (shells out for a commit's blobs), calls `ordo::run`, and renders
the change **in comprehension order**: a reading-order list (advisories `⚠`,
noise dimmed, reviewed `✓`) beside a detail pane with tree-sitter
syntax-highlighted, Neovim-style diff rendering, over the rationale, notes,
def→use edges and advisory ladders. The engine never learns what git is.

Changed lines are refined the way Neovim's `DiffText` refines `DiffChange`: a
removed line is paired with the added line it became, and only the differing
part carries the strong tint —

```diff
- fn content_matches(info: &AgentInfo, node: &AgentNode) -> bool {
+ fn content_matches(info: &AgentInfo, node: &AgentNode, label: Option<&str>) -> bool {
                                                       └── only this is highlighted
```

The unit of comparison is the tree-sitter **leaf node**, never a character or a
whitespace-split word, so `AgentNode` → `AgentNodeRef` reads as one identifier
replaced rather than a three-character suffix appended. Lines with too little in
common are left unrefined and render whole, so a rewrite is never dressed up as
a small edit. The algorithm is `ordo::refine`, a public library module — a
leaf-level LCS, deliberately not a full tree alignment (difftastic's
Dijkstra-over-graphs, or `syndiff`): those buy accuracy on moved and
restructured code at a cost this does not need to pay to say "an argument was
added".

<details>
<summary><b>Revision syntax, path filters, GitButler support</b></summary>

`<rev>` is any git commit-ish, a commit range (`main..branch`, or
`main...branch` to diff from the merge base — an omitted side means `HEAD`), or
`zz` for the uncommitted area. `<base>..zz` (or `<base>...zz` for the merge
base of `<base>` and `HEAD`) reviews everything done on a branch including
what's not yet committed. `zz` on the left (`zz..main`) is meaningless and
rejected.

On a **GitButler**-managed repo `<rev>` also takes the CLI IDs `but status`
prints — the workspace is read once from `but --json status`, and the repo is
otherwise driven by plain git:

| `<rev>` | reviews |
| --- | --- |
| a branch ID (`at`) or name (`feat/multi-session`) | that branch's own commits, as a range |
| a commit ID (`lzm`), or a change-ID / commit-ID prefix | that commit |
| `zz` | the uncommitted area, including changes assigned to a stack |

A branch label wins over git's reading of the same name, where a branch is
only its tip commit — reviewing a branch means reviewing its commits.

Generated and lock files (`Cargo.lock`, `package-lock.json`, `vendor/`,
`node_modules/`, `.min.js`, …) are dropped before their blobs are read, and so
is anything the repo's own `.gitattributes` declares — `linguist-generated` or
an explicit `-diff`. `--all` keeps everything (the engine still flags known
paths `noise`, so they render dimmed).

```sh
ordo main...feature 'src/*' '*.rs'          # the branch, Rust sources only
ordo zz --all                               # everything uncommitted, lock files included
ordo HEAD 'src/*' '!src/generated/*'        # src/, minus a generated subtree
ordo HEAD '!tests/*'                        # everything except tests/
```

A glob prefixed `!` is negative and excludes a path that matches it; with only
negative globs given, everything except those is kept. `\!literal` escapes a
leading bang. Matching is order-independent, deliberately unlike `.gitignore`.

</details>

<details>
<summary><b>Keybindings (<code>--keys vim|vscode</code>) and command bar</b></summary>

Its three panes — reading order, code, why — take focus one at a time (the
focused one is bordered in cyan); motion keys act on the focused pane, paging
always drives the code pane.

<!-- ordo:begin keys -->
| | `vim` (default) | `vscode` |
| --- | --- | --- |
| **general** | | |
| quit (or dismiss an open popup or search) | `q`, `Esc` | `C-q`, `Esc` |
| open the command bar (`:help` lists every command) | `:` | `C-P` |
| open the command bar pre-filled with `goto ` |  | `C-p` |
| **navigation** | | |
| next item / move cursor down | `j`, `Down` | `Down` |
| previous item / move cursor up | `k`, `Up` | `Up` |
| jump to the first item / top | `gg` |  |
| jump to the last item / bottom | `G` |  |
| page down | `Space`, `f`, `C-f`, `PageDown` | `PageDown` |
| page up | `C-b`, `PageUp` | `PageUp` |
| half-page down | `C-d` |  |
| half-page up | `C-u` |  |
| move the code cursor left | `h` | `Left` |
| move the code cursor right | `l` | `Right` |
| move the code cursor to the next word | `w` | `C-Right` |
| move the code cursor to the previous word | `b` | `C-Left` |
| move the code cursor to the end of the word | `e` |  |
| move to line start / pane top | `0` | `Home` |
| move to line end / pane bottom | `$` | `End` |
| jump to the previous blank line | `{` |  |
| jump to the next blank line | `}` |  |
| scroll the code pane (or an open popup) left | `zh` | `S-Left` |
| scroll the code pane (or an open popup) right | `zl` | `S-Right` |
| why pane: jump to the current dep line's target hunk | `gd`, `Enter` | `C-Enter` |
| jump back to the position before the last dep jump | `C-o` | `A-Left` |
| **panes** | | |
| focus the next pane | `C-w C-w`, `C-w w` | `F6` |
| focus the previous pane | `C-w W`, `C-w p` | `S-F6` |
| focus the reading-order pane | `C-w h`, `C-w Left` | `C-1` |
| focus the code pane | `C-w l`, `C-w k`, `C-w Right`, `C-w Up` | `C-2` |
| focus the why pane | `C-w j`, `C-w Down` | `C-3` |
| **search** | | |
| open the text-search prompt | `/` | `C-f` |
| next occurrence of the symbol under the cursor | `*` | `C-F12` |
| previous occurrence of the symbol under the cursor | `#` | `C-S-F12` |
| cycle to the next match | `n` | `F3` |
| cycle to the previous match | `N` | `S-F3` |
| **review** | | |
| toggle reviewed on the selected hunk | `x` | `Space`, `Enter` |
| fold/unfold the selected hunk's group | `za` | `C-k C-l` |
| unfold the selected hunk's group | `zo` |  |
| fold the selected hunk's group | `zc` |  |
| unfold every group | `zR` | `C-k C-j` |
| fold every group | `zM` | `C-k C-0` |
| **editor** | | |
| code pane: show the symbol under the cursor and its history · why pane: preview the current dep line's target | `K` | `F12` |
| open the selected hunk's file in $VISUAL/$EDITOR | `ge` | `C-o` |
| **help** | | |
| show this keybinding help | `?` | `F1` |
<!-- ordo:end keys -->

The command bar (`:` in vim, `Ctrl+Shift+P` in vscode) turns launch-time
choices into live controls, with tab-completion over command names and each
command's own arguments:

<!-- ordo:begin commands -->
| command | does |
| --- | --- |
| `:only-comments` | toggle showing only comment/docstring hunks |
| `:all` | toggle showing generated/formatting-noise hunks |
| `:rules` | where the active rules came from, and what was replaced or disabled |
| `:filter <glob>` | narrow the review to paths matching &lt;glob&gt;; no argument clears it |
| `:keys <preset>` | swap the keymap live (vim, vscode) |
| `:theme <name>` | swap the palette live (:theme with no name lists them) |
| `:strategy <name>` | re-order the review (comprehension, defs-first, file) |
| `:group` | toggle group-reason headers in the reading-order list |
| `:rule` | draft a rule matching the selected hunk's shape |
| `:delta` | what changed since this review was last opened |
| `:note [text]` | anchor a note to the selected hunk's symbol; no text clears it |
| `:mode [ledger|hunks]` | list by symbol (default) or by hunk; no argument toggles |
| `:goto <path>` | select the first hunk of &lt;path&gt;, focus the code pane |
| `:e <rev>` | review a different revision, without restarting |
| `:audit` | account for every hunk and file not on screen, and why |
| `:quickfix` | export the reading order to a vim quickfix list and open it (aliases: :qf, :vim-qfl) |
| `:qf` | alias for :quickfix |
| `:vim-qfl` | alias for :quickfix |
| `:q` | quit |
| `:help` | list these commands |
<!-- ordo:end commands -->

Reviewed hunks persist to
`${XDG_CACHE_HOME:-$HOME/.cache}/ordo/reviewed/<repo>.json`, keyed by
revision + hunk identity + hunk content — a mark drops itself the moment the
hunk's content changes underneath it. Marks older than 90 days are pruned
automatically. The file records only opaque hashes, never a path, symbol name
or source text.

## Themes

`--theme <name>` (also `$ORDO_TUI_THEME`, default `dark`); `:theme` lists
them and swaps live. `dark` and `light` keep the **terminal's own**
foreground palette and only tint the diff backgrounds — the default, because
it matches the rest of your setup for free. Every other theme is truecolor:
every colour is named by the theme, so the reviewer matches your editor
rather than your shell.

<!-- ordo:begin themes -->
`dark`, `light`, `catppuccin-mocha`, `catppuccin-macchiato`,
`catppuccin-frappe`, `catppuccin-latte`, `tokyonight-night`,
`tokyonight-storm`, `tokyonight-moon`, `tokyonight-day`, `gruvbox-dark`,
`gruvbox-light`, `nord`, `dracula`, `solarized-dark`, `solarized-light`
<!-- ordo:end themes -->

No theme paints a window background, so terminal transparency and blur survive.
What a theme *does* assume is a terminal background of matching lightness —
which is why the choice is an explicit flag rather than a detection (OSC 11
background queries aren't reliably supported).

Every role is overridable, on top of any theme:

```toml
[theme]
name = "catppuccin-mocha"
border-focus = "#f5c2e7"    # the focused pane's border
syntax-keyword = "#f38ba8"
add-bg = "#1e3a24"          # the quiet tint on an added line
```

Every role a `[theme]` table may set — each one the name of a field the
program actually reads back, so `--init-config` prints these same names:

<!-- ordo:begin theme-roles -->
`fg`, `dim`, `border`, `border-focus`, `accent`, `category`, `mark`,
`reviewed`, `warn`, `add-fg`, `del-fg`, `add-bg`, `del-bg`, `add-strong-bg`,
`del-strong-bg`, `select-bg`, `match-bg`, `match-current-bg`,
`syntax-comment`, `syntax-keyword`, `syntax-string`, `syntax-number`,
`syntax-function`, `syntax-type`, `syntax-property`, `syntax-operator`,
`syntax-variable`, `syntax-builtin`, `syntax-parameter`, `syntax-attribute`
<!-- ordo:end theme-roles -->

A theme colours twelve *syntax roles* rather than the twenty-six tree-sitter
capture names mapped onto them, so a new grammar's captures never mean touching
every theme.

`ordo --init-config` writes a starting config to
`${XDG_CONFIG_HOME:-~/.config}/ordo/tui.toml` (`--force` to overwrite): every
binding and every colour of the current preset and theme, at its real value,
commented out. It is generated from the same tables the program reads, so it
can't drift from what ordo accepts — a test uncomments the whole file and
checks it parses cleanly and changes nothing.

Keys are configurable in the same file, on top of whichever preset is in use:

```toml
preset = "vim"            # the preset to start from (--keys still wins)

[binds]
"C-n" = "next"            # add or replace a binding
"g d" = "jump-to-edge"    # a chord: prefix, space, key
"x" = "none"              # remove a binding
```

Action names are the ones `?`/`F1` lists. A line that names a key or an action
that doesn't exist is reported with its line number and skipped, so a typo costs
one binding rather than the session.

</details>

