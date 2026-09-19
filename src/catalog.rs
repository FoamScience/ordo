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

/// The catalog grouped the way it is written: one entry per
/// `rulesets/catalog/*.toml`, in file order, holding that file's rule names.
///
/// A reviewer turns off "the C++ constructs", not nineteen names — and the
/// grouping is the file, so it stays true as the catalog grows without anyone
/// maintaining a second list.
pub fn sections() -> &'static [Section] {
    &parsed().2
}

/// One `rulesets/catalog/*.toml`, by its file stem.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Section {
    pub name: String,
    pub rules: Vec<String>,
}

/// Why the catalog is empty, when it is. `None` on the normal path.
pub fn problem() -> Option<&'static str> {
    parsed().1.as_deref()
}

/// The embedded file's shape: sections in file order, each with its rules.
#[derive(serde::Deserialize)]
struct Compiled {
    name: String,
    rules: Vec<Rule>,
}

type Parsed = (Vec<Rule>, Option<String>, Vec<Section>);

fn parsed() -> &'static Parsed {
    static PARSED: OnceLock<Parsed> = OnceLock::new();
    PARSED.get_or_init(
        || match serde_json::from_str::<Vec<Compiled>>(CATALOG_JSON) {
            Ok(files) => {
                let sections = files
                    .iter()
                    .map(|f| Section {
                        name: f.name.clone(),
                        rules: f.rules.iter().map(|r| r.name.clone()).collect(),
                    })
                    .collect();
                let rules = files.into_iter().flat_map(|f| f.rules).collect();
                (rules, None, sections)
            }
            Err(e) => (
                vec![],
                Some(format!(
                    "ordo bug: the built-in construct catalog did not parse ({e}); \
                 no construct advisories in this run"
                )),
                vec![],
            ),
        },
    )
}
