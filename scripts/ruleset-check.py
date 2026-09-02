#!/usr/bin/env python3
"""Run a ruleset file against a sample through `ordo order --json`.

Usage: scripts/ruleset-check.py rulesets/<set>.toml rulesets/samples/<sample> [ordo-binary]

Converts the kebab-case rules file into the engine's JSON `options.rules`,
feeds the sample as a wholly-new file, and reports which rules fired. Exit
code 1 when the engine reports a problem or a rule never fires — a rule that
never fires looks exactly like a convention nobody breaks.
"""
import json, os, subprocess, sys, tomllib

if len(sys.argv) < 3:
    sys.exit(__doc__)
rules_path, sample_path = sys.argv[1], sys.argv[2]
ordo = sys.argv[3] if len(sys.argv) > 3 else os.environ.get("ORDO_BIN", "target/debug/ordo")

ACTIONS = ("name", "note", "warn", "noise", "priority")
rules = []
for r in tomllib.load(open(rules_path, "rb")).get("rule", []):
    when = {k.replace("-", "_"): v for k, v in r.items() if k not in ACTIONS}
    if "noise_when" in when:
        when["noise"] = when.pop("noise_when")
    if "query_file" in when:
        when["query"] = open(os.path.join(os.path.dirname(rules_path), when.pop("query_file"))).read()
    rules.append({"name": r["name"], "when": when, **{k: r[k] for k in ACTIONS[1:] if k in r}})

sample = open(sample_path).read()
inp = json.dumps({"changes": [{"path": os.path.basename(sample_path), "old": "", "new": sample}], "options": {"rules": rules}})
res = subprocess.run([ordo, "order", "--json"], input=inp, capture_output=True, text=True)
if res.returncode:
    sys.exit(res.stderr)
out = json.loads(res.stdout)
fired = {x["rule"] for f in out["files"] for h in f["hunks"] for x in (h.get("rules") or [])}
# a file-size limit cannot be exercised by a sample short enough to read
names = [r["name"] for r in rules if "max_file_lines" not in r["when"]]
silent = [n for n in names if n not in fired]
print(f"{rules_path}: {len(rules)} rules, {len(fired)} fired on {sample_path}")
for p in out.get("problems") or []:
    print("  problem:", p)
for n in silent:
    print("  never fired:", n)
sys.exit(1 if out.get("problems") or silent else 0)
