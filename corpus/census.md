# Census

`scripts/census.py`, 2026-11-04, on AMD Ryzen 7 7735U with Radeon Graphics (16 threads),
engine `target/release/ordo-engine` at `7929673` plus the working tree.
Newest 30 commits of each repo in `corpus/census.toml`; each engine run capped at
60 s and 4 GB of address space, 8 at a time.

**1489 of 1489 commits (100.00%) completed**, over
50 repositories.

| | |
|---|---|
| runtime per commit | p50 61 ms, p90 583 ms, p99 4898 ms, max 55342 ms |
| files | 7965, 0 degraded, 940 unsupported (11.8%) |
| hunks | 71893 |

| language | commits | completed |
|---|---|---|
| c | 148 | 148 |
| c++ | 150 | 150 |
| go | 150 | 150 |
| java | 144 | 144 |
| javascript | 149 | 149 |
| lua | 150 | 150 |
| python | 149 | 149 |
| rust | 150 | 150 |
| shell | 150 | 150 |
| typescript | 149 | 149 |
