//! Readability, per definition: what a change did to the definitions it
//! touched, as measurements on the old and the new version side by side —
//! `schedule: nesting 2 → 5, lines 40 → 118`. Never one score: a reviewer
//! needs to know which measurement moved, and a sum hides that. Only a
//! definition the change touched is measured against itself, so debt it
//! already carried says nothing.

use crate::extract::RawHunk;
use std::collections::HashMap;

/// What one version of a callable definition measures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Metrics {
    /// deepest control flow inside it, counted from its own body
    pub nesting: usize,
    pub lines: usize,
    pub params: usize,
    /// `if`, loops, `match`/`switch`: the paths through it
    pub branches: usize,
    /// each branch weighted by how deep it sits, plus each run of `&&`/`||`
    pub cognitive: usize,
    /// percent of its local names one or two characters long
    pub short_names: usize,
}

type Metric = fn(&Metrics) -> usize;

/// Every measurement, by the name a report uses for it, with its unit.
const METRICS: &[(&str, &str, Metric)] = &[
    ("nesting", "", |m| m.nesting),
    ("cognitive", "", |m| m.cognitive),
    ("branches", "", |m| m.branches),
    ("params", "", |m| m.params),
    ("short names", "%", |m| m.short_names),
    ("lines", "", |m| m.lines),
];

/// One version of a definition: its qualified name, its 0-based rows.
pub struct Measured {
    pub name: String,
    pub rows: (usize, usize),
    pub metrics: Metrics,
}

/// A definition the change touched whose measurements moved.
#[derive(Debug, Clone)]
pub struct DefDelta {
    pub name: String,
    pub was: Metrics,
    pub now: Metrics,
}

impl DefDelta {
    /// how much `metric` grew, 0 when it held or shrank
    pub fn grew(&self, metric: Metric) -> usize {
        metric(&self.now).saturating_sub(metric(&self.was))
    }

    /// the measurements that grew, as `nesting 2 → 5`
    fn regressions(&self) -> Vec<String> {
        METRICS
            .iter()
            .filter(|(_, _, m)| self.grew(*m) > 0)
            .map(|(name, unit, m)| {
                format!("{name} {}{unit} → {}{unit}", m(&self.was), m(&self.now))
            })
            .collect()
    }

    /// Growth in length alone is what adding a feature looks like, not a
    /// definition getting harder to read.
    fn regressed(&self) -> bool {
        METRICS
            .iter()
            .any(|(name, _, m)| *name != "lines" && self.grew(*m) > 0)
    }
}

/// Each definition matched to itself across the two sides by qualified name,
/// when that name is unique on both, then handed to the first hunk that
/// touches either version — one hunk per definition, so a rule on it speaks
/// once. Returns, per hunk, the definitions whose measurements moved.
pub(crate) fn deltas(hunks: &[RawHunk], old: &[Measured], new: &[Measured]) -> Vec<Vec<DefDelta>> {
    let (was, now) = (by_unique_name(old), by_unique_name(new));
    let mut out: Vec<Vec<DefDelta>> = hunks.iter().map(|_| vec![]).collect();
    for n in new {
        let (Some(Some(_)), Some(Some(o))) = (now.get(n.name.as_str()), was.get(n.name.as_str()))
        else {
            continue;
        };
        if o.metrics == n.metrics {
            continue;
        }
        if let Some(i) = hunks.iter().position(|h| touches(h, o.rows, n.rows)) {
            out[i].push(DefDelta {
                name: n.name.clone(),
                was: o.metrics,
                now: n.metrics,
            });
        }
    }
    out
}

/// each name on one side, with its definition when it names only one
fn by_unique_name(side: &[Measured]) -> HashMap<&str, Option<&Measured>> {
    let mut seen: HashMap<&str, Option<&Measured>> = HashMap::new();
    for d in side {
        seen.entry(d.name.as_str())
            .and_modify(|v| *v = None)
            .or_insert(Some(d));
    }
    seen
}

/// whether `h` changed a row of the old version or of the new one
fn touches(h: &RawHunk, (os, oe): (usize, usize), (ns, ne): (usize, usize)) -> bool {
    let [a, b] = h.old_range;
    let old = a >= 1 && b >= a && a - 1 <= oe && os < b;
    let new = h.new_r0.is_some_and(|r0| r0 <= ne && ns <= h.new_r1);
    old || new
}

/// How many definitions the summary names before it counts the rest.
const WORST_SHOWN: usize = 3;

/// The definitions whose readability regressed most, worst first, as one
/// sentence — `None` when none did. Ranked by the measurements in `METRICS`
/// order, so a deeper nesting outranks a longer body.
pub(crate) fn worst<'a>(defs: impl Iterator<Item = (&'a str, &'a DefDelta)>) -> Option<String> {
    let mut defs: Vec<(&str, &DefDelta)> = defs.filter(|(_, d)| d.regressed()).collect();
    if defs.is_empty() {
        return None;
    }
    let key = |d: &DefDelta| {
        METRICS
            .iter()
            .map(|(_, _, m)| d.grew(*m))
            .collect::<Vec<_>>()
    };
    defs.sort_by(|(pa, a), (pb, b)| {
        key(b)
            .cmp(&key(a))
            .then_with(|| (pa, &a.name).cmp(&(pb, &b.name)))
    });
    let shown: Vec<String> = defs
        .iter()
        .take(WORST_SHOWN)
        .map(|(path, d)| format!("{} ({path}): {}", d.name, d.regressions().join(", ")))
        .collect();
    let more = match defs.len().saturating_sub(WORST_SHOWN) {
        0 => String::new(),
        n => format!("; and {n} more"),
    };
    Some(format!(
        "Readability regressed most in {}{more}.",
        shown.join("; ")
    ))
}
