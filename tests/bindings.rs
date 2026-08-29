//! P17 "where is this binding used?" provenance tests.
use ordo::model::Input;

fn rationales(v: serde_json::Value) -> Vec<String> {
    let inp: Input = serde_json::from_value(v).unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks.into_iter().map(|h| h.rationale))
        .collect()
}

fn one(path: &str, old: &str, new: &str) -> Vec<String> {
    rationales(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
}

#[test]
fn discriminates_binding_from_a_name_that_merely_contains_it() {
    // `may_refine_camber_span` shares a prefix with `may_refine` and appears
    // as both a keyword-argument name and its own local binding — a
    // substring/regex match would over-count; exact identifier-node
    // comparison must not.
    let old = "def build_impeller_from_spec(spec):\n\
               \x20   x = foo(may_refine_camber_span=False)\n\
               \x20   may_refine_camber_span = True\n\
               \x20   z = baz(may_refine_camber_span=False)\n\
               \x20   return x, z\n";
    let new = "def build_impeller_from_spec(spec):\n\
               \x20   may_refine = not spec.shrouded\n\
               \x20   x = foo(may_refine_camber_span=False)\n\
               \x20   may_refine_camber_span = True\n\
               \x20   z = baz(may_refine_camber_span=False)\n\
               \x20   return x, z\n";
    // now wire the two call sites to actually use `may_refine`
    let new = new.replacen(
        "x = foo(may_refine_camber_span=False)",
        "x = foo(may_refine_camber_span=may_refine)",
        1,
    );
    let new = new.replacen(
        "z = baz(may_refine_camber_span=False)",
        "z = baz(may_refine_camber_span=may_refine)",
        1,
    );
    let rats = one("impeller.py", old, &new);
    let binding = rats
        .iter()
        .find(|r| r.contains("adds local may_refine,"))
        .unwrap_or_else(|| panic!("no binding rationale found: {rats:?}"));
    assert!(
        binding.starts_with("adds local may_refine, used at L"),
        "{binding}"
    );
    // exactly 2 uses — not the 4 occurrences a substring search would find
    let l_count = binding.matches('L').count();
    assert_eq!(l_count, 2, "{binding}");
    assert!(
        !binding.contains("may_refine_camber_span"),
        "must not count may_refine_camber_span as a use of may_refine: {binding}"
    );
}

#[test]
fn a_def_plus_module_constants_in_one_hunk_names_both() {
    // regression: a hunk defining a real function AND introducing
    // module-level constants beside it used to report only the function —
    // the constants (assignment nodes, deliberately excluded from `defines`)
    // vanished from the rationale entirely. Binding wording must compose
    // with def-side wording rather than being suppressed by it.
    let new = "N_SECTIONS_DEFAULT = 3\nN_SECTIONS_REFINED = 5\n\n\ndef wrap_deg(x):\n    return x % 360\n";
    let rats = one("a.py", "", new);
    let r = rats
        .iter()
        .find(|r| r.starts_with("adds wrap_deg"))
        .unwrap_or_else(|| panic!("no def rationale found: {rats:?}"));
    assert!(r.contains("N_SECTIONS_DEFAULT"), "{r}");
    assert!(r.contains("N_SECTIONS_REFINED"), "{r}");
}

#[test]
fn no_uses_inside_a_function_names_the_enclosing_scope() {
    let old = "def build(spec):\n    return spec\n";
    let new = "def build(spec):\n    may_refine = not spec.shrouded\n    return spec\n";
    let rats = one("a.py", old, new);
    assert!(
        rats.iter()
            .any(|r| r == "adds local may_refine, no uses in build — check nested scopes"),
        "{rats:?}"
    );
}

#[test]
fn no_uses_at_module_level_says_check_other_files() {
    let old = "import os\n";
    let new = "import os\nmay_refine = True\n";
    let rats = one("a.py", old, new);
    assert!(
        rats.iter()
            .any(|r| r == "adds may_refine, no uses in this file — check other files"),
        "{rats:?}"
    );
}

#[test]
fn many_bindings_sharing_an_outcome_collapse_into_one_fragment() {
    // 14 module-level constants added in one hunk, none used anywhere else —
    // must read as ONE "no uses" fragment naming a few, then "and N more",
    // not one repeated "no uses in this file — check other files" per name.
    let names: Vec<String> = (0..14).map(|i| format!("CONST_{i:02}")).collect();
    let old = "";
    let new: String = names.iter().map(|n| format!("{n} = 1\n")).collect();
    let rats = one("a.py", old, &new);
    let binding = rats
        .iter()
        .find(|r| r.contains("no uses in this file"))
        .unwrap_or_else(|| panic!("no binding rationale found: {rats:?}"));
    // one outcome phrase, not fourteen
    assert_eq!(
        binding
            .matches("no uses in this file — check other files")
            .count(),
        1,
        "{binding}"
    );
    assert!(binding.contains("and 11 more"), "{binding}");
    assert!(
        binding.len() < 150,
        "rationale must stay a one-line glance, not a per-symbol listing: {} chars: {binding}",
        binding.len()
    );
}

#[test]
fn many_used_bindings_collapse_with_line_numbers_capped_too() {
    // mirrors ppump/constants.py:L41 in the real corpus: several module-level
    // constants each used once elsewhere, all introduced in one hunk.
    let names: Vec<String> = (0..6).map(|i| format!("FACE_{i}")).collect();
    let old = String::new();
    let mut new = String::new();
    for n in &names {
        new.push_str(&format!("{n} = 1\n"));
    }
    new.push_str("uses = [");
    new.push_str(&names.join(", "));
    new.push_str("]\n");
    let rats = one("a.py", &old, &new);
    let binding = rats
        .iter()
        .find(|r| r.contains("FACE_0 (L"))
        .unwrap_or_else(|| panic!("no binding rationale found: {rats:?}"));
    assert!(binding.contains("more"), "{binding}");
    assert!(binding.len() < 150, "{} chars: {binding}", binding.len());
}

#[test]
fn a_binding_that_is_a_definition_is_not_double_reported() {
    // lua: `local f = function() end` is a `function_definition` value bound
    // to `f` — already named as a def via `bound_name`; the locals table
    // (`variable_declaration`) must not report it a second time as "local f".
    let old = "";
    let new = "local f = function()\n  return 1\nend\n";
    let rats = one("a.lua", old, new);
    assert!(rats.iter().any(|r| r.contains("adds f")), "{rats:?}");
    assert!(
        !rats.iter().any(|r| r.contains("local f")),
        "def-bound name reported twice: {rats:?}"
    );
}

#[test]
fn locals_inside_a_newly_added_def_are_not_relisted() {
    // The rationale already says "adds compute"; its own locals are that
    // function's implementation detail, not independent facts. A binding at
    // module level in the same hunk IS independent and must still be named.
    let d = one(
        "a.py",
        "X = 1\n",
        "X = 1\nLIMIT = 5\n\n\ndef compute(n):\n    scratch = n * 2\n    total = scratch + LIMIT\n    return total\n",
    );
    let joined = d.join(" | ");
    assert!(
        !joined.contains("scratch") && !joined.contains("total"),
        "locals of a new def must not be relisted: {joined}"
    );
    assert!(
        joined.contains("LIMIT"),
        "a module-level binding in the same hunk is still named: {joined}"
    );
}

#[test]
fn bindings_in_several_scopes_say_the_phrase_once() {
    // Found by raising curl's corpus cap: no-uses bindings were grouped per
    // scope, but two scopes each emitted ", no uses in ..." — grouped within,
    // not across. Repeating a provenance phrase is the shape that has produced
    // three separate over-length regressions.
    let d = one(
        "a.c",
        "int go(void) { return 0; }\n",
        "struct A { int x; };\nstruct B { int y; };\n\
         int go(void) { struct A guard; struct B hd; return 0; }\n",
    );
    for r in &d {
        assert!(
            r.matches(", no uses in ").count() <= 1,
            "the phrase must appear once however many scopes are involved: {r}"
        );
    }
}

#[test]
fn a_declaration_the_parser_choked_on_yields_no_bindings() {
    // A macro-heavy c++ header parses with ERROR nodes, and harvesting names
    // from a failed parse produced `adds local virtual` and `adds local
    // override` — keywords, not bindings. Found by the OpenFOAM corpus.
    let d = one(
        "a.H",
        "class A {\n};\n",
        "class A {\n  vtkTypeMacro(A, Base;\n  virtual int Run(int) override;\n};\n",
    );
    let joined = d.join(" | ");
    assert!(
        !joined.contains("virtual") && !joined.contains("override"),
        "a c++ keyword must never be reported as a binding: {joined}"
    );
}
