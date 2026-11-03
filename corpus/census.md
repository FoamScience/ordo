# Census

`scripts/census.py`, 2026-11-03, on AMD Ryzen 7 7735U with Radeon Graphics (16 threads),
engine `target/release/ordo-engine` at `c03b53b` plus the working tree.
Newest 30 commits of each repo in `corpus/census.toml`; each engine run capped at
60 s and 4 GB of address space, 8 at a time.

**1485 of 1489 commits (99.73%) completed**, over
50 repositories.

| | |
|---|---|
| runtime per commit | p50 124 ms, p90 1210 ms, p99 7378 ms, max 57574 ms |
| files | 6617, 0 degraded, 669 unsupported (10.1%) |
| hunks | 42905 |

| language | commits | completed |
|---|---|---|
| c | 148 | 147 |
| c++ | 150 | 150 |
| go | 150 | 150 |
| java | 144 | 144 |
| javascript | 149 | 149 |
| lua | 150 | 150 |
| python | 149 | 149 |
| rust | 150 | 149 |
| shell | 150 | 150 |
| typescript | 149 | 147 |

## Did not complete

| repo | commit | status | ms | detail |
|---|---|---|---|---|
| rtk-ai__rtk | a3718b878de7 | timeout | 60000 |  |
| JustVugg__colibri | 704a3f68184b | timeout | 60000 |  |
| yangshun__tech-interview-handbook | 135af1325f58 | timeout | 60000 |  |
| yangshun__tech-interview-handbook | a5c9b684ae1f | timeout | 60000 |  |
