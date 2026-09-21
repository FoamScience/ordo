// ------------------------------------------------------------------- sarif

use super::*;

/// One analyzer finding, placed at a file and line.
///
/// Ordo does not detect these — it orders them. A SARIF file is what semgrep,
/// CodeQL, clippy, ruff, eslint, shellcheck and gosec all already emit, so one
/// reader puts every analyzer a team runs into the reading order, beside the
/// hunk the reviewer is standing in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Finding {
    /// the analyzer that reported it (`runs[].tool.driver.name`)
    pub(super) tool: String,
    /// `result.ruleId`, the analyzer's own name for the check
    pub(super) rule: String,
    /// `Warn` for error/warning, `Note` for note/none, so an analyzer result
    /// presses exactly as hard as a rule hit saying the same thing
    pub(super) level: ordo::model::Level,
    pub(super) message: String,
    pub(super) path: String,
    /// 1-based, as SARIF writes it and as hunk ranges are kept
    pub(super) line: usize,
}

/// Every finding in the `--sarif` files given, in the order the paths were.
///
/// A file that cannot be read is reported through `note_command_failure` and
/// contributes nothing: an unreadable analyzer report must not stop a review,
/// but it must not vanish either — `:audit` and the post-run notes say it was
/// asked for and did not arrive.
pub(super) fn sarif_findings(paths: &[String]) -> Vec<Finding> {
    paths
        .iter()
        .flat_map(|p| match std::fs::read_to_string(p) {
            Ok(text) => parse_sarif(&text),
            Err(e) => {
                note_command_failure("sarif", &[p.as_str()], &e.to_string());
                vec![]
            }
        })
        .collect()
}

/// Read `runs[].results[]` out of a SARIF document.
///
/// Deliberately a traversal of `serde_json::Value` rather than a typed schema:
/// SARIF 2.1.0 is a very large specification and this needs six fields of it.
/// Anything malformed is skipped rather than failing the run — a review must
/// not be blocked because one analyzer wrote something unexpected.
pub(super) fn parse_sarif(text: &str) -> Vec<Finding> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else {
        return vec![];
    };
    let mut out = vec![];
    for run in doc["runs"].as_array().into_iter().flatten() {
        let tool = run["tool"]["driver"]["name"]
            .as_str()
            .unwrap_or("sarif")
            .to_string();
        for r in run["results"].as_array().into_iter().flatten() {
            let level = match r["level"].as_str().unwrap_or("warning") {
                "error" | "warning" => ordo::model::Level::Warn,
                _ => ordo::model::Level::Note,
            };
            let rule = r["ruleId"].as_str().unwrap_or("").to_string();
            let message = r["message"]["text"].as_str().unwrap_or("").to_string();
            if message.is_empty() {
                continue;
            }
            // a result may carry several locations; each is its own finding,
            // because each is somewhere a reviewer might be standing
            for loc in r["locations"].as_array().into_iter().flatten() {
                let phys = &loc["physicalLocation"];
                let Some(uri) = phys["artifactLocation"]["uri"].as_str() else {
                    continue;
                };
                let Some(line) = phys["region"]["startLine"].as_u64() else {
                    continue;
                };
                out.push(Finding {
                    tool: tool.clone(),
                    rule: rule.clone(),
                    level,
                    message: message.clone(),
                    // SARIF uris are often `file:///abs` or repo-relative;
                    // both are normalised to what git reports for a path
                    path: normalise_uri(uri),
                    line: line as usize,
                });
            }
        }
    }
    out
}

/// A SARIF `artifactLocation.uri` as a repo-relative path. Handles the
/// `file://` form and a leading `./`; an absolute path is left alone here and
/// matched by suffix when the finding is placed.
pub(super) fn normalise_uri(uri: &str) -> String {
    let p = uri.strip_prefix("file://").unwrap_or(uri);
    p.strip_prefix("./").unwrap_or(p).to_string()
}

/// Attach each finding to the hunk whose new-side range covers its line.
///
/// Returns how many could not be placed. Those are not dropped quietly: a
/// finding on a line this change did not touch is the normal case (the file is
/// full of code the diff never reached), and `:audit` reports the count for the
/// same reason it reports every other hunk that never made the screen.
pub(super) fn place_findings(items: &mut [Item], findings: &[Finding]) -> usize {
    let mut unplaced = 0;
    for f in findings {
        let hit = items.iter_mut().find(|it| {
            path_matches(&it.path, &f.path)
                && it.new_range[0] <= f.line
                && f.line <= it.new_range[1]
        });
        match hit {
            Some(it) => {
                // the row mark is decided in `build_items`, before findings
                // exist; a warning-level finding earns the same ⚠ a warn rule
                // or an advisory does, or the reviewer has to open the hunk to
                // discover there is anything to see
                if f.level != ordo::model::Level::Note {
                    it.mark = "⚠ ".to_string();
                }
                it.findings.push(ordo::model::Finding {
                    source: ordo::model::FindingSource::Analyzer,
                    name: if f.rule.is_empty() {
                        f.tool.clone()
                    } else {
                        format!("{} {}", f.tool, f.rule)
                    },
                    message: f.message.clone(),
                    level: f.level,
                });
            }
            None => unplaced += 1,
        }
    }
    unplaced
}

/// Whether a SARIF uri names the same file git called `path`. An analyzer run
/// from the repo root writes the same relative path; one run elsewhere writes
/// an absolute one, which matches by suffix on a path boundary.
pub(super) fn path_matches(path: &str, uri: &str) -> bool {
    if path == uri {
        return true;
    }
    uri.strip_suffix(path)
        .is_some_and(|head| head.is_empty() || head.ends_with('/'))
}

// ---------------------------------------------------------------- coverage

/// Which lines of a file a test run executed, from an lcov tracefile.
///
/// Only `DA:` records are read: lcov emits one per *executable* line, so a
/// blank line, a comment or a declaration never counts against a hunk. That is
/// the difference between "12 of 14 added lines are executed by no test" and a
/// number inflated by every brace in the diff.
#[derive(Default)]
pub(super) struct Coverage {
    /// path as the tracefile wrote it -> (line -> times executed)
    pub(super) files: HashMap<String, HashMap<usize, u64>>,
}

/// Parse an lcov tracefile. Records outside `SF:`/`DA:`/`end_of_record` are
/// ignored — branch and function coverage say nothing about whether a changed
/// line ran. A malformed line is skipped rather than failing the review.
pub(super) fn parse_lcov(text: &str) -> Coverage {
    let mut cov = Coverage::default();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(path) = line.strip_prefix("SF:") {
            cur = Some(normalise_uri(path));
        } else if line == "end_of_record" {
            cur = None;
        } else if let Some(rest) = line.strip_prefix("DA:") {
            let Some(path) = cur.as_ref() else { continue };
            let mut parts = rest.split(',');
            let (Some(Ok(n)), Some(Ok(hits))) = (
                parts.next().map(str::parse::<usize>),
                parts.next().map(str::parse::<u64>),
            ) else {
                continue;
            };
            // a line can appear more than once (several tracefiles merged, or
            // one line owned by several branches); the highest count wins,
            // because executed once anywhere is executed
            let e = cov.files.entry(path.clone()).or_default().entry(n);
            let slot = e.or_insert(0);
            *slot = (*slot).max(hits);
        }
    }
    cov
}

/// How much of what each hunk changed was actually executed.
///
/// Returns the number of tracefile entries that matched no reviewed file —
/// the normal case, since a tracefile covers the whole project and a change
/// touches a handful of it. Counted rather than dropped, for the reason
/// `:audit` counts everything else.
pub(super) fn place_coverage(items: &mut [Item], cov: &Coverage) -> usize {
    let mut unmatched = 0;
    for (path, lines) in &cov.files {
        let mut used = false;
        for it in items.iter_mut() {
            if !path_matches(&it.path, path) {
                continue;
            }
            used = true;
            let [a, b] = it.new_range;
            if a == 0 || a > b {
                continue; // a pure deletion has no new side to have run
            }
            let executable: Vec<u64> = (a..=b).filter_map(|n| lines.get(&n).copied()).collect();
            if executable.is_empty() {
                continue; // nothing here the tracefile considers runnable
            }
            let cold = executable.iter().filter(|h| **h == 0).count();
            it.executed = Some((executable.len() - cold, executable.len()));
        }
        if !used {
            unmatched += 1;
        }
    }
    unmatched
}
