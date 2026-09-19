//! The built-in construct catalog, as data.
//!
//! P14 shipped 48 constructs as hand-written per-language tree-sitter walkers.
//! They answered the same question `Options.rules` answers — "does this hunk
//! introduce a shape worth flagging?" — through a second, parallel mechanism
//! that could not be scoped by path, switched off, read, or copied by a user
//! (tasks-9sj.23). Each construct the rule language can state faithfully now
//! lives in `rulesets/catalog/*.toml` and runs through `rules.rs`, tagged
//! `FindingSource::Catalog` so consumers still know who found it.
//!
//! The engine reads no files — that is the whole point of `ordo-engine order`
//! being a function of its arguments — so the TOML is compiled to JSON and
//! embedded. `catalog::generated` (in the `ordo` binary's tests, which is
//! where `toml` lives) regenerates and compares it, the same
//! recorded-artifact-plus-drift-test shape the docs blocks and goldens use.
use crate::model::Rule;
use std::sync::OnceLock;

const CATALOG_JSON: &str = include_str!("catalog.generated.json");

/// The catalog rules, parsed once.
///
/// A malformed `catalog.generated.json` is our bug and the drift test catches it long
/// before a release — but `run` is a library call, and taking the host process
/// down over our own build artifact is never the right trade. The catalog goes
/// empty and `problem()` says why, so a caller still gets their ordering.
pub fn rules() -> &'static [Rule] {
    &parsed().0
}

/// Why the catalog is empty, when it is. `None` on the normal path.
pub fn problem() -> Option<&'static str> {
    parsed().1.as_deref()
}

fn parsed() -> &'static (Vec<Rule>, Option<String>) {
    static PARSED: OnceLock<(Vec<Rule>, Option<String>)> = OnceLock::new();
    PARSED.get_or_init(|| match serde_json::from_str(CATALOG_JSON) {
        Ok(rules) => (rules, None),
        Err(e) => (
            vec![],
            Some(format!(
                "ordo bug: the built-in construct catalog did not parse ({e}); \
                 no construct advisories in this run"
            )),
        ),
    })
}
