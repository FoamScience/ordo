# ordo-diff

Review a code change in reading order: definitions before their uses.

```sh
npm install -g ordo-diff
ordo main...feat        # review a branch in the browser
```

The package installs two commands, `ordo` (the reviewer) and `ordo-engine`
(the JSON engine), and a small API over the engine:

```js
const { order, review } = require("ordo-diff");
const out = review(require("child_process").execSync("git diff HEAD~1").toString());
```

`order(input)` takes the changeset described by `schema/v2.json`; `review(patch)`
takes a unified diff. Both return the engine's schema v2 output.

The binaries come from a platform package (`ordo-diff-linux-x64` and so on)
installed as an optional dependency. Set `ORDO_BIN_DIR` to use binaries from
elsewhere.

See <https://github.com/FoamScience/ordo> for the full documentation.
