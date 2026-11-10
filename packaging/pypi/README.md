# ordo-diff

Review a code change in reading order: definitions before their uses.

```sh
pip install ordo-diff
ordo main...feat        # review a branch in the browser
```

The wheel ships two commands, `ordo` (the reviewer) and `ordo-engine` (the JSON
engine), and a small API over the engine:

```python
import subprocess
from ordo_diff import order, review

out = review(subprocess.run(["git", "diff", "HEAD~1"], capture_output=True, text=True).stdout)
```

`order(input)` takes the changeset described by `schema/v2.json`; `review(patch)`
takes a unified diff. Both return the engine's schema v2 output. Set
`ORDO_BIN_DIR` to use binaries from elsewhere.

See <https://github.com/FoamScience/ordo> for the full documentation.
