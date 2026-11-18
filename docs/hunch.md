# Hunch

hunch is a small model compiled into ordo that guesses, for every hunk, what a
careful reviewer would decide. It is off by default.

```sh
ordo --hunch HEAD                                  # the reviewer
ordo --json --hunch HEAD | ordo-engine order --json  # the engine, same guesses
```

Engine callers set `options.hunch: true` and may pass `history` (see
[cli.md](cli.md)).

## What it guesses

| Guess | Question | Effect |
| --- | --- | --- |
| `noise` | Can a reviewer skip this hunk? | Moves the noise flag when confident |
| `attention` | How much care does it need (1–3, rescaled to 0–1)? | Reads the most-needed hunks first where the def→use order allows |
| `faithful` | Is ordo's rationale free of false or misleading claims? | None yet: shown, changes nothing |

Every guess comes with the three features that moved it most, signed, in
log-odds. The web reviewer shows them in the why pane; `--json` carries them as
`top`, `attention_top` and `faithful_top`.

## What it may change

- **Noise.** A hunk the engine dimmed comes back when P(skip) < 0.05
  (`Hunch: shown`); a hunk the engine showed is dimmed when P(skip) ≥ 0.9
  (`Hunch: hidden`, `:all` shows it). `--json` marks these `changed: "unhid"`
  or `"hid"`.
- **Order.** Among groups the def→use order has already freed, the
  most-attended group is read first. It replaces file position as the tiebreak
  and nothing else.

It never moves a use ahead of its definition, never overrides a rule's
`priority` or `docs_last`, and never changes groups, edges or rationales.
Without `--hunch` the output is byte-identical to ordo without hunch.

## How well it works

Measured on held-out data: a gold set of 196 hunks from 12 repositories, and
leave-one-repository-out on the rest, so each repository is scored by a model
that never saw it (numbers as of November 2026).

- **Noise policy vs the engine's own flag.** Gold: 3 wrong hides and 13 missed
  skips, against 5 and 13. Held-out repositories: 6 and 111, against 9 and 111.
  No repository gets more wrong hides.
- **Attention vs file order.** For two hunks of one commit rated differently,
  hunch puts the more important one first 82% of the time (gold and held-out),
  against 50–56% for file position and 66–68% for "biggest hunk first".
- **Faithful.** It finds only 14% of unfaithful rationales at the precision
  required, below its 20% bar, so it stays a shadow.

A guess only takes effect after passing a gate set before its results were
seen; the gates are in `scripts/hunch-train.py` and the numbers in
[`corpus/hunch-report.json`](../corpus/hunch-report.json).

## What it learned from

Labels come from an LLM judge (Claude Opus) answering fixed questions about
each hunk, on about 7,000 hunks from 12 open-source repositories. A second
model (Claude Fable) audited 48 gold hunks independently and agreed with the
judge on 98% of skip decisions. These are model judgements, not measured
reviewer behaviour: hunch imitates a careful reviewer's call, and has not been
tested against real review outcomes.

## Features it reads

Per hunk: its category, size, noise and comment flags, what it defines and
uses, the edges in and out, findings by level, test path, language, the
readability change of the definitions it touches, and the rationale's
template. From git: how many commits touched the file in the 90 days before
the change and how many of those were fixes, following renames and counting
only the change's ancestors. A diff on stdin, or a shallow clone cut inside
that window, has no history and is marked so.

## Retraining

```sh
cargo build --release
ORDO_CORPUS=~/.cache/ordo-corpus scripts/hunch-train.py \
  --extra ~/.cache/ordo-corpus/judged.should_skip.jsonl \
  --extra-faithful ~/.cache/ordo-corpus/judged.faithful.jsonl
```

Two tests stop a stale model from shipping: one compares the feature schema's
hash, the other the features computed on fixed sample diffs. Either failure
prints the exact command to run.
