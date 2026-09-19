//! A large pure insertion — an added file above all — is one diff hunk however
//! much it holds. These pin that it is cut at the constructs inside it.
mod fixture;
use fixture::{hunks as one, run_file as run};

/// `n` functions, each `Nth` in the source, as one added file.
fn generated(n: usize) -> String {
    (0..n)
        .map(|i| format!("def f{i}(x):\n    y = x + {i}\n    return y\n\n\n"))
        .collect()
}

#[test]
fn a_large_added_file_is_split_per_definition() {
    let new = generated(20); // 100 lines, well past the one-card threshold
    let hs = one("a.py", "", &new);
    assert_eq!(hs.len(), 20, "one hunk per added function");
    assert_eq!(hs[0].enclosing.as_deref(), Some("f0"));
    assert_eq!(hs[19].enclosing.as_deref(), Some("f19"));
    assert_eq!(hs[7].defines, vec!["f7".to_string()]);
}

#[test]
fn a_short_added_file_stays_one_hunk() {
    // splitting is for the case a reviewer cannot take in at once; a file that
    // fits on a screen reads better whole
    let hs = one("a.py", "", &generated(3));
    assert_eq!(hs.len(), 1);
}

#[test]
fn a_namespace_wrapped_header_is_split_inside_the_namespace() {
    // c++ puts everything in one namespace inside one include guard: cutting
    // only at the outermost construct would put the whole file back in one card
    let body: String = (0..12)
        .map(|i| format!("inline int f{i}(int x)\n{{\n    return x + {i};\n}}\n\n"))
        .collect();
    let new = format!("#ifndef a_H\n#define a_H\n\nnamespace a\n{{\n\n{body}}}\n\n#endif\n");
    let hs = one("a.H", "", &new);
    for i in 0..12 {
        let name = format!("f{i}");
        assert_eq!(
            hs.iter().filter(|h| h.defines.contains(&name)).count(),
            1,
            "{name} has a hunk of its own, got {:?}",
            hs.iter().map(|h| &h.defines).collect::<Vec<_>>()
        );
    }
    assert!(
        hs.len() <= 15,
        "12 functions plus the guard and namespace lines, not a card per line: {}",
        hs.len()
    );
}

#[test]
fn splitting_an_added_file_keeps_its_def_use_edges() {
    // the point of cutting: a helper and its caller land in different hunks, so
    // the definition can be ordered before the use
    let filler: String = (0..20)
        .map(|i| format!("def pad{i}():\n    return {i}\n\n\n"))
        .collect();
    let new = format!("{filler}def helper(x):\n    return x\n\n\ndef caller():\n    return helper(1)\n");
    let out = run("a.py", "", &new);
    let hunk_of = |name: &str| {
        out.files[0]
            .hunks
            .iter()
            .find(|h| h.defines.iter().any(|d| d == name))
            .unwrap_or_else(|| panic!("hunk defining {name}"))
    };
    let (def, usage) = (hunk_of("helper"), hunk_of("caller"));
    assert_ne!(def.id, usage.id, "definition and use are separate hunks");
    assert!(
        out.edges
            .iter()
            .any(|e| e.from == def.id && e.to == usage.id),
        "def→use edge from {} to {}, got {:?}",
        def.id,
        usage.id,
        out.edges
    );
}
