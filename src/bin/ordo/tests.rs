// --------------------------------------------------------------------- tests

use super::*;
use crate::commands::*;
use crate::comments::*;
use crate::config::*;
use crate::editor::*;
use crate::facts::*;
use crate::findings::*;
use crate::git::*;
use crate::handoff::*;
use crate::highlight::*;
use crate::history::*;
use crate::hover::*;
use crate::keys::*;
use crate::marks::*;
use crate::rules::*;
use crate::theme::*;
use crate::watch::*;
use ordo_diff::model::HunkOut;
use ordo_diff::model::Options;
use ordo_diff::model::Strategy;
use std::path::Path;
use tree_sitter::Parser;
use tree_sitter::Point;

fn lines(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

// ---- markdown highlighting ----

#[test]
fn markdown_block_structure_is_highlighted() {
    let src = "# Title\n\n- item\n\n```rust\nfn f() {}\n```\n";
    let h = highlight_roles("f.md", src).unwrap();
    // heading marker, list marker and fence delimiters all get a role other
    // than the default one, `Variable`.
    let heading_marker = h[0].iter().find(|(t, _)| t == "#").unwrap();
    assert_ne!(heading_marker.1, Role::Variable);
    let list_marker = h[2].iter().find(|(t, _)| t.starts_with('-')).unwrap();
    assert_ne!(list_marker.1, Role::Variable);
    let fence_open = h[4].iter().find(|(t, _)| t == "```").unwrap();
    assert_ne!(fence_open.1, Role::Variable);
}

/// A failed command used to leave no trace at all, so an unreadable object
/// mid-load reached the reviewer as "nothing to review".
#[test]
fn a_failed_command_is_recorded_once_and_git_only() {
    note_command_failure("git", &["cat-file", "-p", "deadbeef"], "bad object");
    note_command_failure("git", &["cat-file", "-p", "deadbeef"], "bad object");
    note_command_failure("but", &["--json", "status"], "not installed");

    let got = command_failures();
    let mine: Vec<&String> = got.iter().filter(|l| l.contains("deadbeef")).collect();
    assert_eq!(mine.len(), 1, "recorded more than once: {got:?}");
    assert_eq!(
        mine[0], "git cat-file -p deadbeef: bad object",
        "command and reason both belong in the line"
    );
    assert!(
        !got.iter().any(|l| l.starts_with("but ")),
        "`but` is optional; its absence is not a failure to report: {got:?}"
    );
}

const LCOV: &str = "\
SF:src/a.py
DA:10,3
DA:11,0
DA:12,0
DA:11,7
not-a-record
DA:oops,1
end_of_record
SF:src/untouched.py
DA:1,0
end_of_record
";

#[test]
fn lcov_records_become_line_counts() {
    let cov = parse_lcov(LCOV);
    let a = cov.files.get("src/a.py").expect("src/a.py");
    assert_eq!(a.get(&10), Some(&3));
    // the same line twice keeps the higher count: executed once anywhere
    // is executed, and merged tracefiles repeat lines
    assert_eq!(a.get(&11), Some(&7));
    assert_eq!(a.get(&12), Some(&0));
    assert_eq!(a.len(), 3, "malformed records are skipped: {a:?}");
    assert!(cov.files.contains_key("src/untouched.py"));
}

#[test]
fn malformed_lcov_is_skipped_not_fatal() {
    assert!(parse_lcov("").files.is_empty());
    assert!(
        parse_lcov(
            "garbage
DA:1,1
"
        )
        .files
        .is_empty(),
        "DA with no SF"
    );
}

#[test]
fn coverage_counts_only_lines_the_tracefile_calls_executable() {
    let mut items = vec![test_item("src/a.py")];
    // the hunk spans 9..=13; only 10, 11 and 12 are executable, and of
    // those only 12 never ran — 9 and 13 are blank or comment lines the
    // tracefile never mentions and must not count against the hunk
    items[0].new_range = [9, 13];
    let unmatched = place_coverage(&mut items, &parse_lcov(LCOV));
    assert_eq!(items[0].executed, Some((2, 3)), "2 of 3 executable ran");
    assert_eq!(unmatched, 1, "src/untouched.py matched no reviewed file");
}

#[test]
fn a_hunk_with_nothing_executable_reports_nothing() {
    let mut items = vec![test_item("src/a.py")];
    items[0].new_range = [100, 200];
    place_coverage(&mut items, &parse_lcov(LCOV));
    assert_eq!(
        items[0].executed, None,
        "no DA record in range is not the same as zero coverage"
    );
}

#[test]
fn a_pure_deletion_has_no_coverage_to_report() {
    let mut items = vec![test_item("src/a.py")];
    items[0].new_range = [11, 10]; // end < start: the empty new side
    place_coverage(&mut items, &parse_lcov(LCOV));
    assert_eq!(items[0].executed, None);
}

const SARIF: &str = r#"{"version":"2.1.0","runs":[{
  "tool":{"driver":{"name":"semgrep"}},
  "results":[
    {"ruleId":"no-eval","level":"error","message":{"text":"eval on input"},
     "locations":[{"physicalLocation":{
        "artifactLocation":{"uri":"src/a.py"},"region":{"startLine":10}}}]},
    {"ruleId":"doc","level":"note","message":{"text":"no docstring"},
     "locations":[{"physicalLocation":{
        "artifactLocation":{"uri":"file:///repo/src/a.py"},"region":{"startLine":50}}}]},
    {"ruleId":"broken","level":"warning","message":{"text":""},
     "locations":[{"physicalLocation":{
        "artifactLocation":{"uri":"src/a.py"},"region":{"startLine":1}}}]}
  ]}]}"#;

#[test]
fn sarif_results_become_findings() {
    let f = parse_sarif(SARIF);
    // the third result has an empty message and is skipped: a finding with
    // nothing to say is not worth a row
    assert_eq!(f.len(), 2, "{f:?}");
    assert_eq!(f[0].tool, "semgrep");
    assert_eq!(f[0].rule, "no-eval");
    assert_eq!(
        f[0].level,
        ordo_diff::model::Level::Warn,
        "error maps to the warn half"
    );
    assert_eq!((f[0].path.as_str(), f[0].line), ("src/a.py", 10));
    assert_eq!(
        f[1].level,
        ordo_diff::model::Level::Note,
        "note maps to the note half"
    );
    assert_eq!(f[1].path, "/repo/src/a.py", "file:// is stripped");
}

#[test]
fn malformed_sarif_is_skipped_not_fatal() {
    assert!(parse_sarif("not json").is_empty());
    assert!(parse_sarif("{}").is_empty());
    // a result with no location cannot be placed, so it is not a finding
    assert!(parse_sarif(r#"{"runs":[{"results":[{"message":{"text":"x"}}]}]}"#).is_empty());
}

#[test]
fn an_analyzer_path_matches_on_a_path_boundary() {
    assert!(path_matches("src/a.py", "src/a.py"));
    assert!(path_matches("src/a.py", "/home/u/repo/src/a.py"));
    // the suffix has to end at a separator, or `a.py` would match `ba.py`
    assert!(!path_matches("a.py", "src/ba.py"));
    assert!(!path_matches("src/a.py", "src/b.py"));
}

#[test]
fn a_finding_lands_on_the_hunk_that_contains_its_line() {
    let mut items = vec![test_item("src/a.py"), test_item("src/a.py")];
    items[0].new_range = [1, 20];
    items[1].new_range = [40, 60];
    let f = parse_sarif(SARIF);
    let unplaced = place_findings(&mut items, &f);

    assert_eq!(items[0].findings.len(), 1, "line 10 is in 1..=20");
    assert_eq!(items[1].findings.len(), 1, "line 50 is in 40..=60");
    assert_eq!(unplaced, 0);
    assert_eq!(top_level(&items[0]), Some(ordo_diff::model::Level::Warn));
    assert_eq!(top_level(&items[1]), Some(ordo_diff::model::Level::Note));
}

#[test]
fn a_review_comment_tints_its_row_by_level_like_any_finding() {
    let review = r#"{"runs":[{"tool":{"driver":{"name":"review-bot"}},"results":[
      {"kind":"review","level":"warning","message":{"text":"missed case"},
       "locations":[{"physicalLocation":{
          "artifactLocation":{"uri":"src/a.py"},"region":{"startLine":10}}}]}]}]}"#;
    let mut items = [test_item("src/a.py")];
    items[0].new_range = [1, 20];
    place_findings(&mut items, &parse_sarif(review));
    assert_eq!(
        items[0].findings[0].source,
        ordo_diff::model::FindingSource::Review
    );
    assert_eq!(top_level(&items[0]), Some(ordo_diff::model::Level::Warn));
}

#[test]
fn a_finding_outside_every_hunk_is_counted_not_dropped() {
    let mut items = vec![test_item("src/a.py")];
    items[0].new_range = [100, 200];
    // both findings sit outside it; neither may vanish silently
    assert_eq!(place_findings(&mut items, &parse_sarif(SARIF)), 2);
    assert!(items[0].findings.is_empty());
}

/// One run per tool is the normal shape in CI — semgrep and clippy in one
/// document — and the driver name is what a finding is labelled with, so
/// reading only the first run silently drops half the report.
#[test]
fn every_run_in_a_sarif_file_is_read() {
    let two = r#"{"version":"2.1.0","runs":[
      {"tool":{"driver":{"name":"semgrep"}},
       "results":[{"ruleId":"no-eval","level":"error","message":{"text":"eval"},
         "locations":[{"physicalLocation":{
            "artifactLocation":{"uri":"src/a.py"},"region":{"startLine":3}}}]}]},
      {"tool":{"driver":{"name":"clippy"}},
       "results":[{"ruleId":"needless_range_loop","level":"warning","message":{"text":"range"},
         "locations":[{"physicalLocation":{
            "artifactLocation":{"uri":"src/b.rs"},"region":{"startLine":7}}}]}]}
    ]}"#;
    let f = parse_sarif(two);
    assert_eq!(f.len(), 2, "{f:?}");
    let tools: Vec<&str> = f.iter().map(|x| x.tool.as_str()).collect();
    assert_eq!(
        tools,
        vec!["semgrep", "clippy"],
        "each run keeps its driver"
    );
}

/// A region without `startLine` cannot be placed on a hunk, and a result
/// with no `region` at all is a file-level finding — neither may be
/// invented a line number, and neither may take the whole file down.
#[test]
fn a_result_with_no_line_is_skipped_rather_than_placed_at_zero() {
    let no_line = r#"{"runs":[{"tool":{"driver":{"name":"t"}},"results":[
      {"ruleId":"a","message":{"text":"no region"},
       "locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/a.py"}}}]},
      {"ruleId":"b","message":{"text":"empty region"},
       "locations":[{"physicalLocation":{
          "artifactLocation":{"uri":"src/a.py"},"region":{}}}]},
      {"ruleId":"c","message":{"text":"real"},
       "locations":[{"physicalLocation":{
          "artifactLocation":{"uri":"src/a.py"},"region":{"startLine":4}}}]}
    ]}]}"#;
    let f = parse_sarif(no_line);
    assert_eq!(f.len(), 1, "only the located result survives: {f:?}");
    assert_eq!((f[0].rule.as_str(), f[0].line), ("c", 4));
}

/// `--sarif` may be given more than once, in either spelling. The flag
/// loop is the only thing standing between a reviewer's command line and
/// the reader that is already tested.
#[test]
fn the_sarif_flag_collects_every_path_in_both_spellings() {
    let argv: Vec<String> = ["--sarif", "a.json", "--sarif=b.json", "--sarif", "c.json"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let parsed = parse_argv(argv).expect("accepted");
    assert_eq!(parsed.sarif, vec!["a.json", "b.json", "c.json"]);
}

/// An unreadable analyzer report must not stop the review, and must not
/// vanish either: the run is reported the way every other failed command
/// is, and the readable files still contribute.
#[test]
fn an_unreadable_sarif_file_is_reported_and_the_rest_still_load() {
    let dir = std::env::temp_dir().join(format!("ordo-sarif-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp");
    let good = dir.join("good.sarif");
    std::fs::write(&good, SARIF).expect("write");
    let missing = dir.join("nope.sarif");

    let paths = vec![missing.display().to_string(), good.display().to_string()];
    let found = sarif_findings(&paths);
    assert_eq!(found.len(), 2, "the readable file still counts: {found:?}");
    assert!(
        command_failures()
            .iter()
            .any(|l| l.starts_with("sarif ") && l.contains("nope.sarif")),
        "the unreadable one is reported: {:?}",
        command_failures()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `:audit` accounts for every finding that could not be placed. Without
/// the line, a reviewer reads an analyzer report as fully covered when
/// some of it landed on lines this change never touched.
#[test]
fn audit_accounts_for_findings_that_landed_outside_the_change() {
    let items = vec![test_item("src/a.py")];
    let ledger = Ledger {
        findings_seen: 5,
        findings_unplaced: 2,
        ..Ledger::default()
    };
    let rows = build_audit(&items, 1, &Hidden::default(), &ledger, None);
    assert!(
        rows.iter()
            .any(|r| r.contains('2') && r.contains("analyzer findings")),
        "{rows:?}"
    );
    // …and says nothing at all when no analyzer was run
    let quiet = build_audit(&items, 1, &Hidden::default(), &Ledger::default(), None);
    assert!(
        !quiet.iter().any(|r| r.contains("analyzer findings")),
        "{quiet:?}"
    );
}

/// Every language the engine resolves is either painted here or listed as
/// deliberately unpainted. The two tables were separate copies of the same
/// extension map and had drifted by three entries — `.C`, `.H` and `.zsh`
/// got full semantics and no colour — which nothing could have noticed.
#[test]
fn every_engine_language_is_painted_or_listed() {
    for (name, _) in ordo_diff::languages() {
        assert!(
            highlight_for_lang(name).is_some() || NO_HIGHLIGHT.contains(&name),
            "engine language `{name}` has no highlights query and is not in NO_HIGHLIGHT"
        );
    }
}

/// The reverse: nothing is listed as unpainted that is actually painted,
/// so the list cannot rot into an excuse.
#[test]
fn nothing_listed_as_unpainted_is_painted() {
    for name in NO_HIGHLIGHT {
        assert!(
            highlight_for_lang(name).is_none(),
            "`{name}` is in NO_HIGHLIGHT but has a query"
        );
    }
}

/// The three extensions the duplicate table had lost.
#[test]
fn the_extensions_the_duplicate_table_dropped_are_painted() {
    for path in ["a.C", "a.H", "s.zsh"] {
        assert!(
            highlight_spec(path).is_some(),
            "{path} resolves in the engine but is not painted"
        );
    }
}

#[test]
fn markdown_inline_emphasis_is_highlighted() {
    let src = "plain and *emphasised* text\n";
    let h = highlight_roles("f.md", src).unwrap();
    let word = h[0].iter().find(|(t, _)| t == "emphasised").unwrap();
    assert_eq!(word.1, Role::Keyword);
}

#[test]
fn markdown_fenced_rust_uses_rust_grammar() {
    let src = "```rust\nfn add() {\n    let x = 1;\n}\n```\n";
    let h = highlight_roles("f.md", src).unwrap();
    let fn_kw = h[1].iter().find(|(t, _)| t == "fn").unwrap();
    assert_eq!(fn_kw.1, Role::Keyword);
    let let_kw = h[2].iter().find(|(t, _)| t == "let").unwrap();
    assert_eq!(let_kw.1, Role::Keyword);
}

#[test]
fn markdown_fence_in_unsupported_language_does_not_panic() {
    let src = "```console\n$ echo hi\nhi\n```\n";
    let h = highlight_roles("f.md", src).unwrap();
    assert_eq!(h.len(), src.lines().count() + 1);
}

#[test]
fn non_markdown_file_highlighting_is_unaffected() {
    let src = "fn add(a: i32) -> i32 {\n    a\n}\n";
    let h = highlight_roles("f.rs", src).unwrap();
    let fn_kw = h[0].iter().find(|(t, _)| t == "fn").unwrap();
    assert_eq!(fn_kw.1, Role::Keyword);
    let ty = h[0].iter().find(|(t, _)| t == "i32").unwrap();
    assert_eq!(ty.1, Role::Type);
}

// ---- background-load progress messages ----

#[test]
fn read_progress_is_one_based() {
    assert_eq!(
        read_progress("src/foo.rs", 0, 30),
        "reading src/foo.rs (1/30)"
    );
    assert_eq!(
        read_progress("src/foo.rs", 11, 30),
        "reading src/foo.rs (12/30)"
    );
    assert_eq!(
        read_progress("src/foo.rs", 29, 30),
        "reading src/foo.rs (30/30)"
    );
}

// ---- cursor clamping ----

#[test]
fn clamp_cursor_handles_shrunk_or_empty_files() {
    let ls = lines(&["ab"]);
    assert_eq!(
        clamp_cursor(Cursor { line: 5, col: 5 }, &ls),
        Cursor { line: 0, col: 1 }
    );
    assert_eq!(
        clamp_cursor(Cursor { line: 0, col: 0 }, &[]),
        Cursor { line: 0, col: 0 }
    );
}

// ---- byte/char column conversion ----

#[test]
fn char_byte_handles_multibyte() {
    let line = "héllo";
    assert_eq!(char_byte(line, 0), 0);
    assert_eq!(char_byte(line, 1), 1); // 'é' starts at byte 1
    assert_eq!(char_byte(line, 2), 3); // 'l' starts after the 2-byte 'é'
    assert_eq!(char_byte(line, 99), line.len());
}

// ---- symbol resolution + signature/docstring extraction ----

fn parse(lang: tree_sitter::Language, src: &str) -> tree_sitter::Tree {
    let mut p = Parser::new();
    p.set_language(&lang).unwrap();
    p.parse(src, None).unwrap()
}

#[test]
fn resolves_rust_call_site_to_its_definition() {
    let src = "/// adds two numbers\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    add(1, 2);\n}\n";
    let tree = parse(tree_sitter_rust::LANGUAGE.into(), src);
    let root = tree.root_node();
    // cursor on `add` inside `add(1, 2)` — row 6, "    add(1, 2);"
    let point = Point { row: 6, column: 5 };
    let mut node = root.descendant_for_point_range(point, point).unwrap();
    while !node.kind().contains("identifier") {
        node = node.parent().unwrap();
    }
    let name = node_text(node, src);
    assert_eq!(name, "add");
    let kinds = def_kinds("f.rs");
    let def = find_definition(root, kinds, &name, src).unwrap();
    assert_eq!(def.kind(), "function_item");
    assert_eq!(signature(def, src), "fn add(a: i32, b: i32) -> i32");
    assert_eq!(
        doc_for("f.rs", def, src).as_deref(),
        Some("/// adds two numbers")
    );
}

#[test]
fn reports_missing_rust_definition_plainly() {
    let src = "fn main() {\n    unknown_fn();\n}\n";
    let tree = parse(tree_sitter_rust::LANGUAGE.into(), src);
    let root = tree.root_node();
    let point = Point { row: 1, column: 5 };
    let mut node = root.descendant_for_point_range(point, point).unwrap();
    while !node.kind().contains("identifier") {
        node = node.parent().unwrap();
    }
    let name = node_text(node, src);
    assert_eq!(name, "unknown_fn");
    assert!(find_definition(root, def_kinds("f.rs"), &name, src).is_none());
}

#[test]
fn extracts_python_docstring_and_signature() {
    let src = "def greet(name):\n    \"\"\"Say hello.\"\"\"\n    print(name)\n";
    let tree = parse(tree_sitter_python::LANGUAGE.into(), src);
    let root = tree.root_node();
    let def = find_definition(root, def_kinds("f.py"), "greet", src).unwrap();
    assert_eq!(def.kind(), "function_definition");
    assert_eq!(signature(def, src), "def greet(name):");
    assert_eq!(
        doc_for("f.py", def, src).as_deref(),
        Some("\"\"\"Say hello.\"\"\"")
    );
}

#[test]
fn c_function_definition_name_comes_from_declarator() {
    // C has no `name` field on function_definition — the identifier is
    // nested in `declarator`, past the parameter_list.
    let src = "// squares x\nint square(int x) {\n    return x * x;\n}\n";
    let tree = parse(tree_sitter_c::LANGUAGE.into(), src);
    let root = tree.root_node();
    let def = find_definition(root, def_kinds("f.c"), "square", src).unwrap();
    assert_eq!(def.kind(), "function_definition");
    assert_eq!(signature(def, src), "int square(int x)");
    assert_eq!(doc_for("f.c", def, src).as_deref(), Some("// squares x"));
}

// ---- cross-commit history: identity, windowing, labeling ----

fn sym(name: &str, kind: &str, scope: Option<&str>) -> Symbol {
    Symbol {
        name: name.to_string(),
        kind: kind.to_string(),
        scope: scope.map(str::to_string),
    }
}

// The project owner's rule, verbatim: "ordo tree sitter type + scope for
// the symbol must match, otherwise it's a different symbol". A module-level
// `run` must NOT match a method `run` on class `A`, nor a same-scope `run`
// of a different tree-sitter kind — name alone is never enough.
#[test]
fn symbol_eq_requires_matching_name_kind_and_scope() {
    let module_level = sym("run", "function_definition", None);
    let method_a = sym("run", "function_definition", Some("A"));
    assert!(!symbol_eq(&module_level, &method_a));

    let different_kind = sym("run", "async_function_definition", None);
    assert!(!symbol_eq(&module_level, &different_kind));

    let same = sym("run", "function_definition", None);
    assert!(symbol_eq(&module_level, &same));
}

#[test]
fn qualified_name_joins_scope_and_name() {
    assert_eq!(
        qualified_name(&sym("run", "function_definition", None)),
        "run"
    );
    assert_eq!(
        qualified_name(&sym("run", "function_definition", Some("A"))),
        "A.run"
    );
}

#[test]
fn bound_earlier_drops_current_and_orders_oldest_of_window_first() {
    let shas: Vec<String> = ["current", "a", "b", "c"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let got = bound_earlier(shas, "current", 2);
    assert_eq!(got, vec!["b".to_string(), "a".to_string()]); // nearest 2, oldest first
}

#[test]
fn bound_earlier_bounds_a_long_lived_files_history() {
    let shas: Vec<String> = (0..20).map(|i| format!("c{i}")).collect(); // c0 nearest
    let got = bound_earlier(shas, "none-of-these", 10);
    assert_eq!(got.len(), 10);
    assert_eq!(got.first().unwrap(), "c9"); // oldest of the nearest-10 window
    assert_eq!(got.last().unwrap(), "c0"); // nearest to the reviewed rev
}

#[test]
fn bound_later_orders_nearest_to_rev_first_and_bounds_the_window() {
    // rev-list's own order: HEAD first, nearest descendant of rev last
    let shas: Vec<String> = ["head", "b", "a"].iter().map(|s| s.to_string()).collect();
    let got = bound_later(shas, 10);
    assert_eq!(
        got,
        vec!["a".to_string(), "b".to_string(), "head".to_string()]
    );

    let many: Vec<String> = (0..20).map(|i| format!("c{i}")).collect(); // c19 nearest to rev
    let got = bound_later(many, 10);
    assert_eq!(got.len(), 10);
    assert_eq!(got.first().unwrap(), "c19");
}

fn test_hunk(rationale: &str, enclosing: Option<&str>, symbols: Vec<Symbol>) -> HunkOut {
    HunkOut {
        id: "h1".to_string(),
        old_range: [0, 0],
        new_range: [1, 1],
        category: ordo_diff::model::Category::Definition,
        enclosing: enclosing.map(str::to_string),
        enclosing_kind: None,
        defines: vec![],
        uses: vec![],
        group: "g".to_string(),
        order_index: 0,
        rationale: rationale.to_string(),
        noise: false,
        extent: None,
        comment: false,
        details: vec![],
        notes: vec![],
        findings: vec![],
        uses_at: vec![],
        symbols,
    }
}

#[test]
fn label_for_picks_the_fragment_naming_the_symbol() {
    let target = sym("run", "function_definition", None);
    let h = test_hunk("adds helper; changes signature of run", None, vec![]);
    assert_eq!(label_for(&h, &target), "changes signature of run");
}

#[test]
fn label_for_falls_back_to_the_whole_rationale_for_a_body_edit() {
    let target = sym("run", "function_definition", Some("A"));
    let h = test_hunk("edits A.run", Some("A.run"), vec![]);
    assert_eq!(label_for(&h, &target), "edits A.run");
}

// ---- open-in-editor: command building ----

#[test]
fn split_command_separates_program_and_flags() {
    assert_eq!(split_command("code --wait"), vec!["code", "--wait"]);
    assert_eq!(split_command("vim"), vec!["vim"]);
    assert_eq!(
        split_command("emacsclient  -nw  -a ''"),
        vec!["emacsclient", "-nw", "-a", "''"]
    );
}

#[test]
fn editor_args_covers_each_line_argument_shape() {
    assert_eq!(
        editor_args("vim", "path", 120),
        vec!["+120".to_string(), "path".to_string()]
    );
    assert_eq!(
        editor_args("nvim", "path", 120),
        vec!["+120".to_string(), "path".to_string()]
    );
    assert_eq!(editor_args("hx", "path", 120), vec!["path:120".to_string()]);
    assert_eq!(
        editor_args("code", "path", 120),
        vec!["-g".to_string(), "path:120".to_string()]
    );
    // unknown editor: file only — never a guessed syntax it might read as
    // another filename
    assert_eq!(editor_args("subl", "path", 120), vec!["path".to_string()]);
}

#[test]
fn build_command_keeps_editor_flags_ahead_of_the_location() {
    let spec = split_command("code --wait");
    let (program, args) = build_command(&spec, "path", 120).unwrap();
    assert_eq!(program, "code");
    assert_eq!(
        args,
        vec![
            "--wait".to_string(),
            "-g".to_string(),
            "path:120".to_string()
        ]
    );
}

#[test]
fn build_command_matches_on_basename_not_full_path() {
    let spec = split_command("/usr/local/bin/hx");
    let (program, args) = build_command(&spec, "path", 120).unwrap();
    assert_eq!(program, "/usr/local/bin/hx");
    assert_eq!(args, vec!["path:120".to_string()]);
}

#[test]
fn build_command_is_none_for_an_empty_spec() {
    assert!(build_command(&[], "path", 120).is_none());
}

// ---- key/chord rendering ----

#[test]
fn key_label_renders_plain_and_modified_keys() {
    assert_eq!(key_label(ch('j')), "j");
    assert_eq!(key_label(ctrl('w')), "C-w");
    assert_eq!(key_label(plain(KeyCode::F(12))), "F12");
    assert_eq!(key_label((KeyCode::F(3), KeyModifiers::SHIFT)), "S-F3");
    assert_eq!(key_label(plain(KeyCode::Esc)), "Esc");
}

#[test]
fn chord_label_renders_known_chords() {
    // plain-char chords render tight, like vim's own "gg"/"ge" spelling
    assert_eq!(chord_label(Some(ch('g')), ch('g')), "gg");
    assert_eq!(chord_label(Some(ch('g')), ch('e')), "ge");
    // a modified prefix or key renders spaced, like "C-w C-w"
    assert_eq!(chord_label(Some(ctrl('w')), ctrl('w')), "C-w C-w");
    assert_eq!(chord_label(Some(ctrl('w')), ch('h')), "C-w h");
    assert_eq!(chord_label(None, ch('q')), "q");
}

// ---- keymap tables ----

#[test]
fn no_preset_binds_the_same_chord_twice() {
    // a key resolves to its first match, so a duplicate (prefix, key)
    // is silently shadowed rather than rejected — the second binding just
    // never fires. These tables have been edited by many separate changes,
    // so the invariant is worth asserting rather than assuming.
    for name in ["vim", "vscode"] {
        let km = keymap(name).unwrap();
        let mut seen: Vec<(Option<Key>, Key)> = vec![];
        for (prefix, key, _) in &km.binds {
            let pair = (*prefix, *key);
            assert!(
                !seen.contains(&pair),
                "{name}: {} is bound twice",
                chord_label(*prefix, *key)
            );
            seen.push(pair);
        }
    }
}

#[test]
fn every_bound_action_has_a_help_entry() {
    // The `?` help is generated from the bind table, so an Action without a
    // description would render a blank row instead of documenting the key.
    for name in ["vim", "vscode"] {
        let km = keymap(name).unwrap();
        for (prefix, key, action) in &km.binds {
            let (_, desc) = action_help(*action);
            assert!(
                !desc.trim().is_empty(),
                "{name}: {} has no help description",
                chord_label(*prefix, *key)
            );
        }
    }
}

// ---- command mode: line parsing ----

#[test]
fn parse_command_line_splits_name_and_argument() {
    assert_eq!(
        parse_command_line("strategy defs-first"),
        ("strategy", "defs-first")
    );
    assert_eq!(parse_command_line("filter   src/*   "), ("filter", "src/*"));
    assert_eq!(parse_command_line("q"), ("q", ""));
    assert_eq!(parse_command_line("  q  "), ("q", ""));
    assert_eq!(parse_command_line(""), ("", ""));
}

// ---- command mode: `:help` generated from the command table ----

#[test]
fn command_help_is_generated_from_the_command_table() {
    let help = build_command_help();
    // if `strategy`'s entry in `COMMANDS` ever changes, this row (built
    // from the table, not hand-copied) changes with it, and this breaks
    let strategy = COMMANDS.iter().find(|c| c.name == "strategy").unwrap();
    let row = help
        .iter()
        .find(|l| l.contains(strategy.help))
        .expect("strategy row present");
    assert!(
        row.contains(":strategy"),
        "expected the command name in the row: {row}"
    );
}

#[test]
fn every_command_has_a_non_empty_help_line() {
    for c in COMMANDS {
        assert!(!c.help.trim().is_empty(), ":{} has no help text", c.name);
    }
}

// ---- command mode: filter view + index clamping ----

#[test]
fn hidden_breakdown_partitions_the_hidden_set_exactly() {
    let mut noisy = test_item("src/a.rs");
    noisy.noise = true;
    let mut both = test_item("tests/b.rs"); // noise AND outside the glob
    both.noise = true;
    let outside = test_item("tests/c.rs");
    let shown = test_item("src/d.rs");
    let items = vec![noisy, both, outside, shown];
    let globs = build_globs(&["src/*".to_string()]).unwrap();

    let h = hidden_breakdown(&items, false, false, Some(&globs), None);
    // an item hidden twice is charged once, to the first reason
    assert_eq!(
        h,
        Hidden {
            comment: 0,
            noise: 2,
            glob: 1,
            wave: 0,
            unaccounted: 0
        }
    );
    assert_eq!(
        compute_view(&items, false, false, Some(&globs), None).len() + h.noise + h.glob,
        items.len()
    );
}

#[test]
fn hidden_breakdown_charges_only_comments_before_noise() {
    let mut a = test_item("a.rs");
    a.comment = true;
    let mut b = test_item("b.rs");
    b.noise = true; // hidden by only-comments first, not by noise
    let items = vec![a, b];

    let h = hidden_breakdown(&items, true, false, None, None);
    assert_eq!(
        h,
        Hidden {
            comment: 1,
            noise: 0,
            glob: 0,
            wave: 0,
            unaccounted: 0
        }
    );
}

#[test]
fn build_audit_reports_every_reason_and_flags_an_unaccounted_remainder() {
    let items = vec![test_item("a.rs"), test_item("b.rs"), test_item("c.rs")];
    let ledger = Ledger {
        files_seen: 9,
        files_generated: 2,
        files_declared: 1,
        files_globbed: 3,
        files_unreadable: 1,
        hunks_import: 4,
        hunks_non_comment: 0,
        findings_seen: 0,
        findings_unplaced: 0,
        coverage_files: 0,
        coverage_unmatched: 0,
    };
    let clean = Hidden {
        comment: 0,
        noise: 1,
        glob: 0,
        wave: 0,
        unaccounted: 0,
    };
    let text = build_audit(&items, 2, &clean, &ledger, None).join("\n");
    assert!(text.contains("2 of 3 hunks shown"), "{text}");
    assert!(text.contains("   4  of which import hunks"), "{text}");
    assert!(text.contains("files: 9 changed"), "{text}");
    assert!(
        text.contains("never fetched: excluded by a launch-time glob"),
        "{text}"
    );
    assert!(
        text.contains("every hidden hunk is accounted for"),
        "{text}"
    );

    let leak = Hidden {
        comment: 0,
        noise: 0,
        glob: 0,
        wave: 0,
        unaccounted: 1,
    };
    let text = build_audit(&items, 2, &leak, &ledger, Some("src/*")).join("\n");
    assert!(text.contains("1 hidden hunk unaccounted for"), "{text}");
    assert!(text.contains("outside the path filter 'src/*'"), "{text}");
}

#[test]
fn compute_view_applies_only_comments_show_all_and_glob_independently() {
    let mut a = test_item("src/a.rs");
    a.comment = true;
    let mut b = test_item("src/b.rs");
    b.noise = true;
    let c = test_item("tests/c.rs");
    let items = vec![a, b, c];

    assert_eq!(compute_view(&items, false, true, None, None), vec![0, 1, 2]);
    assert_eq!(compute_view(&items, true, true, None, None), vec![0]); // only-comments
    assert_eq!(compute_view(&items, false, false, None, None), vec![0, 2]); // hide noise

    let globs = build_globs(&["src/*".to_string()]).unwrap();
    assert_eq!(
        compute_view(&items, false, true, Some(&globs), None),
        vec![0, 1]
    );
}

#[test]
fn set_filters_clamps_selection_off_a_hunk_that_falls_out_of_view() {
    let mut app = test_app();
    app.items = vec![test_item("a.rs"), test_item("b.rs"), test_item("c.rs")];
    app.view = vec![0, 1, 2];
    app.reviewed = vec![false, false, false];
    app.sel = 1; // b.rs — about to be filtered out
    app.popup = Some(Popup::new("x", vec![]));

    let globs = build_globs(&["a.rs".to_string()]).unwrap();
    set_filters(
        &mut app,
        false,
        true,
        Some(("a.rs".to_string(), globs)),
        None,
    )
    .unwrap();

    assert_eq!(app.view, vec![0]);
    assert_eq!(app.sel, 0, "selection must move off the now-hidden item");
    assert!(app.popup.is_none(), "select() clears a stale popup");
}

#[test]
fn set_filters_rejects_a_combination_that_would_empty_the_view() {
    let mut app = test_app();
    app.items = vec![test_item("a.rs")];
    app.view = vec![0];
    app.reviewed = vec![false];

    let globs = build_globs(&["nope/*".to_string()]).unwrap();
    let err = set_filters(
        &mut app,
        false,
        true,
        Some(("nope/*".to_string(), globs)),
        None,
    );
    assert!(err.is_err());
    // rejected: state is untouched, still pointing at the one real item
    assert_eq!(app.view, vec![0]);
    assert!(app.path_filter.is_none());
}

#[test]
fn view_pos_reads_the_view_not_the_raw_item_list() {
    assert_eq!(view_pos(&[3, 5, 8], 5), 1);
    assert_eq!(view_pos(&[3, 5, 8], 99), 0); // not found: degrades to 0
}

fn test_item(path: &str) -> Item {
    Item {
        bucket: "g0".to_string(),
        ledger: None,
        path: path.to_string(),
        old_range: [0, 0],
        new_range: [0, 0],
        cat: ordo_diff::model::Category::Other,
        rationale: String::new(),
        details: vec![],
        notes: vec![],
        edges: vec![],
        uses_at: vec![],
        noise: false,
        comment: false,
        symbols: vec![],
        enclosing: None,
        group: String::new(),
        findings: vec![],
        executed: None,
        refined: ordo_diff::refine::Refined::default(),
        wave: None,
    }
}

fn edge(label: &str, target: Option<usize>) -> EdgeRef {
    EdgeRef {
        label: label.to_string(),
        target,
        // `←` is the direction that means "this hunk uses what the target
        // defines", which is what makes the target a dependency
        dependency: label.starts_with('←'),
    }
}

// ---- def→use edges: target resolution ----

fn id_hunk(id: &str) -> HunkOut {
    let mut h = test_hunk("does a thing", None, vec![]);
    h.id = id.to_string();
    h
}

fn test_file_out(path: &str, hunks: Vec<HunkOut>) -> ordo_diff::model::FileOut {
    ordo_diff::model::FileOut {
        path: path.to_string(),
        hunks,
        degraded: false,
        unsupported: false,
        dropped: vec![],
    }
}

#[test]
fn build_items_resolves_edge_targets_within_the_review_and_flags_ones_outside_it() {
    let out = Output {
        schema: 1,
        order: vec![
            ordo_diff::model::OrderItem {
                path: "a.rs".to_string(),
                hunk: "h1".to_string(),
            },
            ordo_diff::model::OrderItem {
                path: "a.rs".to_string(),
                hunk: "h2".to_string(),
            },
        ],
        files: vec![test_file_out("a.rs", vec![id_hunk("h1"), id_hunk("h2")])],
        groups: vec![],
        edges: vec![
            // resolves: h2 is item index 1
            ordo_diff::model::Edge {
                from: "h1".to_string(),
                to: "h2".to_string(),
                why: "uses it".to_string(),
            },
            // doesn't resolve: "ghost" was never built into an item (e.g.
            // filtered out, or a cross-file edge to an unreviewed file)
            ordo_diff::model::Edge {
                from: "h1".to_string(),
                to: "ghost".to_string(),
                why: "calls it".to_string(),
            },
        ],
        clusters: vec![],
        problems: vec![],
        notes: vec![],
        ledger: vec![],
        summary: None,
    };
    let items = build_items(&out);
    assert_eq!(items.len(), 2);
    assert!(items[0].edges.iter().any(|e| e.target == Some(1)));
    assert!(items[0].edges.iter().any(|e| e.target.is_none()));
}

/// `catalog = false` in a rules file has to reach the engine, and a rules
/// file that says nothing must leave the catalog alone.
#[test]
fn a_rules_file_can_switch_the_catalog_off() {
    let base = Path::new(".");
    assert_eq!(
        parse_rules_doc("catalog = false\n", base).catalog,
        Some(false),
        "the file said no"
    );
    assert_eq!(
        parse_rules_doc("[[rule]]\nname = \"x\"\nnote = \"y\"\n", base).catalog,
        None,
        "a file that says nothing does not vote"
    );
    assert_eq!(
        parse_rules_doc("catalog = true\n", base).catalog,
        Some(true)
    );
}

/// Two rules of one name coexist only while no file could see both. The
/// engine keys its work by index, so `magic-number` can be a python rule
/// and a rust rule; two python ones are still a mistake.
#[test]
fn one_name_twice_is_a_mistake_only_when_the_languages_meet() {
    let base = Path::new(".");
    let two = |a: &str, b: &str| {
        let text = format!(
            "[[rule]]\nname = \"dup\"\n{a}\nnote = \"x\"\n\n\
             [[rule]]\nname = \"dup\"\n{b}\nnote = \"y\"\n"
        );
        parse_rules(&text, base)
    };
    let (rules, problems) = two("lang = \"python\"", "lang = \"rust\"");
    assert_eq!(rules.len(), 2, "different grammars coexist: {problems:?}");
    assert!(problems.is_empty(), "{problems:?}");

    let (rules, problems) = two("lang = \"python\"", "lang = [\"python\", \"rust\"]");
    assert_eq!(rules.len(), 1, "overlapping grammars clash");
    assert!(problems[0].contains("same language"), "{problems:?}");

    let (rules, problems) = two("", "lang = \"rust\"");
    assert_eq!(rules.len(), 1, "an unscoped rule overlaps with everything");
    assert!(problems[0].contains("same language"), "{problems:?}");
}

/// A colour and a key binding are free text. A settings view that shows
/// them but cannot change them is a list, not a config.
#[test]
fn a_text_setting_can_be_typed_into() {
    let app = test_app();
    let mut ui = ConfigUi::open(&app);
    let at = ui
        .rows
        .iter()
        .position(|&(i, j)| ui.sections[i].fields[j].key.starts_with("theme."))
        .expect("a theme role");
    ui.sel = at;

    // Space opens it rather than cycling, and changes nothing yet
    assert!(!ui.toggle(), "text does not toggle");
    assert!(ui.editing.is_some(), "it opened for editing");
    assert!(!ui.dirty, "opening an editor is not a change");

    ui.editing = Some("#ff0000".to_string());
    ui.commit_edit();
    assert!(ui.editing.is_none(), "committing closes the editor");
    assert!(ui.dirty);
    assert_eq!(
        ui.changed()
            .iter()
            .map(|f| f.key.as_str())
            .collect::<Vec<_>>(),
        vec![ui.field(at).unwrap().key.as_str()],
        "exactly the edited field changed"
    );

    // and typing the same value back is not a change
    let key = ui.field(at).unwrap().key.clone();
    let before = ui
        .original
        .iter()
        .flat_map(|s| &s.fields)
        .find(|f| f.key == key)
        .and_then(|f| match &f.kind {
            FieldKind::Text(t) => Some(t.clone()),
            _ => None,
        })
        .expect("original value");
    ui.editing = Some(before);
    ui.commit_edit();
    assert!(ui.changed().is_empty(), "typed back to where it started");
}

/// The whole point: a toggle reaches the file on disk.
#[test]
fn writing_lands_in_the_files_the_sections_name() {
    let dir = std::env::temp_dir().join(format!("ordo-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ordo")).expect("temp config dir");
    std::fs::write(dir.join("ordo/ordo.toml"), "# my notes\npreset = \"vim\"\n")
        .expect("seed ordo.toml");
    std::fs::write(dir.join("ordo/rules.toml"), "# my rules\n").expect("seed rules.toml");
    // SAFETY: single-threaded test, and the value is read through
    // `config_path` on this thread only
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

    let mut app = test_app();
    app.config = Some(ConfigUi::open(&app));
    let at = {
        let c = app.config.as_ref().unwrap();
        c.rows
            .iter()
            .position(|&(i, j)| c.sections[i].fields[j].key == "catalog")
            .expect("catalog switch")
    };
    app.config.as_mut().unwrap().sel = at;
    app.config.as_mut().unwrap().toggle();
    config_write(&mut app);

    let rules = std::fs::read_to_string(dir.join("ordo/rules.toml")).expect("rules.toml");
    assert!(rules.contains("catalog = false"), "not written:\n{rules}");
    assert!(
        rules.contains("# my rules"),
        "the reviewer's own note survived"
    );
    let settings = std::fs::read_to_string(dir.join("ordo/ordo.toml")).expect("ordo.toml");
    assert_eq!(
        settings, "# my notes\npreset = \"vim\"\n",
        "untouched by a rules change"
    );
    assert!(!app.config.as_ref().unwrap().dirty, "written means clean");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writing must carry only what the reviewer altered. The schema holds
/// every setting; materialising all of them would bury a one-line change.
#[test]
fn only_the_changed_settings_are_written() {
    let app = test_app();
    let mut ui = ConfigUi::open(&app);
    assert!(ui.changed().is_empty(), "nothing changed yet");

    let at = ui
        .rows
        .iter()
        .position(|&(i, j)| ui.sections[i].fields[j].key == "catalog")
        .expect("catalog switch");
    ui.sel = at;
    ui.toggle();

    let changed: Vec<&str> = ui.changed().iter().map(|f| f.key.as_str()).collect();
    assert_eq!(changed, vec!["catalog"], "one flip, one changed field");

    // and flipping it back is no change at all, not two
    ui.toggle();
    assert!(ui.changed().is_empty(), "back to where it started");
}

/// The config file's commented-out defaults ARE its documentation, and a
/// hand-written comment is someone's note to themselves. Writing one
/// setting must not cost either.
#[test]
fn upsert_keeps_the_rest_of_the_file() {
    let file = "# ordo configuration\n\
                preset = \"vim\"\n\
                theme = \"dark\"\n\
                \n\
                # ---- keys\n\
                [binds]\n\
                # \"j\" = \"next\"  # move down\n\
                \n\
                [theme]\n\
                # accent = \"#89b4fa\"\n";

    // a live key is replaced where it sits
    let got = upsert_toml(file, None, "theme", "\"catppuccin-mocha\"");
    assert!(got.contains("theme = \"catppuccin-mocha\""));
    assert!(got.contains("preset = \"vim\""), "the neighbour survives");
    assert!(got.contains("# ---- keys"), "comments survive");
    assert_eq!(got.matches("theme =").count(), 1, "not duplicated");

    // a commented-out default is uncommented in place, keeping its section
    let got = upsert_toml(file, Some("theme"), "accent", "\"#ff0000\"");
    assert!(got.contains("accent = \"#ff0000\""));
    assert!(!got.contains("# accent"), "the old commented line is gone");
    assert!(
        got.contains("# \"j\" = \"next\""),
        "other sections untouched"
    );

    // a key with no line yet lands inside its section
    let got = upsert_toml(file, Some("binds"), "\"q\"", "\"quit\"");
    let binds_at = got.find("[binds]").expect("binds");
    let theme_at = got.find("[theme]").expect("theme");
    let q_at = got.find("\"q\" = \"quit\"").expect("the new bind");
    assert!(
        binds_at < q_at && q_at < theme_at,
        "landed in [binds]:\n{got}"
    );

    // a missing section is created rather than the key going astray
    let got = upsert_toml("preset = \"vim\"\n", Some("theme"), "accent", "\"#ff0000\"");
    assert!(got.contains("[theme]"));
    assert!(got.trim_end().ends_with("accent = \"#ff0000\""));
}

/// The view owns its keys instead of going through the keymap, so its hints
/// are true in any preset — and so cycling the `preset` setting cannot
/// rebind the view while it is open. That bug cost a live debugging session:
/// `x` cycled vim to vscode and `w` stopped writing.
#[test]
fn the_config_view_does_not_lose_its_keys_when_the_preset_changes() {
    let mut app = test_app();
    app.config = Some(ConfigUi::open(&app));
    let at = |app: &App, key: &str| {
        let c = app.config.as_ref().unwrap();
        c.rows
            .iter()
            .position(|&(i, j)| c.sections[i].fields[j].key == key)
            .unwrap_or_else(|| panic!("no {key}"))
    };
    // cycle the preset: the live keymap changes under the view
    app.config.as_mut().unwrap().sel = at(&app, "preset");
    let before = app.keys.name;
    config_toggle(&mut app);
    assert_ne!(app.keys.name, before, "the preset really did change");

    // and the view still works: its keys never went through the keymap
    app.config.as_mut().unwrap().sel = at(&app, "catalog");
    config_toggle(&mut app);
    let c = app.config.as_ref().unwrap();
    assert!(
        matches!(
            c.field(c.sel).map(|f| &f.kind),
            Some(FieldKind::Flag(false))
        ),
        "the catalog switch still toggles after a preset change"
    );
}

/// Turning a catalog section off carries its rules with it, and the section
/// reads off once nothing in it is left on. That cascade is the gesture the
/// UI exists for: nobody wants to press Space fourteen times.
#[test]
fn a_catalog_section_carries_its_rules() {
    let mut ui = ConfigUi::open(&test_app());
    let sections = ordo_diff::catalog::sections();
    let first = &sections[0];
    let at = |ui: &ConfigUi, key: &str| {
        ui.rows
            .iter()
            .position(|&(i, j)| ui.sections[i].fields[j].key == key)
            .unwrap_or_else(|| panic!("no field {key}"))
    };
    let flag = |ui: &ConfigUi, key: &str| match ui.field(at(ui, key)).map(|f| &f.kind) {
        Some(FieldKind::Flag(v)) => *v,
        other => panic!("{key} is not a flag: {other:?}"),
    };
    let section_key = format!("section:{}", first.name);
    assert!(flag(&ui, &section_key), "sections start on");

    ui.sel = at(&ui, &section_key);
    assert!(ui.toggle());
    assert!(!flag(&ui, &section_key), "the section went off");
    for r in &first.rules {
        assert!(
            !flag(&ui, &format!("disable:{r}")),
            "{r} should have gone with its section"
        );
    }

    // one rule back on brings the section back with it
    ui.sel = at(&ui, &format!("disable:{}", first.rules[0]));
    assert!(ui.toggle());
    assert!(flag(&ui, &section_key), "the section follows its rules");
    assert!(ui.dirty);
}

/// The settings surface is derived, never listed. Every table that feeds it
/// must be represented, so adding a theme role or a catalog rule shows up
/// in `:config` without anyone editing a second list.
#[test]
fn the_config_schema_is_generated_from_the_live_tables() {
    let app = test_app();
    let schema = config_schema(&app);
    let titles: Vec<&str> = schema.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "general",
            "catalog",
            "levels",
            "rulesets",
            "theme roles",
            "keys"
        ]
    );
    let by = |t: &str| {
        schema
            .iter()
            .find(|s| s.title == t)
            .unwrap_or_else(|| panic!("no {t} section"))
    };
    // one field per entry of each source table, plus the catalog's own
    // switch and one per section
    assert_eq!(by("rulesets").fields.len(), PRESETS.len());
    assert_eq!(by("theme roles").fields.len(), THEME_ROLES.len());
    assert_eq!(by("keys").fields.len(), app.keys.binds.len());
    let sections = ordo_diff::catalog::sections();
    let rules: usize = sections.iter().map(|s| s.rules.len()).sum();
    assert_eq!(by("catalog").fields.len(), 1 + sections.len() + rules);
    let names: HashSet<&String> = sections.iter().flat_map(|s| &s.rules).collect();
    assert_eq!(
        by("levels").fields.len(),
        names.len(),
        "one level row per name"
    );
    // and the preset choice offers exactly the presets that resolve
    let general = &by("general").fields[0];
    match &general.kind {
        FieldKind::Choice { options, .. } => assert_eq!(options.len(), KEYMAP_NAMES.len()),
        k => panic!("preset should be a choice, got {k:?}"),
    }
}

#[test]
fn the_keymap_list_and_the_keymap_function_agree() {
    for n in KEYMAP_NAMES {
        assert!(keymap(n).is_some(), "`{n}` is listed but does not resolve");
    }
    assert!(
        keymap("nano").is_none(),
        "an unlisted preset must not resolve"
    );
}

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

fn log(rows: &[(&str, &str, &str)]) -> String {
    rows.iter()
        .map(|(s, a, d)| format!("{s}\t{a}\t{d}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The reviewed commit leads its own `git log -L` output. It is the change
/// being read, not history, so it must not be counted — but only when the
/// query was made against it (see `churn_query`).
#[test]
fn the_reviewed_commit_is_not_counted_as_its_own_history() {
    let out = log(&[
        (SHA_A, "Ada", "2026-09-10"),
        (SHA_B, "Bob", "2026-09-01"),
        (SHA_C, "Cy", "2026-08-01"),
    ]);
    let c = churn_from_log(&out, SHA_A, true);
    assert_eq!(c.commits, 2, "the reviewed commit is dropped");
    assert_eq!(c.last, Some(("Bob".to_string(), "2026-09-01".to_string())));

    // an uncommitted review follows the old side against HEAD, and HEAD is
    // real history — dropping it would lose an edit
    let c = churn_from_log(&out, SHA_A, false);
    assert_eq!(c.commits, 3);
    assert_eq!(c.last, Some(("Ada".to_string(), "2026-09-10".to_string())));

    // the reviewed commit did not touch these exact lines, so it does not
    // lead: nothing may be dropped
    let c = churn_from_log(&out, SHA_C, true);
    assert_eq!(c.commits, 3);
}

/// `review_sha` is the rev the user typed, which may be abbreviated, while
/// `%H` is always full.
#[test]
fn an_abbreviated_review_sha_still_matches_the_leading_commit() {
    let out = log(&[(SHA_A, "Ada", "2026-09-10"), (SHA_B, "Bob", "2026-09-01")]);
    assert_eq!(churn_from_log(&out, &SHA_A[..8], true).commits, 1);
}

/// "exactly the window" and "more than the window" are different claims.
#[test]
fn the_window_distinguishes_exactly_from_at_least() {
    let rows: Vec<(String, &str, &str)> = (0..CHURN_WINDOW + 4)
        .map(|i| (format!("{i:040}"), "Ada", "2026-09-01"))
        .collect();
    let as_refs: Vec<(&str, &str, &str)> = rows.iter().map(|(s, a, d)| (&s[..], *a, *d)).collect();

    let exact = log(&as_refs[..CHURN_WINDOW]);
    let c = churn_from_log(&exact, "none", false);
    assert_eq!((c.commits, c.capped), (CHURN_WINDOW, false));

    let over = log(&as_refs);
    let c = churn_from_log(&over, "none", false);
    assert_eq!((c.commits, c.capped), (CHURN_WINDOW, true));
}

#[test]
fn no_history_and_malformed_rows_report_nothing_rather_than_guessing() {
    let c = churn_from_log("", "none", true);
    assert_eq!((c.commits, c.capped, c.last), (0, false, None));
    // a row without the three tab-separated fields is skipped, not counted
    let c = churn_from_log("garbage\nalso garbage", "none", false);
    assert_eq!(c.commits, 0);
}

/// A pure insertion has an empty old-side range; git rejects an inverted
/// range, so it must never be asked.
#[test]
fn an_insertion_has_no_old_lines_to_follow() {
    let mut it = test_item("a.rs");
    it.old_range = [2, 1]; // what the engine emits for a pure insert
    it.new_range = [2, 2];
    let q = churn_query(&it, true);
    assert_eq!(q.rows, [2, 1]);
    let c = hunk_churn("HEAD", "a.rs", &q);
    assert_eq!((c.commits, c.last), (0, None));
}

/// The side the query follows is the one the rev actually contains.
#[test]
fn a_commit_review_follows_the_new_side_and_an_uncommitted_one_the_old() {
    let mut it = test_item("a.rs");
    it.old_range = [10, 20];
    it.new_range = [30, 40];
    let q = churn_query(&it, false);
    assert_eq!((q.rows, q.drop_leading_rev), ([30, 40], true));
    let q = churn_query(&it, true);
    assert_eq!((q.rows, q.drop_leading_rev), ([10, 20], false));
}

/// A sweeping change must not pay for a signal nobody triages by. One
/// `rev-list` per file is ~9ms, which is nothing for a normal review and
/// seconds for a mass rename.
#[test]
fn the_eager_churn_pass_gives_up_on_a_sweeping_change() {
    let many: Vec<String> = (0..FILE_CHURN_MAX_FILES + 1)
        .map(|i| format!("f{i}.rs"))
        .collect();
    let calls = std::cell::Cell::new(0usize);
    let progress = |_: String| calls.set(calls.get() + 1);
    let got = file_churn("HEAD", &many, &progress);
    assert!(got.is_empty(), "past the cap it counts nothing");
    assert_eq!(calls.get(), 0, "and does not shell out even once");
}

fn test_app() -> App {
    App {
        items: vec![test_item("a.rs")],
        changes: vec![],
        docs_last: true,
        reviewed: vec![false],
        view: vec![0],
        comments_only: false,
        show_all: true,
        path_filter: None,
        only_wave: None,
        sel: 0,
        keys: keymap("vim").unwrap(),
        sources: HashMap::new(),
        cursor: Cursor { line: 0, col: 0 },
        popup: None,
        trees: HashMap::new(),
        review_sha: None,
        subject: None,
        uncommitted: false,
        history_cache: HashMap::new(),
        churn_cache: HashMap::new(),
        file_churn: HashMap::new(),
        catalog: true,
        disables: vec![],
        levels: Default::default(),
        includes: vec![],
        config: None,
        rev: "HEAD".to_string(),
        marks_path: None,
        marks: HashMap::new(),
        theme: theme("dark").unwrap(),
        show_groups: false,
        groups: HashMap::new(),
        group_reasons: HashMap::new(),
        mode: ViewMode::Hunks,
        symbol_ledger: vec![],
        summary: None,
        notes: HashMap::new(),
        notes_path: None,
        comments: vec![],
        comments_path: None,
        selection: None,
        watch: WatchMode::Off,
        stale: None,
        notice: None,
        waves: Default::default(),
        reload_requested: false,
        deltas: vec![],
        delta_gone: 0,
        collapsed: HashSet::new(),
        ledger: Ledger::default(),
        rules: vec![],
        strategy: "comprehension".to_string(),
        rules_report: vec![],
    }
}

#[test]
fn the_list_defaults_to_symbols_and_mode_switches_it_back() {
    let mut app = test_app();
    app.items = vec![grouped_item("a.rs", "g0"), grouped_item("b.rs", "g1")];
    app.items[0].ledger = Some(0);
    let led = vec![ordo_diff::model::LedgerEntry {
        name: "fetch".to_string(),
        kind: Some("function_definition".to_string()),
        scope: None,
        path: "a.rs".to_string(),
        at: "h0".to_string(),
        change: ordo_diff::model::SymbolChange::Signature,
        from: None,
        used_by: vec!["h1".to_string()],
    }];

    app.symbol_ledger = led.clone();
    set_mode(&mut app, ViewMode::Ledger, &led);
    // the symbol with a ledger entry buckets under it; the one without
    // falls into the shared bucket rather than vanishing
    assert_eq!(app.items[0].bucket, "L0");
    assert_eq!(app.items[1].bucket, "L-");
    // groups *are* the ledger in this mode, so they are forced on
    assert!(app.show_groups);
    assert_eq!(app.groups.get("L0").map(String::as_str), Some("fetch"));
    assert_eq!(app.groups.get("L-").map(String::as_str), Some(""));
    // the entry's sentence moved to the why pane's scope row
    assert_eq!(
        scope_line(&app, 0).as_deref(),
        Some("fetch — signature, used by 1 hunk")
    );

    set_mode(&mut app, ViewMode::Hunks, &led);
    assert_eq!(app.items[0].bucket, "g0");
    assert_eq!(app.items[1].bucket, "g1");
}

#[test]
fn switching_mode_drops_fold_state_keyed_to_the_old_one() {
    let mut app = test_app();
    app.items = vec![grouped_item("a.rs", "g0")];
    app.collapsed.insert("g0".to_string());
    set_mode(&mut app, ViewMode::Ledger, &[]);
    // "g0" means nothing in ledger mode; keeping it would fold a bucket
    // that no longer exists
    assert!(app.collapsed.is_empty());
}

fn item_with_symbol(path: &str, name: &str) -> Item {
    let mut it = test_item(path);
    it.symbols = vec![sym(name, "function_definition", None)];
    it
}

/// Two items where 1 uses what 0 defines: `1 ← 0`.
fn dependent_pair() -> App {
    let mut app = test_app();
    app.items = vec![test_item("a.py"), test_item("b.py")];
    app.items[1].edges = vec![edge("← a.py:L1   def→use: f", Some(0))];
    app.items[0].edges = vec![edge("→ b.py:L1   def→use: f", Some(1))];
    app.view = vec![0, 1];
    app.reviewed = vec![false, false];
    app
}

fn snaps_of(app: &App, sources: &Sources) -> HashMap<String, Snap> {
    let order: Vec<usize> = (0..app.items.len()).collect();
    compare_runs(&app.items, &order, sources, &HashMap::new()).0
}

/// 0 defines what 1 uses; 1 defines what 2 uses. Rejecting 0 strands both.
fn dependency_chain() -> App {
    let mut app = test_app();
    app.items = vec![test_item("a.py"), test_item("b.py"), test_item("c.py")];
    app.items[1].edges = vec![edge("← a.py:L1   def→use: f", Some(0))];
    app.items[2].edges = vec![edge("← b.py:L1   def→use: g", Some(1))];
    app.view = vec![0, 1, 2];
    app.reviewed = vec![false, false, false];
    app
}

#[test]
fn a_draft_rule_states_the_hunk_it_came_from() {
    let mut app = test_app();
    app.items = vec![item_with_symbol("a.py", "fetch")];
    app.items[0].notes = vec!["7 params".to_string()];
    let d = draft_rule(&app, 0).join("\n");
    assert!(d.contains("[[rule]]"), "{d}");
    assert!(d.contains("lang = \"python\""), "{d}");
    assert!(d.contains("kind = [\"function_definition\"]"), "{d}");
    // one below what this hunk measured, so the rule fires on it
    assert!(d.contains("max-params = 6"), "{d}");
    assert!(d.contains("warn ="), "{d}");
}

#[test]
fn a_draft_turns_each_structural_note_into_its_limit() {
    let mut app = test_app();
    app.items = vec![item_with_symbol("a.py", "f")];
    app.items[0].notes = vec![
        "large definition (120 lines)".to_string(),
        "deeply nested (depth 4)".to_string(),
    ];
    let d = draft_rule(&app, 0).join("\n");
    assert!(d.contains("max-lines = 119"), "{d}");
    assert!(d.contains("max-nesting = 3"), "{d}");
}

#[test]
fn a_draft_from_a_hunk_with_no_symbol_says_it_is_broad() {
    let mut app = test_app();
    app.items = vec![test_item("a.py")];
    let d = draft_rule(&app, 0).join("\n");
    assert!(!d.contains("kind = ["), "{d}");
    assert!(d.contains("no symbol"), "{d}");
}

#[test]
fn a_cascade_is_transitive_not_just_the_direct_dependents() {
    // pushing back on a leaf when the root is the problem sends the author
    // round the loop twice
    let app = dependency_chain();
    assert_eq!(cascade(&app, 0), vec![1, 2]);
    assert_eq!(cascade(&app, 1), vec![2]);
    assert!(cascade(&app, 2).is_empty());
}

#[test]
fn a_cascade_terminates_on_a_dependency_cycle() {
    // mutual recursion is a real def→use cycle
    let mut app = dependency_chain();
    app.items[0].edges = vec![edge("← c.py:L1   def→use: h", Some(2))];
    let hit = cascade(&app, 0);
    assert_eq!(hit, vec![1, 2], "{hit:?}");
}

#[test]
fn a_cascade_ignores_hunks_filtered_out_of_the_view() {
    let mut app = dependency_chain();
    app.view = vec![0, 1];
    assert_eq!(cascade(&app, 0), vec![1]);
}

#[test]
fn a_leaf_strands_nothing() {
    let app = dependency_chain();
    assert!(cascade(&app, 2).is_empty());
    assert!(!cascade(&app, 0).is_empty());
}

#[test]
fn a_hunk_that_only_moved_in_the_reading_order_is_reported_as_such() {
    // the case no other tool reports: byte-identical, but it reads
    // somewhere else now because what it depends on changed
    let app = dependent_pair();
    let sources = Sources::new();
    let mut prev = snaps_of(&app, &sources);
    // same content, different position last time
    for v in prev.values_mut() {
        v.p += 5;
    }
    let order: Vec<usize> = (0..app.items.len()).collect();
    let (_, deltas) = compare_runs(&app.items, &order, &sources, &prev);
    assert!(deltas.iter().all(|d| *d == Delta::Moved), "{deltas:?}");
}

#[test]
fn a_run_identical_to_the_last_one_reports_nothing() {
    let app = dependent_pair();
    let sources = Sources::new();
    let prev = snaps_of(&app, &sources);
    let order: Vec<usize> = (0..app.items.len()).collect();
    let (_, deltas) = compare_runs(&app.items, &order, &sources, &prev);
    assert!(deltas.iter().all(|d| *d == Delta::Same), "{deltas:?}");
}

#[test]
fn a_hunk_with_no_previous_snapshot_is_new() {
    let app = dependent_pair();
    let sources = Sources::new();
    let order: Vec<usize> = (0..app.items.len()).collect();
    let (_, deltas) = compare_runs(&app.items, &order, &sources, &HashMap::new());
    assert!(deltas.iter().all(|d| *d == Delta::New), "{deltas:?}");
}

#[test]
fn a_changed_dependency_set_counts_as_moved() {
    let app = dependent_pair();
    let sources = Sources::new();
    let mut prev = snaps_of(&app, &sources);
    for v in prev.values_mut() {
        v.d = vec!["something-else".to_string()];
    }
    let order: Vec<usize> = (0..app.items.len()).collect();
    let (_, deltas) = compare_runs(&app.items, &order, &sources, &prev);
    assert!(deltas.contains(&Delta::Moved), "{deltas:?}");
}

#[test]
fn approving_a_use_before_its_definition_is_reported() {
    let mut app = dependent_pair();
    app.reviewed[1] = true; // the caller, not the callee
    let out = unreviewed_deps(&app, 1);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(app.items[out[0]].path, "a.py", "{out:?}");
}

#[test]
fn reviewing_the_definition_first_says_nothing() {
    let mut app = dependent_pair();
    app.reviewed[0] = true;
    app.reviewed[1] = true;
    assert!(unreviewed_deps(&app, 1).is_empty());
}

/// The binds as `view::keys` sends them: (prefix, key, action name).
fn sent_binds(preset: &str) -> Vec<(Option<String>, String, String)> {
    crate::web::view::keys(&keymap(preset).unwrap())
        .into_iter()
        .map(|b| {
            (
                b["prefix"].as_str().map(str::to_string),
                b["key"].as_str().unwrap().to_string(),
                b["action"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn sends(
    binds: &[(Option<String>, String, String)],
    prefix: Option<&str>,
    key: &str,
) -> Option<String> {
    binds
        .iter()
        .find(|(p, k, _)| p.as_deref() == prefix && k == key)
        .map(|(_, _, a)| a.clone())
}

/// `]` opens the `]f` chord yet still names its own action, which the browser
/// runs when no chord follows.
#[test]
fn a_bracket_is_both_a_chord_prefix_and_a_key() {
    let vim = sent_binds("vim");
    assert_eq!(sends(&vim, Some("]"), "f").as_deref(), Some("finding-next"));
    assert_eq!(sends(&vim, Some("["), "f").as_deref(), Some("finding-prev"));
    assert_eq!(sends(&vim, None, "]").as_deref(), Some("mark-next"));
    assert_eq!(sends(&vim, None, "[").as_deref(), Some("mark-prev"));
    assert_eq!(sends(&vim, Some("]"), "j"), None);
    let vs = sent_binds("vscode");
    assert_eq!(sends(&vs, None, "F8").as_deref(), Some("finding-next"));
    assert_eq!(sends(&vs, None, "S-F8").as_deref(), Some("finding-prev"));
}

#[test]
fn vim_takes_arrows_after_a_window_chord() {
    // vim accepts arrows wherever it accepts hjkl; a reviewer who reaches
    // for C-w Right should land where C-w l lands
    let vim = sent_binds("vim");
    for (arrow, letter, want) in [
        ("Left", "h", "focus-list"),
        ("Right", "l", "focus-code"),
        ("Up", "k", "focus-code"),
        ("Down", "j", "focus-why"),
    ] {
        assert_eq!(
            sends(&vim, Some("C-w"), arrow).as_deref(),
            Some(want),
            "{arrow}"
        );
        assert_eq!(
            sends(&vim, Some("C-w"), letter).as_deref(),
            Some(want),
            "{letter}"
        );
    }
}

/// One wave in view: a dep into another wave still reaches the browser with
/// that hunk's wave and mark, so the dep line can say which and follow it.
#[test]
fn a_dep_into_another_wave_is_labelled_and_followed_across() {
    let mut app = test_app();
    app.items = vec![test_item("a.rs"), test_item("b.rs")];
    app.items[0].wave = Some(2);
    app.items[1].wave = Some(1);
    app.items[0].edges = vec![edge("← b.rs:L1   uses it", Some(1))];
    app.view = vec![0, 1];
    app.reviewed = vec![false, true];
    set_filters(&mut app, false, true, None, Some(2)).unwrap();
    let r = crate::web::view::review(&app);
    assert_eq!(r["view"], serde_json::json!([0]));
    assert_eq!(r["only_wave"], 2);
    let target = r["items"][0]["edges"][0]["target"].as_u64().unwrap() as usize;
    assert_eq!(target, 1);
    assert_eq!(r["items"][target]["wave"], 1);
    assert_eq!(r["items"][target]["reviewed"], true);
}

/// A finding whose level `rules.toml` moved carries the level it was written
/// with, so the browser can say which way it moved.
#[test]
fn a_moved_level_names_where_it_came_from() {
    use ordo_diff::model::Level;
    let mut app = test_app();
    let mut lowered = finding(Level::Note);
    lowered.original_level = Some(Level::Warn);
    let mut raised = finding(Level::Verdict);
    raised.original_level = Some(Level::Warn);
    app.items[0].findings = vec![lowered, raised, finding(Level::Note)];
    let r = crate::web::view::review(&app);
    let f = &r["items"][0]["findings"];
    assert_eq!(
        (&f[0]["level"], &f[0]["original_level"]),
        (&"note".into(), &"warn".into())
    );
    assert_eq!(
        (&f[1]["level"], &f[1]["original_level"]),
        (&"verdict".into(), &"warn".into())
    );
    assert!(f[2].get("original_level").is_none(), "{f}");
    assert_eq!(r["items"][0]["level"], "verdict");
}

#[test]
fn an_unreviewed_hunk_is_never_out_of_order() {
    let mut app = dependent_pair();
    assert!(out_of_order(&app, 1).is_empty(), "nothing approved yet");
    app.reviewed[1] = true;
    assert_eq!(out_of_order(&app, 1), unreviewed_deps(&app, 1));
    assert_eq!(out_of_order(&app, 1).len(), 1);
}

#[test]
fn edge_coverage_needs_both_ends_reviewed() {
    let mut app = dependent_pair();
    assert_eq!(coverage(&app), (0, 2, 0, 1));
    app.reviewed[1] = true;
    // one hunk done, but the link between them is still unchecked
    assert_eq!(coverage(&app), (1, 2, 0, 1));
    app.reviewed[0] = true;
    assert_eq!(coverage(&app), (2, 2, 1, 1));
}

#[test]
fn an_edge_leaving_the_view_is_not_counted_against_it() {
    // filtering the review must not make coverage look better than it is
    let mut app = dependent_pair();
    app.view = vec![1];
    let (_, _, _, edges) = coverage(&app);
    assert_eq!(edges, 0);
}

#[test]
fn a_note_key_ignores_everything_a_rebase_can_move() {
    // same symbol, different file, different lines, different content —
    // a line anchor would be lost, the note must not be
    let mut a = item_with_symbol("api.py", "fetch");
    a.new_range = [10, 12];
    let mut b = item_with_symbol("moved/elsewhere.py", "fetch");
    b.new_range = [900, 902];
    b.rationale = "totally different".to_string();
    assert_eq!(note_key(&a), note_key(&b));
}

#[test]
fn a_note_key_separates_two_symbols_that_share_a_name() {
    // name alone is not identity: kind and scope are part of it
    let mut a = item_with_symbol("a.py", "run");
    a.symbols = vec![sym("run", "function_definition", None)];
    let mut b = item_with_symbol("a.py", "run");
    b.symbols = vec![sym("run", "function_definition", Some("Worker"))];
    assert_ne!(note_key(&a), note_key(&b));
}

#[test]
fn a_hunk_with_no_symbol_cannot_be_anchored_to() {
    // anchoring to the enclosing name would silently drift
    let it = test_item("a.py");
    assert!(it.symbols.is_empty());
    assert!(note_key(&it).is_none());
}

#[test]
fn a_rename_maps_the_old_identity_onto_the_new_one() {
    // what `load` uses to carry a note across `renames parse_cfg → load_cfg`
    let new = item_with_symbol("cfg.py", "load_cfg");
    let old = item_with_symbol("cfg.py", "parse_cfg");
    assert_eq!(note_key_named(&new, "parse_cfg"), note_key(&old));
    assert_ne!(note_key_named(&new, "parse_cfg"), note_key(&new));
}

#[test]
fn notes_round_trip_through_the_cache_file() {
    let dir = std::env::temp_dir().join(format!("ordo-notes-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("n.json");
    let mut notes = HashMap::new();
    notes.insert(42u64, "check the retry path".to_string());
    save_notes(&path, &notes).unwrap();
    assert_eq!(load_notes(&path), notes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_note_file_is_not_an_error() {
    let missing = std::env::temp_dir().join("ordo-notes-does-not-exist.json");
    assert!(load_notes(&missing).is_empty());
}

// ---- reviewed-mark persistence ----

// A known FNV-1a 64-bit test vector, hard-coded so a future refactor that
// swaps in a different hash (or a different offset/prime) fails loudly
// instead of silently invalidating every mark ever written.
#[test]
fn fnv1a_is_stable_for_a_known_input() {
    assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
    assert_eq!(fnv1a(b"hello"), 0xa430d84680aabd0b);
}

fn item_with(
    path: &str,
    old: [usize; 2],
    new: [usize; 2],
    symbols: Vec<Symbol>,
    enclosing: Option<&str>,
) -> Item {
    let mut it = test_item(path);
    it.old_range = old;
    it.new_range = new;
    it.symbols = symbols;
    it.enclosing = enclosing.map(str::to_string);
    it
}

fn sources_for(path: &str, old: &[&str], new: &[&str]) -> Sources {
    let mut s: Sources = HashMap::new();
    s.insert(path.to_string(), (lines(old), lines(new)));
    s
}

#[test]
fn mark_key_changes_when_hunk_content_changes() {
    let it_a = item_with(
        "f.rs",
        [1, 1],
        [1, 1],
        vec![sym("run", "function_item", None)],
        None,
    );
    let src_a = sources_for("f.rs", &["fn run() {}"], &["fn run() { 1 }"]);
    let src_b = sources_for("f.rs", &["fn run() {}"], &["fn run() { 2 }"]);
    let ka = mark_key("HEAD", &it_a, &src_a).unwrap();
    let kb = mark_key("HEAD", &it_a, &src_b).unwrap();
    assert_ne!(
        ka, kb,
        "a body edit must drop the mark, never carry it over silently"
    );
}

#[test]
fn mark_key_covers_both_old_and_new_sides() {
    // same new content, different old content (e.g. a reformat that
    // happens to converge) — still a different key
    let it = item_with("f.rs", [1, 1], [1, 1], vec![], Some("run"));
    let src_a = sources_for("f.rs", &["fn run() { 1 }"], &["fn run() {\n    1\n}"]);
    let src_b = sources_for("f.rs", &["fn run() { 2 }"], &["fn run() {\n    1\n}"]);
    assert_ne!(
        mark_key("HEAD", &it, &src_a).unwrap(),
        mark_key("HEAD", &it, &src_b).unwrap()
    );
}

#[test]
fn mark_key_is_unchanged_by_reordering_symbols() {
    let syms_a = vec![
        sym("a", "function_item", None),
        sym("b", "function_item", None),
    ];
    let syms_b = vec![
        sym("b", "function_item", None),
        sym("a", "function_item", None),
    ];
    let it_a = item_with("f.rs", [1, 1], [1, 2], syms_a, None);
    let it_b = item_with("f.rs", [1, 1], [1, 2], syms_b, None);
    let src = sources_for("f.rs", &["old"], &["fn a() {}", "fn b() {}"]);
    assert_eq!(
        mark_key("HEAD", &it_a, &src).unwrap(),
        mark_key("HEAD", &it_b, &src).unwrap()
    );
}

#[test]
fn mark_key_falls_back_to_enclosing_when_no_symbols() {
    // a body-edit hunk (no `symbols`) still gets a key, via `enclosing`
    let it = item_with("f.rs", [1, 1], [1, 1], vec![], Some("A.run"));
    let src = sources_for("f.rs", &["old"], &["new"]);
    assert!(mark_key("HEAD", &it, &src).is_some());
}

#[test]
fn mark_key_none_without_source_content() {
    let it = item_with("f.rs", [1, 1], [1, 1], vec![], None);
    let empty: Sources = HashMap::new();
    assert!(mark_key("HEAD", &it, &empty).is_none());
}

#[test]
fn mark_key_differs_across_rev_and_path() {
    let it = item_with(
        "f.rs",
        [1, 1],
        [1, 1],
        vec![sym("run", "function_item", None)],
        None,
    );
    let src = sources_for("f.rs", &["old"], &["new"]);
    let k1 = mark_key("HEAD", &it, &src).unwrap();
    let k2 = mark_key("abc123", &it, &src).unwrap();
    assert_ne!(k1, k2, "different revs must not collide");

    let it2 = item_with(
        "g.rs",
        [1, 1],
        [1, 1],
        vec![sym("run", "function_item", None)],
        None,
    );
    let mut src2 = src.clone();
    src2.insert("g.rs".to_string(), src2["f.rs"].clone());
    let k3 = mark_key("HEAD", &it2, &src2).unwrap();
    assert_ne!(
        k1, k3,
        "different paths must not collide even with identical content/symbol"
    );
}

#[test]
fn prune_marks_drops_old_entries_and_keeps_recent() {
    let now = 1_000_000_000u64;
    let mut marks: HashMap<u64, u64> = HashMap::new();
    marks.insert(1, now); // just written
    marks.insert(2, now - MARK_TTL_SECS + 10); // just inside the window
    marks.insert(3, now - MARK_TTL_SECS - 10); // just outside — dropped
    marks.insert(4, 0); // ancient — dropped
    prune_marks(&mut marks, now);
    assert_eq!(marks.len(), 2);
    assert!(marks.contains_key(&1));
    assert!(marks.contains_key(&2));
    assert!(!marks.contains_key(&3));
    assert!(!marks.contains_key(&4));
}

#[test]
fn a_save_never_leaves_the_file_half_written() {
    // `fs::write` truncates in place; a process killed mid-write left an
    // empty marks file, losing every mark ever made
    let dir = std::env::temp_dir().join(format!("ordo-atomic-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("m.json");
    let mut marks: HashMap<u64, u64> = HashMap::new();
    marks.insert(7, 1234);
    save_marks(&path, &marks).unwrap();
    assert_eq!(load_marks(&path), marks);
    // the temporary is renamed, never left behind
    let strays: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(strays.is_empty(), "{strays:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_mark_that_cannot_be_written_says_so() {
    // a file where the directory should be: nothing can be written under it
    let dir = std::env::temp_dir().join(format!("ordo-unwritable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "").unwrap();
    let marks: HashMap<u64, u64> = HashMap::from([(7, 1)]);
    assert!(save_marks(&blocker.join("m.json"), &marks).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_marks_degrades_to_empty_on_a_missing_or_corrupt_file() {
    let dir = std::env::temp_dir().join(format!("ordo-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let missing = dir.join("missing.json");
    assert!(load_marks(&missing).is_empty());

    let corrupt = dir.join("corrupt.json");
    std::fs::write(&corrupt, b"not json").unwrap();
    assert!(load_marks(&corrupt).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_marks_then_load_marks_round_trips() {
    let dir = std::env::temp_dir().join(format!("ordo-test-roundtrip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("nested").join("marks.json");
    let mut marks: HashMap<u64, u64> = HashMap::new();
    marks.insert(0xdeadbeef, 123);
    marks.insert(0x1, 456);
    save_marks(&path, &marks).unwrap();
    let got = load_marks(&path);
    assert_eq!(got, marks);

    // the file on disk reveals nothing about the code under review — no
    // path, symbol name, or source text, only hex keys and timestamps
    let text = std::fs::read_to_string(&path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let obj = value.as_object().unwrap();
    assert_eq!(obj.len(), 2);
    for (k, v) in obj {
        assert!(
            u64::from_str_radix(k, 16).is_ok(),
            "key must be plain hex: {k}"
        );
        assert!(v.is_u64(), "value must be a plain timestamp: {v}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- negative globs ----

fn filt(pats: &[&str]) -> Filter {
    let globs = build_globs(&pats.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
    Filter {
        globs,
        skip_generated: false,
        negatives_emptied: std::cell::Cell::new(false),
        tally: std::cell::Cell::new(Ledger::default()),
    }
}

#[test]
fn positives_only_keep_only_matching_paths() {
    let f = filt(&["src/*"]);
    assert!(f.keep("src/a.rs"));
    assert!(!f.keep("tests/a.rs"));
}

#[test]
fn negatives_only_keep_everything_except_those() {
    let f = filt(&["!tests/*"]);
    assert!(f.keep("src/a.rs"));
    assert!(f.keep("README.md"));
    assert!(!f.keep("tests/a.rs"));
}

#[test]
fn positive_and_negative_combine_as_and() {
    let f = filt(&["src/*", "!src/generated/*"]);
    assert!(f.keep("src/a.rs"));
    assert!(!f.keep("src/generated/x.rs"));
    assert!(!f.keep("tests/a.rs"), "outside the positive set entirely");
}

#[test]
fn a_negative_excludes_a_path_a_positive_also_matches_regardless_of_order() {
    // deliberately unlike .gitignore: order on the command line never
    // matters, and a later positive can never re-include what a negative
    // excluded
    let f = filt(&["!src/a.rs", "src/*"]);
    assert!(!f.keep("src/a.rs"));
    let f2 = filt(&["src/*", "!src/a.rs"]);
    assert!(!f2.keep("src/a.rs"));
}

#[test]
fn escaped_bang_is_a_literal_positive_pattern() {
    let f = filt(&["\\!weird"]);
    assert!(f.keep("!weird"));
    assert!(!f.keep("weird"));
}

#[test]
fn apply_note_distinguishes_negatives_emptying_it_from_no_positive_match() {
    let f = filt(&["!src/*"]);
    let kept = f.apply(vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
    assert!(kept.is_empty());
    assert_eq!(
        f.note(),
        " (every matching path was excluded by a negative glob)"
    );

    let f2 = filt(&["nomatch/*"]);
    let kept2 = f2.apply(vec!["src/a.rs".to_string()]);
    assert!(kept2.is_empty());
    assert_eq!(f2.note(), " matching the given globs");
}

#[test]
fn filter_command_glob_type_supports_negatives_too() {
    // `:filter` reuses `build_globs`, so a single negative pattern like
    // `!tests/*` narrows to "everything except tests" the same as the CLI
    let globs = build_globs(&["!tests/*".to_string()]).unwrap();
    let mut a = test_item("src/a.rs");
    a.comment = false;
    let items = vec![test_item("src/a.rs"), test_item("tests/b.rs")];
    assert_eq!(
        compute_view(&items, false, true, Some(&globs), None),
        vec![0]
    );
}

// ---- theme selection ----

#[test]
fn theme_selects_dark_and_light_by_name() {
    assert!(theme("dark").is_some());
    assert!(theme("light").is_some());
}

#[test]
fn theme_rejects_an_unknown_name() {
    assert!(theme("nonsense").is_none());
}

#[test]
fn light_theme_is_not_the_dark_values_inverted() {
    let dark = theme("dark").unwrap();
    let light = theme("light").unwrap();
    // a real, distinct palette — not a placeholder equal to dark, and not
    // literally 255-x of dark's channels either
    assert!(!colors_eq(dark.add_bg, light.add_bg));
    let Color::Rgb(dr, dg, db) = dark.add_bg else {
        panic!("dark add_bg not Rgb")
    };
    let Color::Rgb(lr, lg, lb) = light.add_bg else {
        panic!("light add_bg not Rgb")
    };
    assert!(
        !(lr == 255 - dr && lg == 255 - dg && lb == 255 - db),
        "not a bitwise inversion"
    );
}

fn colors_eq(a: Color, b: Color) -> bool {
    matches!((a, b), (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) if ar == br && ag == bg && ab == bb)
}

// ---- :group header rows ----

fn grouped_item(path: &str, group: &str) -> Item {
    let mut it = test_item(path);
    it.group = group.to_string();
    // hunk mode: the list buckets by group id
    it.bucket = group.to_string();
    it
}

/// two hunks in `g0` and one in `g1`, with the symbols their groups show
fn grouped_trio() -> (Vec<Item>, HashMap<String, String>) {
    let items = vec![
        grouped_item("a.rs", "g0"),
        grouped_item("a.rs", "g0"),
        grouped_item("b.rs", "g1"),
    ];
    let groups = HashMap::from([
        ("g0".to_string(), "run".to_string()),
        ("g1".to_string(), String::new()),
    ]);
    (items, groups)
}

/// `display_rows` as words, so a test can say what the list looks like
fn row_kinds(rows: &[DisplayRow]) -> Vec<String> {
    rows.iter()
        .map(|r| match r {
            DisplayRow::File(path, n) => format!("file {path} {n}"),
            DisplayRow::Gap => "gap".to_string(),
            DisplayRow::Item(i, true) => format!("item {i} opens"),
            DisplayRow::Item(i, false) => format!("item {i}"),
            DisplayRow::Folded(hidden) => format!("folded {hidden:?}"),
        })
        .collect()
}

#[test]
fn display_rows_bands_each_file_and_names_a_group_on_its_first_row() {
    let (items, groups) = grouped_trio();
    let rows = display_rows(&[0, 1, 2], &items, &groups, true, &HashSet::new());
    assert_eq!(
        row_kinds(&rows),
        [
            "file a.rs 2",
            "item 0 opens",
            "item 1",
            "file b.rs 1",
            "item 2 opens"
        ]
    );
}

#[test]
fn a_gap_parts_top_level_code_only_from_a_named_group_above_it() {
    let items = vec![
        grouped_item("a.rs", "top"),
        grouped_item("a.rs", "run"),
        grouped_item("a.rs", "run"),
        grouped_item("a.rs", "top2"),
        grouped_item("a.rs", "top3"),
        grouped_item("a.rs", "main"),
    ];
    let groups = HashMap::from([
        ("top".to_string(), String::new()),
        ("top2".to_string(), String::new()),
        ("top3".to_string(), String::new()),
        ("run".to_string(), "run".to_string()),
        ("main".to_string(), "main".to_string()),
    ]);
    let rows = display_rows(&[0, 1, 2, 3, 4, 5], &items, &groups, true, &HashSet::new());
    // none after the band, none before a named group, none between two
    // top-level groups: only where top-level code would read as `run`'s
    assert_eq!(
        row_kinds(&rows),
        [
            "file a.rs 6",
            "item 0 opens",
            "item 1 opens",
            "item 2",
            "gap",
            "item 3 opens",
            "item 4 opens",
            "item 5 opens",
        ]
    );
}

// ---- reviewing rules: the client half ----

#[test]
fn rules_come_from_the_user_then_the_repository() {
    let srcs = rule_sources("/repo");
    assert_eq!(srcs.len(), 2);
    assert!(srcs[0].ends_with("ordo/rules.toml"), "{:?}", srcs[0]);
    assert_eq!(srcs[1], PathBuf::from("/repo/.ordo/rules.toml"));
}

#[test]
fn a_rules_file_parses_into_rules() {
    let (rules, problems) = parse_rules(
        "[[rule]]\nname = \"security-first\"\npath = \"src/security/**\"\nnote = \"sensitive\"\npriority = 100\n\n             [[rule]]\nname = \"vendored\"\npath = \"vendor/**\"\nnoise = true\n",
        Path::new("."),
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].name, "security-first");
    assert_eq!(rules[0].when.path.as_deref(), Some("src/security/**"));
    assert_eq!(rules[0].note.as_deref(), Some("sensitive"));
    assert_eq!(rules[0].priority, 100);
    assert!(rules[1].noise);
}

#[test]
fn a_rule_without_a_name_is_a_problem_not_a_silent_default() {
    // the old line-based parser couldn't tell "missing" from "empty" and
    // papered over it with an auto name (`rule-1`); real TOML makes `name`
    // a required field, so a rule without one fails to convert and is
    // reported, rather than kept under a name nobody wrote
    let (rules, problems) =
        parse_rules("[[rule]]\npath = \"a/**\"\nnote = \"n\"\n", Path::new("."));
    assert!(rules.is_empty(), "{rules:?}");
    assert!(problems[0].contains("missing field `name`"), "{problems:?}");
}

#[test]
fn a_bad_rule_is_reported_and_dropped_not_partially_applied() {
    // the old parser evaluated each `key = value` line independently, so
    // a rule with a bad line still got kept with whatever lines *did*
    // parse, plus a problem per bad line. A typed table deserializes
    // atomically: an unknown key fails the whole rule, one problem,
    // nothing partially applied
    let (rules, problems) =
        parse_rules("[[rule]]\nname = \"a\"\nnonsense = \"x\"\n", Path::new("."));
    assert!(rules.is_empty(), "{rules:?}");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("nonsense"), "{problems:?}");
}

#[test]
fn one_broken_rule_does_not_sink_the_others() {
    let (rules, problems) = parse_rules(
        "[[rule]]\nname = \"bad\"\npriority = \"soon\"\n\n[[rule]]\nname = \"good\"\npath = \"x/**\"\n",
        Path::new("."),
    );
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert_eq!(rules[0].name, "good");
    assert_eq!(rules[0].when.path.as_deref(), Some("x/**"));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("bad"), "{problems:?}");
}

#[test]
fn an_unknown_key_names_itself_in_the_problem() {
    let (rules, problems) = parse_rules("name = \"loose\"\n", Path::new("."));
    assert!(rules.is_empty());
    assert!(problems[0].contains("name"), "{problems:?}");
}

#[test]
fn a_query_can_live_in_its_own_file() {
    let dir = std::env::temp_dir().join(format!("ordo-rules-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let q = "(call function: (identifier) @fn)";
    std::fs::write(dir.join("q.scm"), q).unwrap();
    let (rules, problems) = parse_rules("[[rule]]\nname = \"q\"\nquery-file = \"q.scm\"\n", &dir);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(rules[0].when.query.as_deref(), Some(q));

    // and a missing one is reported rather than silently never matching
    let (_, problems) = parse_rules("[[rule]]\nname = \"q\"\nquery-file = \"nope.scm\"\n", &dir);
    assert!(problems[0].contains("nope.scm"), "{problems:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_multi_line_query_and_a_kind_list_both_read() {
    let (rules, problems) = parse_rules(
        "[[rule]]\nname = \"loop-shapes\"\nkind = [\"for_statement\", \"while_statement\"]\nquery = '''\n(call\n  function: (identifier) @f)\n'''\n",
        Path::new("."),
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        rules[0].when.kind.as_deref(),
        Some(&["for_statement".to_string(), "while_statement".to_string()][..])
    );
    assert_eq!(
        rules[0].when.query.as_deref(),
        Some("(call\n  function: (identifier) @f)\n")
    );
}

#[test]
fn kind_as_a_bare_string_also_reads_as_a_one_entry_list() {
    let (rules, problems) = parse_rules(
        "[[rule]]\nname = \"one-kind\"\nkind = \"for_statement\"\n",
        Path::new("."),
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        rules[0].when.kind.as_deref(),
        Some(&["for_statement".to_string()][..])
    );
}

#[test]
fn kebab_case_keys_reach_the_matching_when_fields() {
    let (rules, problems) = parse_rules(
        "[[rule]]\nname = \"limits\"\npath-not = \"vendor/**\"\nmax-params = 4\ncontainer-without = \"Drop\"\nmember-uninitialized = true\n",
        Path::new("."),
    );
    assert!(problems.is_empty(), "{problems:?}");
    let w = &rules[0].when;
    assert_eq!(w.path_not.as_deref(), Some("vendor/**"));
    assert_eq!(w.max_params, Some(4));
    assert_eq!(w.container_without.as_deref(), Some("Drop"));
    assert_eq!(w.member_uninitialized, Some(true));
}

#[test]
fn a_rules_flag_file_is_layered_last_and_a_missing_one_is_a_problem() {
    let dir = std::env::temp_dir().join(format!("ordo-rules-flag-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let extra = dir.join("extra.toml");
    std::fs::write(
        &extra,
        "[[rule]]\nname = \"from-flag\"\nkind = \"type_definition\"\nnote = \"n\"\n",
    )
    .unwrap();
    let (rules, problems) = load_rules("", &[extra.to_string_lossy().into_owned()]);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(rules.iter().any(|r| r.name == "from-flag"));
    // the implicit user/repo files may be absent; a file named on the
    // command line was asked for, so its absence is reported
    let (_, problems) = load_rules("", &[dir.join("nope.toml").to_string_lossy().into_owned()]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("nope.toml"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_shipped_ruleset_is_a_bundled_preset() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("rulesets");
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|x| x == "toml") {
            let stem = p.file_stem().unwrap().to_str().unwrap();
            assert!(
                preset(stem).is_some(),
                "rulesets/{stem}.toml is not in PRESETS"
            );
        }
    }
    for (name, text) in PRESETS {
        let d = parse_rules_doc(text, Path::new("."));
        assert!(d.problems.is_empty(), "{name}: {:?}", d.problems);
    }
}

fn rules_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ordo-rules-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_included_preset_layers_first_and_a_same_named_rule_replaces_its_entry() {
    let dir = rules_dir("include");
    let mine = dir.join("rules.toml");
    std::fs::write(&mine, concat!(
        "include = [\"go-uber-guide\"]\n",
        "[[rule]]\nname = \"no-panic\"\nlang = \"go\"\nuses = \"panic\"\nnote = \"ours: panic is fine in main\"\n",
        "[[rule]]\nname = \"no-cgo\"\nlang = \"go\"\nimports = \"C\"\nwarn = \"cgo\"\n",
    )).unwrap();
    let r = report_from(vec![], &[mine.to_string_lossy().into_owned()]);
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    let preset_len = parse_rules_doc(preset("go-uber-guide").unwrap(), Path::new("."))
        .rules
        .len();
    assert_eq!(
        r.rules.len(),
        preset_len + 1,
        "one replaced in place, one added"
    );
    let np = r.rules.iter().find(|x| x.name == "no-panic").unwrap();
    assert_eq!(np.note.as_deref(), Some("ours: panic is fine in main"));
    assert_eq!(r.replaced.len(), 1);
    assert!(r.replaced[0].starts_with("no-panic"), "{:?}", r.replaced);
    assert_eq!(
        r.origins
            .iter()
            .find(|(o, _)| o == "go-uber-guide")
            .map(|(_, n)| *n),
        Some(preset_len - 1)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disables_apply_after_every_layer_so_an_earlier_file_can_silence_a_later_include() {
    let dir = rules_dir("disable");
    let user = dir.join("user.toml");
    let repo = dir.join("repo.toml");
    std::fs::write(&user, "disable = [\"no-init\", \"*-size\"]\n").unwrap();
    std::fs::write(&repo, "include = [\"go-uber-guide\"]\n").unwrap();
    let r = report_from(
        vec![],
        &[
            user.to_string_lossy().into_owned(),
            repo.to_string_lossy().into_owned(),
        ],
    );
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert!(r.rules.iter().all(|x| x.name != "no-init"));
    assert!(
        r.disabled.iter().any(|d| d.starts_with("no-init")),
        "{:?}",
        r.disabled
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn levels_in_rules_toml_reach_the_engine_and_a_bad_name_or_level_is_a_problem() {
    let dir = rules_dir("levels");
    let user = dir.join("user.toml");
    let repo = dir.join("repo.toml");
    std::fs::write(
        &user,
        "[levels]\nmagic-number = \"verdict\"\n\"eval/exec\" = \"warn\"\nmagic-numbr = \"note\"\n",
    )
    .unwrap();
    std::fs::write(
        &repo,
        "[levels]\nmagic-number = \"warn\"\nassert-validation = \"loud\"\n",
    )
    .unwrap();
    let r = report_from(
        vec![],
        &[
            user.to_string_lossy().into_owned(),
            repo.to_string_lossy().into_owned(),
        ],
    );
    let levels = r.rule_set(true).levels;
    assert_eq!(
        levels.into_iter().collect::<Vec<_>>(),
        vec![
            ("eval/exec".to_string(), ordo_diff::model::Level::Warn),
            ("magic-number".to_string(), ordo_diff::model::Level::Warn),
            ("magic-numbr".to_string(), ordo_diff::model::Level::Note),
        ],
        "a later file wins, and a bad level is dropped"
    );
    assert!(
        r.problems
            .iter()
            .any(|p| p == "levels: no rule named `magic-numbr`"),
        "{:?}",
        r.problems
    );
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("`assert-validation` = \"loud\" is not a level")),
        "{:?}",
        r.problems
    );
    assert!(r.lines(false).iter().any(|l| l == "  magic-number → warn"));

    // and the engine applies them: the finding moves and keeps where it was
    let out = ordo_diff::run(ordo_diff::model::Input {
        changes: vec![ordo_diff::model::Change {
            path: "a.py".to_string(),
            old: Some(String::new()),
            new: Some("def f(x):\n    assert x > 0\n".to_string()),
            diff: None,
        }],
        options: Options {
            levels: [(
                "assert-validation".to_string(),
                ordo_diff::model::Level::Note,
            )]
            .into(),
            ..Options::default()
        },
        consumers: vec![],
    });
    let f = out.files[0].hunks[0]
        .findings
        .iter()
        .find(|f| f.name == "assert-validation")
        .expect("assert-validation fired");
    assert_eq!(f.level, ordo_diff::model::Level::Note);
    assert_eq!(f.original_level, Some(ordo_diff::model::Level::Verdict));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A level row cycles default → note → warn → verdict, moves the live
/// session, and `w` writes only the names that changed into `[levels]`.
#[test]
fn a_level_row_changes_the_session_and_writes_to_the_levels_table() {
    let mut app = test_app();
    app.levels = [("magic-number".to_string(), ordo_diff::model::Level::Verdict)].into();
    app.config = Some(ConfigUi::open(&app));
    let at = |app: &App, key: &str| {
        let c = app.config.as_ref().unwrap();
        c.rows
            .iter()
            .position(|&(i, j)| c.sections[i].fields[j].key == key)
            .unwrap_or_else(|| panic!("no {key}"))
    };
    app.config.as_mut().unwrap().sel = at(&app, "level:assert-validation");
    config_toggle(&mut app);
    assert_eq!(
        app.levels.get("assert-validation"),
        Some(&ordo_diff::model::Level::Note),
        "default → note"
    );
    // verdict wraps round to default: the override goes
    app.config.as_mut().unwrap().sel = at(&app, "level:magic-number");
    config_toggle(&mut app);
    assert!(!app.levels.contains_key("magic-number"), "{:?}", app.levels);

    let dir = rules_dir("level-write");
    let path = dir.join("rules.toml");
    let seed = "# mine\ndisable = []\n\n[levels]\nmagic-number = \"verdict\"\nmine = \"warn\"\n";
    std::fs::write(&path, seed).unwrap();
    let c = app.config.as_ref().unwrap();
    write_rules_file(&path, c, &c.changed()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text,
        "# mine\ndisable = []\n\n[levels]\n# magic-number = \"verdict\"\nmine = \"warn\"\nassert-validation = \"note\"\n"
    );
    let doc = parse_rules_doc(&text, Path::new("."));
    assert!(doc.problems.is_empty(), "{:?}", doc.problems);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_include_and_a_duplicate_name_are_problems() {
    let dir = rules_dir("problems");
    let f = dir.join("rules.toml");
    std::fs::write(
        &f,
        concat!(
            "include = [\"./nope.toml\"]\n",
            "[[rule]]\nname = \"twice\"\nnote = \"a\"\n",
            "[[rule]]\nname = \"twice\"\nnote = \"b\"\n",
        ),
    )
    .unwrap();
    let r = report_from(vec![], &[f.to_string_lossy().into_owned()]);
    assert_eq!(r.rules.len(), 1);
    assert!(
        r.problems.iter().any(|p| p.contains("nope.toml")),
        "{:?}",
        r.problems
    );
    assert!(
        r.problems.iter().any(|p| p.contains("defined twice")),
        "{:?}",
        r.problems
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_include_cycle_is_reported_not_looped() {
    let dir = rules_dir("cycle");
    let f = dir.join("rules.toml");
    std::fs::write(&f, "include = [\"./rules.toml\"]\n").unwrap();
    let r = report_from(vec![], &[f.to_string_lossy().into_owned()]);
    assert!(
        r.problems.iter().any(|p| p.contains("cycle")),
        "{:?}",
        r.problems
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rules_flag_names_a_preset_or_a_file() {
    let r = report_from(vec![], &["c-power-of-ten".to_string()]);
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert!(r.rules.iter().any(|x| x.name == "no-recursion"));
    assert_eq!(
        r.origins,
        vec![("c-power-of-ten".to_string(), r.rules.len())]
    );
    assert!(r.lines(false)[0].ends_with("rules active"));
}

#[test]
fn ordos_own_rules_file_loads_with_zero_problems() {
    let text = std::fs::read_to_string(".ordo/rules.toml").expect("repo has .ordo/rules.toml");
    let (rules, problems) = parse_rules(&text, Path::new(".ordo"));
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!rules.is_empty());
}

// ---- --init-config ----

#[test]
fn the_generated_config_is_one_the_program_accepts() {
    // uncommenting the whole file must parse with no complaints: a
    // generated config that ordo itself rejects is worse than none
    for (preset, theme_name) in [("vim", "dark"), ("vscode", "catppuccin-mocha")] {
        let text = init_config(preset, theme_name);
        let live: String = text
            .lines()
            .map(|l| match l.trim_start().strip_prefix("# ") {
                // a `#` line that looks like a setting is a commented-out
                // default; anything else is prose
                Some(rest) if rest.contains(" = ") => rest.to_string(),
                _ => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let cfg = parse_key_config(&live);
        assert!(
            cfg.problems.is_empty(),
            "{preset}/{theme_name}: {:?}",
            cfg.problems
        );
        assert_eq!(cfg.preset.as_deref(), Some(preset));
        assert_eq!(cfg.theme.as_deref(), Some(theme_name));
    }
}

#[test]
fn the_generated_config_changes_nothing_when_fully_uncommented() {
    // ...and the values it writes are the ones already in effect, so a
    // reviewer who uncomments everything sees no difference
    let text = init_config("vim", "catppuccin-mocha");
    // keep the section headers: uncommenting the settings without them
    // would file every line under the top level
    let live: String = text
        .lines()
        .filter_map(|l| match l.trim_start().strip_prefix("# ") {
            Some(rest) if rest.contains(" = ") => Some(rest.to_string()),
            _ if l.starts_with('[') => Some(l.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let cfg = parse_key_config(&live);
    assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);

    let base = keymap("vim").unwrap();
    let after = apply_key_config(keymap("vim").unwrap(), &cfg);
    assert_eq!(
        after.binds.len(),
        base.binds.len(),
        "no binding gained or lost"
    );
    for b in &base.binds {
        assert!(after.binds.contains(b), "{:?} was dropped", key_label(b.1));
    }

    let t0 = theme("catppuccin-mocha").unwrap();
    let t1 = apply_theme_colors(t0, &cfg.colors);
    for role in THEME_ROLES {
        assert!(
            colors_eq(theme_role_color(&t0, role), theme_role_color(&t1, role)),
            "role `{role}` changed"
        );
    }
}

#[test]
fn every_generated_binding_names_a_real_action() {
    let text = init_config("vscode", "nord");
    for line in text.lines().filter_map(|l| l.strip_prefix("# \"")) {
        let Some((_, rest)) = line.split_once("\" = \"") else {
            continue;
        };
        let Some((action, _)) = rest.split_once('"') else {
            continue;
        };
        assert!(
            action_by_name(action).is_some(),
            "`{action}` is not an action"
        );
    }
}

// ---- theming ----

#[test]
fn every_theme_name_resolves_and_truecolor_themes_leave_nothing_to_the_page() {
    for name in theme_names() {
        let t = theme(&name).expect(&name);
        assert_eq!(t.name, name, "a theme must know its own name");
        if ["dark", "light"].contains(&name.as_str()) {
            // the promise of a plain theme: text follows the page
            assert_eq!(t.fg, Color::Default, "{name}");
            continue;
        }
        // a truecolor theme names everything; a stray Default would show up
        // as one element mysteriously following the page instead
        let roles: Vec<(&str, Color)> = vec![
            ("fg", t.fg),
            ("dim", t.dim),
            ("border", t.border),
            ("border_focus", t.border_focus),
            ("accent", t.accent),
            ("reviewed", t.reviewed),
            ("warn", t.warn),
            ("add_bg", t.add_bg),
            ("del_bg", t.del_bg),
            ("add_strong_bg", t.add_strong_bg),
            ("del_strong_bg", t.del_strong_bg),
            ("select_bg", t.select_bg),
            ("match_bg", t.match_bg),
            ("match_cur_bg", t.match_cur_bg),
            ("band_bg", t.band_bg),
            ("verdict_bg", t.verdict_bg),
            ("verdict_strong_bg", t.verdict_strong_bg),
            ("warn_bg", t.warn_bg),
            ("warn_strong_bg", t.warn_strong_bg),
            ("note_bg", t.note_bg),
            ("note_strong_bg", t.note_strong_bg),
            ("reviewed_bg", t.reviewed_bg),
            ("syn.comment", t.syn.comment),
            ("syn.keyword", t.syn.keyword),
            ("syn.string", t.syn.string),
            ("syn.number", t.syn.number),
            ("syn.function", t.syn.function),
            ("syn.type", t.syn.type_),
            ("syn.property", t.syn.property),
            ("syn.operator", t.syn.operator),
            ("syn.variable", t.syn.variable),
            ("syn.builtin", t.syn.builtin),
            ("syn.param", t.syn.param),
            ("syn.attribute", t.syn.attribute),
        ];
        for (role, c) in roles {
            assert!(
                matches!(c, Color::Rgb(..)),
                "{name}: {role} is not truecolor"
            );
        }
    }
}

#[test]
fn a_diff_tint_is_distinguishable_from_the_selection_tint() {
    // the three tints a row can carry must not collapse into each other,
    // or an added line and a selected line look the same
    for name in theme_names() {
        let t = theme(&name).unwrap();
        assert!(!colors_eq(t.add_bg, t.del_bg), "{name}: add/del");
        assert!(!colors_eq(t.add_bg, t.select_bg), "{name}: add/select");
        assert!(!colors_eq(t.del_bg, t.select_bg), "{name}: del/select");
        assert!(
            !colors_eq(t.add_bg, t.add_strong_bg),
            "{name}: add/add-strong"
        );
        assert!(
            !colors_eq(t.del_bg, t.del_strong_bg),
            "{name}: del/del-strong"
        );
        assert!(
            !colors_eq(t.match_bg, t.match_cur_bg),
            "{name}: match/current"
        );
    }
}

#[test]
fn every_state_tint_is_its_own_colour() {
    // a row's state is its background and nothing else, so no two states
    // (nor a state and the selection, the band or a diff tint) may share one
    for name in theme_names() {
        let t = theme(&name).unwrap();
        let tints = [
            ("verdict", t.verdict_bg),
            ("verdict-strong", t.verdict_strong_bg),
            ("warn", t.warn_bg),
            ("warn-strong", t.warn_strong_bg),
            ("note", t.note_bg),
            ("note-strong", t.note_strong_bg),
            ("reviewed", t.reviewed_bg),
            ("band", t.band_bg),
            ("select", t.select_bg),
            ("add", t.add_bg),
            ("del", t.del_bg),
            ("match", t.match_bg),
        ];
        for (i, (a, ca)) in tints.iter().enumerate() {
            for (b, cb) in &tints[i + 1..] {
                assert!(!colors_eq(*ca, *cb), "{name}: {a}/{b}");
            }
        }
    }
}

#[test]
fn parse_hex_takes_the_form_palettes_publish_and_nothing_else() {
    assert_eq!(parse_hex("#89b4fa"), Some(Color::Rgb(0x89, 0xb4, 0xfa)));
    assert_eq!(parse_hex("89b4fa"), Some(Color::Rgb(0x89, 0xb4, 0xfa)));
    assert_eq!(parse_hex("  #000000 "), Some(Color::Rgb(0, 0, 0)));
    assert_eq!(parse_hex("#89b4f"), None, "five digits");
    assert_eq!(parse_hex("#89b4fag"), None, "not hex");
    assert_eq!(parse_hex("blue"), None, "colour names are not accepted");
}

#[test]
fn every_documented_theme_role_actually_changes_the_theme() {
    // THEME_ROLES is what the docs promise a config can set; a name listed
    // there but missing from `apply_theme_colors` would silently do nothing
    let base = theme("catppuccin-mocha").unwrap();
    let sentinel = Color::Rgb(1, 2, 3);
    for role in THEME_ROLES {
        let got = apply_theme_colors(base, &[(role.to_string(), sentinel)]);
        let changed = [
            got.fg,
            got.dim,
            got.border,
            got.border_focus,
            got.accent,
            got.reviewed,
            got.warn,
            got.add_bg,
            got.del_bg,
            got.add_strong_bg,
            got.del_strong_bg,
            got.select_bg,
            got.match_bg,
            got.match_cur_bg,
            got.band_bg,
            got.verdict_bg,
            got.verdict_strong_bg,
            got.warn_bg,
            got.warn_strong_bg,
            got.note_bg,
            got.note_strong_bg,
            got.reviewed_bg,
            got.syn.comment,
            got.syn.keyword,
            got.syn.string,
            got.syn.number,
            got.syn.function,
            got.syn.type_,
            got.syn.property,
            got.syn.operator,
            got.syn.variable,
            got.syn.builtin,
            got.syn.param,
            got.syn.attribute,
        ]
        .iter()
        .filter(|c| colors_eq(**c, sentinel))
        .count();
        assert_eq!(changed, 1, "role `{role}` set {changed} fields, expected 1");
        // and the read side must name the same field as the write side —
        // without this, `--init-config` can print one role's colour under
        // another role's name
        assert!(
            colors_eq(theme_role_color(&got, role), sentinel),
            "role `{role}` reads back a different field than it writes"
        );
    }
}

#[test]
fn a_config_can_name_a_theme_and_override_its_roles() {
    let cfg = parse_key_config(
        "[theme]\nname = \"nord\"\nborder-focus = \"#ff0000\"\nsyntax-keyword = \"00ff00\"\n",
    );
    assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
    assert_eq!(cfg.theme.as_deref(), Some("nord"));

    let t = apply_theme_colors(theme(&cfg.theme.clone().unwrap()).unwrap(), &cfg.colors);
    assert!(colors_eq(t.border_focus, Color::Rgb(0xff, 0, 0)));
    assert!(colors_eq(t.syn.keyword, Color::Rgb(0, 0xff, 0)));
    // everything not named keeps the palette's own value
    assert!(colors_eq(t.syn.string, theme("nord").unwrap().syn.string));
}

#[test]
fn a_bad_theme_line_is_reported_by_number_and_skipped() {
    let cfg = parse_key_config(
        "[theme]\nnmae = \"nord\"\nborder-focus = \"redish\"\nwarn = \"#ffcc00\"\n",
    );
    assert_eq!(cfg.problems.len(), 2, "{:?}", cfg.problems);
    assert!(cfg.problems[0].contains("line 2"), "{:?}", cfg.problems);
    assert!(cfg.problems[1].contains("line 3"), "{:?}", cfg.problems);
    assert_eq!(cfg.colors.len(), 1, "the good line still lands");
}

#[test]
fn a_hash_opens_a_comment_only_outside_quotes() {
    // every palette value starts with `#`; cutting at the first one
    // regardless would eat the whole theme section
    assert_eq!(
        strip_comment("warn = \"#ffcc00\"  # the warning colour"),
        "warn = \"#ffcc00\"  "
    );
    assert_eq!(strip_comment("# whole line"), "");
    assert_eq!(strip_comment("preset = \"vim\""), "preset = \"vim\"");

    let cfg = parse_key_config("[theme]\nwarn = \"#ffcc00\"  # trailing\n");
    assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
    assert_eq!(cfg.colors[0].0, "warn");
    assert!(colors_eq(cfg.colors[0].1, Color::Rgb(0xff, 0xcc, 0)));
}

#[test]
fn an_unknown_section_is_reported_rather_than_silently_ignored() {
    let cfg = parse_key_config("[colours]\nfg = \"#ffffff\"\n");
    assert!(
        cfg.problems[0].contains("unknown section"),
        "{:?}",
        cfg.problems
    );
}

/// `:strategy` on the strategy already in force must change nothing. It
/// used to rebuild the engine's input out of `app.sources`, where a
/// removed file (`new: None`) came back as an emptied one and every line
/// lost its trailing newline — so the group reasons changed and the
/// headers fell back to raw group ids.
#[test]
fn re_running_the_current_strategy_keeps_the_group_reasons() {
    let changes: Vec<Change> = serde_json::from_value(serde_json::json!([
        {"path": "gone.py", "old": "def helper(x):\n    return x\n", "new": null},
        {"path": "use.py",
         "old": "from gone import helper\n\ndef run():\n    return helper(1)\n",
         "new": "def run():\n    return 1\n"}
    ]))
    .expect("changes");
    let input = Input {
        changes: changes.clone(),
        options: Options {
            strategy: Strategy::Comprehension,
            cross_file: true,
            ..Options::default()
        },
        consumers: vec![],
    };
    let out = ordo_diff::run(input);
    let mut app = test_app();
    app.changes = changes;
    app.items = build_items(&out);
    app.view = (0..app.items.len()).collect();
    app.reviewed = vec![false; app.items.len()];
    // the app holds the same content split into lines; the review must not
    // be rebuilt out of it (that is the bug), so it is here to prove it is
    // not what `:strategy` reads
    app.sources = app
        .changes
        .iter()
        .map(|c| {
            let split = |t: &Option<String>| {
                t.as_ref()
                    .map(|t| t.lines().map(String::from).collect())
                    .unwrap_or_default()
            };
            (c.path.clone(), (split(&c.old), split(&c.new)))
        })
        .collect();
    app.group_reasons = group_reasons(&out);
    app.symbol_ledger = out.ledger.clone();
    set_mode(&mut app, ViewMode::Ledger, &out.ledger);
    app.strategy = "comprehension".to_string();
    let before = app.group_reasons.clone();
    let symbols = app.groups.clone();
    assert!(!before.is_empty(), "the fixture has to group something");

    run_strategy(&mut app, "comprehension").expect("a no-op re-order");
    assert_eq!(
        app.group_reasons, before,
        "a no-op re-order changed the reasons"
    );
    assert_eq!(app.groups, symbols, "a no-op re-order renamed the groups");
}

/// The role rows and the palette are one setting between them: cycling the
/// theme must not carry the old palette's colours over, and must not throw
/// the reviewer's own overrides away either.
#[test]
fn config_keeps_role_overrides_across_a_theme_swap_and_applies_an_edit() {
    let mut app = test_app();
    let red = Color::Rgb(0xff, 0, 0);
    app.theme = apply_theme_colors(theme("dark").unwrap(), &[("accent".to_string(), red)]);
    app.config = Some(ConfigUi::open(&app));

    let row = |app: &App, key: &str| {
        let c = app.config.as_ref().unwrap();
        c.rows
            .iter()
            .position(|&(i, j)| c.sections[i].fields[j].key == key)
            .unwrap_or_else(|| panic!("no `{key}` row"))
    };
    // only the override is materialised; a role following its palette is
    // blank, or the swap below would paste dark's colours onto the next one
    let untouched: Vec<&str> = THEME_ROLES
        .iter()
        .copied()
        .filter(|r| *r != "accent")
        .collect();
    let c = app.config.as_ref().unwrap();
    assert_eq!(
        c.field(row(&app, "theme.accent")).unwrap().kind,
        FieldKind::Text("#ff0000".to_string())
    );
    for r in &untouched {
        assert_eq!(
            c.field(row(&app, &format!("theme.{r}"))).unwrap().kind,
            FieldKind::Text(String::new()),
            "`{r}` follows the palette and must read as blank"
        );
    }

    let at = row(&app, "theme");
    app.config.as_mut().unwrap().sel = at;
    config_toggle(&mut app);
    assert_ne!(app.theme.name, "dark", "cycling picks another palette");
    assert_eq!(
        theme_role_color(&app.theme, "accent"),
        red,
        "the reviewer's override was thrown away by the swap"
    );
    let next = theme(app.theme.name).expect("the cycled-to palette");
    for r in &untouched {
        assert_eq!(
            theme_role_color(&app.theme, r),
            theme_role_color(&next, r),
            "`{r}` kept dark's colour instead of following the new palette"
        );
    }

    // and editing a role applies without waiting for a restart
    let at = row(&app, "theme.border-focus");
    let c = app.config.as_mut().unwrap();
    c.sel = at;
    c.editing = Some("#00ff00".to_string());
    config_commit_edit(&mut app);
    assert_eq!(
        theme_role_color(&app.theme, "border-focus"),
        Color::Rgb(0, 0xff, 0)
    );
}

// ---- configurable keybinds ----

#[test]
fn every_action_has_a_config_name_and_every_name_resolves() {
    // the table is the only way to name an action in a config file: an
    // action missing from it simply cannot be bound
    for km in ["vim", "vscode"] {
        for (_, _, action) in keymap(km).unwrap().binds {
            assert!(
                ACTION_NAMES.iter().any(|(_, a)| *a == action),
                "{action:?} is bound in {km} but has no config name"
            );
        }
    }
    for (name, action) in ACTION_NAMES {
        assert_eq!(action_by_name(name), Some(*action));
    }
}

#[test]
fn parse_key_is_the_inverse_of_key_label() {
    for km in ["vim", "vscode"] {
        for (prefix, key, _) in keymap(km).unwrap().binds {
            for k in prefix.into_iter().chain([key]) {
                assert_eq!(parse_key(&key_label(k)), Some(k), "{}", key_label(k));
            }
        }
    }
}

#[test]
fn a_config_can_add_replace_and_remove_bindings() {
    let cfg = parse_key_config(
        "preset = \"vscode\"\n\n[binds]\n\"C-n\" = \"next\"\n\"g d\" = \"jump-to-edge\"\n\"x\" = \"none\"\n",
    );
    assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
    assert_eq!(cfg.preset.as_deref(), Some("vscode"));

    let km = apply_key_config(keymap("vim").unwrap(), &cfg);
    let has = |p: Option<Key>, k: Key, a: Action| km.binds.contains(&(p, k, a));
    assert!(has(None, ctrl('n'), Action::Next), "added binding");
    assert!(
        has(Some(ch('g')), ch('d'), Action::JumpToEdge),
        "chord binding"
    );
    assert!(
        !km.binds
            .iter()
            .any(|(p, k, _)| p.is_none() && *k == ch('x')),
        "`none` removes the binding"
    );
}

#[test]
fn a_config_binding_replaces_the_presets_own() {
    let cfg = parse_key_config("[binds]\n\"j\" = \"prev\"\n");
    let km = apply_key_config(keymap("vim").unwrap(), &cfg);
    let bound: Vec<Action> = km
        .binds
        .iter()
        .filter(|(p, k, _)| p.is_none() && *k == ch('j'))
        .map(|(_, _, a)| *a)
        .collect();
    assert_eq!(bound, vec![Action::Prev], "one binding, the config's");
}

#[test]
fn a_bad_config_line_is_reported_by_number_and_skipped() {
    let cfg = parse_key_config(
        "[binds]\n\"C-n\" = \"nonsense\"\n\"!!\" = \"next\"\nnot a pair\n\"C-y\" = \"help\"\n",
    );
    assert_eq!(cfg.problems.len(), 3, "{:?}", cfg.problems);
    assert!(cfg.problems[0].contains("line 2"), "{:?}", cfg.problems);
    assert!(cfg.problems[1].contains("line 3"), "{:?}", cfg.problems);
    // the good line still lands
    assert_eq!(cfg.binds.len(), 1);
    assert_eq!(cfg.binds[0].2, Some(Action::Help));
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let cfg = parse_key_config("# a comment\n\npreset = \"vim\"  # trailing\n");
    assert!(cfg.problems.is_empty(), "{:?}", cfg.problems);
    assert_eq!(cfg.preset.as_deref(), Some("vim"));
}

#[test]
fn a_folded_group_is_one_row_holding_its_hunks() {
    let (items, groups) = grouped_trio();
    let collapsed: HashSet<String> = ["g0".to_string()].into_iter().collect();
    let rows = display_rows(&[0, 1, 2], &items, &groups, true, &collapsed);
    assert_eq!(
        row_kinds(&rows),
        [
            "file a.rs 2",
            "folded [0, 1]",
            "file b.rs 1",
            "item 2 opens"
        ]
    );
}

#[test]
fn folding_keeps_the_selection_on_the_fold() {
    let mut app = test_app();
    app.items = vec![
        grouped_item("a.rs", "g0"),
        grouped_item("a.rs", "g0"),
        grouped_item("b.rs", "g1"),
    ];
    app.view = vec![0, 1, 2];
    app.reviewed = vec![false; 3];
    app.sel = 1;
    app.show_groups = true;

    fold(&mut app, Fold::Close);
    assert!(app.collapsed.contains("g0"));
    assert_eq!(app.sel, 1, "the fold's row carries the selection");
    // the fold is one stop for j/k, standing on the selection inside it
    assert_eq!(folded_view(&app), vec![1, 2]);
    app.sel = 2;
    assert_eq!(folded_view(&app), vec![0, 2]);

    // the same key, on the fold, opens it again
    app.sel = 0;
    fold(&mut app, Fold::Toggle);
    assert!(app.collapsed.is_empty());

    fold(&mut app, Fold::CloseAll);
    fold(&mut app, Fold::OpenAll);
    assert_eq!(folded_view(&app), vec![0, 1, 2]);
}

#[test]
fn folding_turns_group_headers_on_because_that_is_what_was_meant() {
    let mut app = test_app();
    app.items = vec![grouped_item("a.rs", "g0")];
    app.view = vec![0];
    app.reviewed = vec![false];
    app.show_groups = false;
    fold(&mut app, Fold::Toggle);
    assert!(app.show_groups);
}

#[test]
fn with_groups_off_every_row_names_its_own_symbol_and_nothing_folds() {
    let items = vec![grouped_item("a.rs", "g0"), grouped_item("a.rs", "g0")];
    let groups = HashMap::from([("g0".to_string(), "run".to_string())]);
    let collapsed: HashSet<String> = ["g0".to_string()].into_iter().collect();
    let rows = display_rows(&[0, 1], &items, &groups, false, &collapsed);
    assert_eq!(
        row_kinds(&rows),
        ["file a.rs 2", "item 0 opens", "item 1 opens"]
    );
}

#[test]
fn imports_fold_by_default_in_either_mode_and_groups_name_their_symbol() {
    let mut app = test_app();
    let mut import = grouped_item("a.rs", "gi");
    import.cat = ordo_diff::model::Category::Import;
    let mut defined = grouped_item("a.rs", "gd");
    defined.ledger = Some(0);
    defined.enclosing = Some("run".to_string());
    let mut gone = grouped_item("a.rs", "gr");
    gone.ledger = Some(1);
    app.items = vec![import, defined, gone, grouped_item("a.rs", "gt")];
    app.view = vec![0, 1, 2, 3];
    app.reviewed = vec![false; 4];
    let entry = |name: &str, change| ordo_diff::model::LedgerEntry {
        name: name.to_string(),
        kind: None,
        scope: None,
        path: "a.rs".to_string(),
        at: String::new(),
        change,
        from: None,
        used_by: vec![],
    };
    let ledger = vec![
        entry("run", ordo_diff::model::SymbolChange::Added),
        entry("old", ordo_diff::model::SymbolChange::Removed),
    ];

    set_mode(&mut app, ViewMode::Ledger, &ledger);
    assert_eq!(app.items[0].bucket, "gi", "imports keep the engine's group");
    assert!(app.collapsed.contains("gi"));
    assert_eq!(app.groups["gi"], "imports");
    assert_eq!(app.groups["L0"], "run");
    assert_eq!(
        app.groups["L1"], "",
        "a removed symbol has nothing left to read"
    );
    assert_eq!(app.groups["L-"], "");

    set_mode(&mut app, ViewMode::Hunks, &ledger);
    assert!(app.collapsed.contains("gi"));
    assert_eq!(app.groups["gd"], "run");
    assert_eq!(app.groups["gt"], "");
}

// ---- `<base>..zz` / `<base>...zz` parsing (relies on the test binary
// running inside the ordo git checkout, same assumption `resolve`'s own
// callers make; HEAD, not main, as the base: CI's shallow checkout of a PR
// has no local main) ----

#[test]
fn base_dotdot_zz_resolves_to_a_worktree_range_at_base() {
    let base = git(&["rev-parse", "--verify", "-q", "HEAD^{commit}"]);
    let base = base.trim().to_string();
    match resolve("HEAD..zz") {
        Some(Target::WorktreeRange(b)) => assert_eq!(b, base),
        _ => panic!("expected a WorktreeRange at HEAD"),
    }
}

#[test]
fn base_dotdotdot_zz_resolves_to_the_merge_base_with_head() {
    let merge_base = git(&["merge-base", "HEAD", "HEAD"]);
    let merge_base = merge_base.trim().to_string();
    match resolve("HEAD...zz") {
        Some(Target::WorktreeRange(b)) => assert_eq!(b, merge_base),
        _ => panic!("expected a WorktreeRange at the merge base"),
    }
}

#[test]
fn zz_on_the_left_is_rejected() {
    assert!(resolve_range("zz..main").is_none());
    assert!(resolve_range("zz...main").is_none());
    // resolve() as a whole also refuses it, same as any unresolvable rev
    assert!(resolve("zz..main").is_none());
}

#[test]
fn plain_zz_is_unchanged() {
    assert!(matches!(resolve("zz"), Some(Target::Uncommitted)));
}

#[test]
fn ordinary_commit_range_is_unchanged() {
    match resolve("HEAD..HEAD") {
        Some(Target::Range(_, tip)) => {
            let head = git(&["rev-parse", "--verify", "-q", "HEAD"]);
            assert_eq!(tip, head.trim());
        }
        _ => panic!("expected a plain commit Range"),
    }
}

/// A sibling repository whose pyproject points back at this one is found, and
/// only its files that import a changed module are handed over.
#[test]
fn a_sibling_that_imports_the_changed_module_becomes_a_consumer() {
    let base = std::env::temp_dir().join(format!("ordo-consumers-{}", std::process::id()));
    let (pump, pipeline, other) = (base.join("pump"), base.join("pipeline"), base.join("other"));
    for d in [&pump, &pipeline, &other] {
        std::fs::create_dir_all(d).unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(d)
            .status()
            .unwrap();
    }
    std::fs::write(
        pipeline.join("pyproject.toml"),
        "[tool.uv.sources]\npump = { path = \"../pump\", editable = true }\n",
    )
    .unwrap();
    std::fs::write(
        pipeline.join("driver.py"),
        "from script.run import main\nmain(1)\n",
    )
    .unwrap();
    std::fs::write(pipeline.join("unrelated.py"), "print(1)\n").unwrap();
    std::fs::write(other.join("uses.py"), "from script.run import main\n").unwrap();
    for d in [&pipeline, &other] {
        Command::new("git")
            .args(["add", "."])
            .current_dir(d)
            .status()
            .unwrap();
    }
    let changes = vec![ordo_diff::model::Change {
        path: "script/run.py".into(),
        old: Some("def main(a):\n    pass\n".into()),
        new: Some("def main(a, b):\n    pass\n".into()),
        diff: None,
    }];
    let found = crate::consumers::gather(&pump, &changes, &[]);
    let paths: Vec<&str> = found.iter().map(|c| c.path.as_str()).collect();
    // `other` imports it too, but nothing there says it depends on this repo
    assert_eq!(paths, vec!["../pipeline/driver.py"]);
    let declared = crate::consumers::gather(&pump, &changes, std::slice::from_ref(&other));
    assert_eq!(
        declared.len(),
        2,
        "{:?}",
        declared.iter().map(|c| &c.path).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// `consumers` in a rules file resolves beside that file, like an `include`.
#[test]
fn consumers_in_a_rules_file_resolve_beside_it() {
    let dir = std::env::temp_dir().join(format!("ordo-consumers-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("rules.toml");
    std::fs::write(&f, "consumers = [\"../pipeline\"]\n").unwrap();
    let report = report_from(vec![f], &[]);
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(
        report.rule_set(true).consumers,
        vec![dir.join("../pipeline")]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The handoff prompt carries the reviewer's notes anchored to file and line;
/// ordo's own notes and findings only when asked for with `all`.
#[test]
fn the_review_prompt_anchors_notes_and_adds_ordo_only_with_all() {
    let mut app = test_app();
    app.items[0].new_range = [12, 18];
    app.items[0].symbols = vec![sym("fetch", "function_definition", None)];
    app.items[0].notes = vec!["fetch became async; 1 call never awaits it".to_string()];
    assert_eq!(
        review_prompt(&app, false),
        None,
        "no note, nothing to hand back"
    );
    let k = note_key(&app.items[0]).unwrap();
    app.notes
        .insert(k, "retry forever is wrong here".to_string());
    let mine = review_prompt(&app, false).unwrap();
    assert!(
        mine.contains("## a.rs:12-18\nretry forever is wrong here"),
        "{mine}"
    );
    assert!(
        mine.contains("(1 point)") && !mine.contains("ordo:"),
        "{mine}"
    );
    let all = review_prompt(&app, true).unwrap();
    assert!(all.contains("- ordo: fetch became async"), "{all}");
    // in a review of waves, a point says which turn wrote the code
    app.items[0].wave = Some(2);
    app.waves
        .intents
        .insert(2, "asked: add retry to fetch\n\nagent: done".to_string());
    let waved = review_prompt(&app, false).unwrap();
    assert!(
        waved.contains(
            "## a.rs:12-18\n(wave 2, when you were asked: add retry to fetch)\nretry forever"
        ),
        "{waved}"
    );
}

/// `:send` pipes the prompt to `$ORDO_SEND`, a failing command says why, and
/// a hanging one is stopped. One test: they share the variable.
#[test]
fn send_pipes_to_the_configured_command() {
    let out = std::env::temp_dir().join(format!("ordo-send-{}", std::process::id()));
    // SAFETY: no other test reads or writes ORDO_SEND
    // quoted: an unquoted windows path loses its backslashes to the shell
    unsafe { std::env::set_var(crate::handoff::SEND_ENV, format!("cat > '{}'", out.display())) };
    assert!(send("the review").is_ok());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "the review");
    unsafe { std::env::set_var(crate::handoff::SEND_ENV, "echo nope >&2; exit 3") };
    let err = send("x").unwrap_err();
    assert!(err.contains("nope"), "{err}");
    // one that never returns is stopped rather than freezing the review
    unsafe { std::env::set_var(crate::handoff::SEND_ENV, "exec sleep 60") };
    let t = std::time::Instant::now();
    let err = send("x").unwrap_err();
    assert!(err.contains("no answer"), "{err}");
    assert!(t.elapsed() < std::time::Duration::from_secs(30));
    unsafe { std::env::remove_var(crate::handoff::SEND_ENV) };
    let _ = std::fs::remove_file(&out);
}

fn lines_of(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

/// A comment follows its lines when an edit above moves them, and stays put
/// but stale when the lines themselves change.
#[test]
fn a_line_comment_follows_its_lines_and_goes_stale_when_they_change() {
    let before = lines_of("a\nfetch()\nretry()\nz\n");
    let mut c = LineComment::new("f.py", 2, 3, "bound the retries", &before);
    c.relocate(&lines_of("new\nnew\na\nfetch()\nretry()\nz\n"));
    assert_eq!((c.start, c.end, c.stale), (4, 5, false));
    c.relocate(&lines_of("a\nfetch(timeout=1)\nretry()\nz\n"));
    assert_eq!((c.start, c.end, c.stale), (4, 5, true));
}

/// `:comment` comments on the `v` selection, the same range again replaces
/// it, and no text deletes it.
#[test]
fn comment_adds_replaces_and_deletes_on_the_selection() {
    let mut app = test_app();
    app.sources.insert(
        "a.rs".to_string(),
        (vec![], lines_of("fn a() {}\nfn b() {}\nfn c() {}\n")),
    );
    app.cursor = Cursor { line: 2, col: 0 };
    app.selection = Some(1);
    execute_command(&mut app, "comment these two should be one").unwrap();
    assert_eq!(app.comments.len(), 1);
    assert_eq!((app.comments[0].start, app.comments[0].end), (2, 3));
    assert_eq!(app.selection, None, "a comment ends the selection");
    app.selection = Some(1);
    execute_command(&mut app, "comment merge them").unwrap();
    assert_eq!(app.comments.len(), 1);
    assert_eq!(app.comments[0].text, "merge them");
    let prompt = review_prompt(&app, false).unwrap();
    assert!(prompt.contains("## a.rs:2-3\nmerge them"), "{prompt}");
    app.selection = Some(1);
    execute_command(&mut app, "comment").unwrap();
    assert!(app.comments.is_empty());
    assert!(
        execute_command(&mut app, "comment").is_err(),
        "nothing left to delete"
    );
}

/// Five hunks: warn, none, verdict, verdict, note — in that view order.
fn finding(level: ordo_diff::model::Level) -> ordo_diff::model::Finding {
    ordo_diff::model::Finding {
        source: ordo_diff::model::FindingSource::Rule,
        name: "r".to_string(),
        message: "m".to_string(),
        level,
        row: None,
        original_level: None,
    }
}

fn findings_app() -> App {
    use ordo_diff::model::Level;
    let mut app = test_app();
    let levels = [
        Some(Level::Warn),
        None,
        Some(Level::Verdict),
        Some(Level::Verdict),
        Some(Level::Note),
    ];
    app.items = levels
        .iter()
        .enumerate()
        .map(|(n, level)| {
            let mut it = test_item(&format!("{n}.rs"));
            it.findings.extend(level.map(finding));
            it
        })
        .collect();
    app.view = (0..5).collect();
    app.reviewed = vec![false; 5];
    app
}

/// `]f` cycles verdicts, then warns, then notes, each in view order, and
/// wraps; `[f` walks the same cycle backward.
#[test]
fn finding_next_visits_the_most_severe_first_and_wraps() {
    let mut app = findings_app();
    let mut seen = vec![];
    for _ in 0..5 {
        finding_move(&mut app, true);
        seen.push(app.sel);
    }
    assert_eq!(seen, [4, 2, 3, 0, 4]);
    finding_move(&mut app, false);
    assert_eq!(app.sel, 0);
    finding_move(&mut app, false);
    assert_eq!(app.sel, 3);
}

/// From a hunk with no finding, the jump lands on the nearest most-severe
/// hunk that way, wrapping when there is none.
#[test]
fn finding_jump_from_outside_the_cycle_starts_at_the_most_severe() {
    let mut app = findings_app();
    app.sel = 1;
    finding_move(&mut app, true);
    assert_eq!(app.sel, 2);
    app.sel = 1;
    finding_move(&mut app, false);
    assert_eq!(app.sel, 3, "no verdict before it, so the last one");
}

/// A hunk filtered out of the view is not jumped to.
#[test]
fn finding_jump_skips_hunks_outside_the_view() {
    let mut app = findings_app();
    app.view = vec![0, 1, 3, 4];
    finding_move(&mut app, true);
    assert_eq!(app.sel, 4);
    finding_move(&mut app, true);
    assert_eq!(app.sel, 3);
}

#[test]
fn finding_jump_with_no_findings_says_so_and_stays() {
    let mut app = findings_app();
    for it in &mut app.items {
        it.findings.clear();
    }
    app.sel = 2;
    finding_move(&mut app, true);
    assert_eq!(app.sel, 2);
    assert_eq!(app.notice.as_deref(), Some("no findings in view"));
}

/// A review of `names`, one hunk per symbol in `f.py`, ten lines apart.
fn review_of(names: &[&str]) -> App {
    let mut app = test_app();
    app.items = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut it = test_item("f.py");
            it.symbols = vec![sym(n, "function_definition", None)];
            it.new_range = [i * 10 + 1, i * 10 + 5];
            it
        })
        .collect();
    app.view = (0..names.len()).collect();
    app.reviewed = vec![false; names.len()];
    let lines: Vec<String> = (1..=80).map(|i| format!("line {i}")).collect();
    app.sources.insert("f.py".to_string(), (vec![], lines));
    app
}

/// Reloading keeps the reviewer on the same hunk when others arrive around it.
#[test]
fn a_reload_keeps_the_selected_hunk() {
    let mut old = review_of(&["a", "b", "c"]);
    select(&mut old, 1);
    let place = place_of(&old);
    let mut new = review_of(&["a", "new", "b", "c"]);
    restore(&mut new, place);
    assert_eq!(new.items[new.sel].symbols[0].name, "b");
}

/// A hunk that is gone leaves the reviewer on what now sits where it was.
#[test]
fn a_reload_whose_hunk_is_gone_lands_on_the_next_in_reading_order() {
    let mut old = review_of(&["a", "b", "c"]);
    select(&mut old, 1);
    let place = place_of(&old);
    let mut new = review_of(&["a", "c"]);
    restore(&mut new, place);
    assert_eq!(new.items[new.sel].symbols[0].name, "c");
}

/// A renamed symbol is followed through the engine's ledger.
#[test]
fn a_reload_follows_a_renamed_symbol() {
    let mut old = review_of(&["a", "parse", "c"]);
    select(&mut old, 1);
    let place = place_of(&old);
    let mut new = review_of(&["a", "c", "load"]);
    new.symbol_ledger = vec![ordo_diff::model::LedgerEntry {
        name: "load".to_string(),
        kind: None,
        scope: None,
        path: "f.py".to_string(),
        at: "h2".to_string(),
        change: ordo_diff::model::SymbolChange::Renamed,
        from: Some("parse".to_string()),
        used_by: vec![],
    }];
    restore(&mut new, place);
    assert_eq!(new.items[new.sel].symbols[0].name, "load");
}

/// Filters survive a reload.
#[test]
fn a_reload_keeps_filters() {
    let mut old = review_of(&["a", "b"]);
    old.show_all = false;
    let place = place_of(&old);
    let mut new = review_of(&["a", "b"]);
    restore(&mut new, place);
    assert!(!new.show_all);
}

/// `--watch=auto` reloads only when stale, idle, and nothing is open.
#[test]
fn auto_reload_waits_for_an_idle_reviewer_with_nothing_open() {
    let mut app = review_of(&["a"]);
    let idle = std::time::Duration::from_secs(10);
    app.stale = Some(Stale::Drift(2));
    app.watch = WatchMode::Hint;
    assert!(!auto_reload_now(&app, idle), "hint only marks");
    app.watch = WatchMode::Auto;
    assert!(auto_reload_now(&app, idle));
    assert!(
        !auto_reload_now(&app, std::time::Duration::from_millis(500)),
        "a key just now"
    );
    app.popup = Some(Popup::new("x", vec![]));
    assert!(!auto_reload_now(&app, idle), "a popup is open");
    app.popup = None;
    app.stale = Some(Stale::BaseMoved);
    assert!(
        !auto_reload_now(&app, idle),
        "a moved base is never followed"
    );
}

/// `:watch` on a committed revision says there is nothing to watch.
#[test]
fn watch_on_a_committed_revision_is_refused() {
    let mut app = review_of(&["a"]);
    app.uncommitted = false;
    assert!(execute_command(&mut app, "watch on").is_err());
    app.uncommitted = true;
    execute_command(&mut app, "watch auto").unwrap();
    assert_eq!(app.watch, WatchMode::Auto);
    execute_command(&mut app, "watch").unwrap();
    assert_eq!(app.watch, WatchMode::Off);
}

#[test]
fn watch_flags_parse_and_a_bad_debounce_is_refused() {
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let raw = flag_loop(argv(&["zz", "--watch"])).ok().unwrap();
    assert_eq!(raw.watch, Some(WatchMode::Hint));
    let raw = flag_loop(argv(&["zz", "--watch=auto", "--watch-debounce", "500"]))
        .ok()
        .unwrap();
    assert_eq!(raw.watch, Some(WatchMode::Auto));
    assert_eq!(raw.debounce_ms.as_deref(), Some("500"));
    assert!(flag_loop(argv(&["zz", "--watch=sometimes"])).is_err());
    assert!(flag_loop(argv(&["zz", "--watch-debounce=soon"])).is_err());
}

/// `ordo <rev>` serves the review, and `ordo web <rev>` is the same command:
/// the server's own flags read on either.
#[test]
fn web_is_an_alias_and_the_server_flags_need_no_alias() {
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    for args in [
        &["zz", "--port", "4780", "--no-open"][..],
        &["web", "zz", "--port", "4780", "--no-open"],
    ] {
        let raw = flag_loop(argv(args)).ok().unwrap();
        assert_eq!(raw.rev.as_deref(), Some("zz"), "{args:?}");
        assert_eq!(raw.port.as_deref(), Some("4780"), "{args:?}");
        assert!(raw.no_open, "{args:?}");
    }
    assert!(flag_loop(argv(&["zz", "--port", "many"])).is_err());
}

#[test]
fn no_revision_means_head_with_or_without_the_alias() {
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    for args in [&[][..], &["web"], &["--no-open"], &["web", "--no-open"]] {
        let parsed = parse_argv(argv(args)).ok().unwrap();
        assert_eq!(parsed.rev, "HEAD", "{args:?}");
    }
}

#[test]
fn an_unknown_flag_is_refused_with_or_without_the_alias() {
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(flag_loop(argv(&["--bogus"])).err(), Some(2));
    assert_eq!(flag_loop(argv(&["web", "--bogus"])).err(), Some(2));
    assert_eq!(flag_loop(argv(&["web", "zz", "--bogus"])).err(), Some(2));
}

#[test]
fn json_flag_keeps_the_revision_and_globs() {
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let raw = flag_loop(argv(&["main...zz", "src/*", "--json"]))
        .ok()
        .unwrap();
    assert!(raw.json);
    assert_eq!(raw.rev.as_deref(), Some("main...zz"));
    assert_eq!(raw.globs, vec!["src/*"]);
}

#[test]
fn json_refuses_flags_it_would_ignore() {
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(parse_argv(argv(&["zz", "--json", "--only-comments"])).is_err());
    assert!(parse_argv(argv(&["zz", "--json", "--sarif", "f.sarif"])).is_err());
    assert!(parse_argv(argv(&["zz", "--json", "--all", "src/*"])).is_ok());
}

#[test]
fn json_output_is_engine_input() {
    let input = Input {
        changes: vec![Change {
            path: "a.rs".into(),
            old: None,
            new: Some("fn a() {}\n".into()),
            diff: None,
        }],
        options: Default::default(),
        consumers: vec![],
    };
    let back: Input = serde_json::from_value(input_json(&input)).unwrap();
    assert_eq!(back.changes[0].path, "a.rs");
    assert_eq!(back.changes[0].old, None);
    assert_eq!(back.changes[0].new.as_deref(), Some("fn a() {}\n"));
}

/// `zz` in a repository GitButler does not manage: plain git's changed and
/// untracked files, ignored ones left out.
#[test]
fn the_uncommitted_area_of_a_plain_git_repository() {
    let dir = std::env::temp_dir().join(format!("ordo-zz-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sh = |cmd: &str| {
        Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&dir)
            .output()
            .unwrap();
    };
    sh(
        "git init -q && git config user.email t@t && git config user.name t \
        && printf a > a.py && printf b > b.py && printf 'out\\n' > .gitignore \
        && git add . && git commit -qm init \
        && printf A > a.py && printf n > new.py && mkdir out && printf x > out/x.py",
    );
    let mut got = uncommitted_paths(dir.to_str().unwrap());
    got.sort();
    assert_eq!(got, vec!["a.py", "new.py"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// One wave in view: the other waves' hunks are hidden, and accounted for.
#[test]
fn one_wave_in_view_accounts_for_the_others() {
    let mut app = test_app();
    app.items = vec![test_item("a.rs"), test_item("b.rs")];
    app.items[0].wave = Some(2);
    app.items[1].wave = Some(1);
    app.view = vec![0, 1];
    app.reviewed = vec![false, true];
    set_filters(&mut app, false, true, None, Some(2)).unwrap();
    assert_eq!(app.view, vec![0]);
    let hidden = hidden_breakdown(&app.items, false, true, None, Some(2));
    assert_eq!((hidden.wave, hidden.unaccounted), (1, 0));
}

/// `gw` walks all → each wave → all again, `gW` the other way, and the status
/// line names the wave with what it was asked.
#[test]
fn stepping_through_waves_cycles_back_to_all_of_them() {
    let mut app = test_app();
    app.items = vec![test_item("a.rs"), test_item("b.rs"), test_item("c.rs")];
    app.items[0].wave = Some(1);
    app.items[1].wave = Some(3);
    app.view = vec![0, 1, 2];
    app.reviewed = vec![false; 3];
    app.waves
        .intents
        .insert(3, "asked: add retry\n\nagent: done".to_string());
    let mut seen = vec![];
    for _ in 0..3 {
        step_wave(&mut app, true);
        seen.push((app.only_wave, app.view.clone()));
    }
    assert_eq!(
        seen,
        vec![
            (Some(1), vec![0]),
            (Some(3), vec![1]),
            (None, vec![0, 1, 2])
        ]
    );
    step_wave(&mut app, false);
    assert_eq!(app.only_wave, Some(3));
    assert_eq!(app.notice.as_deref(), Some("wave 3 · asked: add retry"));
}

/// GitButler 0.23 lists linked worktrees beside the stacks; their branch and
/// commit IDs resolve the same way a stack's do.
#[test]
fn a_linked_worktrees_branch_and_commit_ids_resolve() {
    let ws = serde_json::json!({
        "stacks": [{"branches": [{"cliId": "at", "name": "feat", "commits": [
            {"cliId": "twxq", "changeId": "twxqvl", "commitId": "4c35259a"}]}]}],
        "worktrees": [{"cliId": "wt", "reference": "refs/heads/wtb", "branches": [
            {"cliId": "wt", "name": "wtb", "commits": [
                {"cliId": "torx", "changeId": "torxqy", "commitId": "2d221776"}]}]}],
    });
    let names: Vec<&str> = branches(&ws).map(|b| field(b, "name")).collect();
    assert_eq!(names, vec!["feat", "wtb"]);
    assert!(matches!(commit_target(&ws, "torx"), Some(Target::Commit(c)) if c == "2d221776"));
    assert!(matches!(commit_target(&ws, "twxq"), Some(Target::Commit(c)) if c == "4c35259a"));
}

/// A range end by GitButler ID: a branch is its newest commit, a commit itself.
#[test]
fn a_range_end_takes_a_gitbutler_branch_or_commit_id() {
    let ws = serde_json::json!({
        "stacks": [{"branches": [
            {"cliId": "pp", "name": "feat", "commits": [
                {"cliId": "twxq", "changeId": "twxqvl", "commitId": "4c35259a"},
                {"cliId": "lzmo", "changeId": "lzmoab", "commitId": "9e01aa77"}]},
            {"cliId": "em", "name": "empty", "commits": []}]}],
    });
    assert_eq!(workspace_rev(&ws, "pp").as_deref(), Some("4c35259a"));
    assert_eq!(workspace_rev(&ws, "feat").as_deref(), Some("4c35259a"));
    assert_eq!(workspace_rev(&ws, "lzmo").as_deref(), Some("9e01aa77"));
    assert_eq!(workspace_rev(&ws, "em"), None);
    assert_eq!(workspace_rev(&ws, "nope"), None);
}

#[test]
fn the_rules_report_counts_the_built_in_catalog_it_runs() {
    let r = report_from(vec![], &[]);
    let with = r.lines(true);
    let catalog: usize = ordo_diff::catalog::rules().len();
    assert_eq!(with[0], format!("{catalog} rules active"));
    assert!(with.iter().any(|l| l.ends_with("built-in catalog: python")));
    assert_eq!(r.lines(false)[0], "0 rules active");
}
