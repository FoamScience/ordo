//! Advanced-construct advisories (P14): detect powerful/overusable language
//! constructs and attach escalation-ladder guidance, with a downgrade *verdict*
//! only where the pattern is deterministically wrong. A curated catalog of
//! senior review knowledge — deliberately NOT a style linter.
use crate::lang::LangSpec;
use crate::model::Advisory;
use tree_sitter::Node;

/// Detect constructs in the parsed *new* tree → (0-based start row, advisory).
pub fn advise(spec: &LangSpec, root: Node, src: &[u8]) -> Vec<(usize, Advisory)> {
    let mut out = vec![];
    let walker = match spec.name {
        "python" => walk_python,
        "rust" => walk_rust,
        "javascript" | "typescript" | "tsx" => walk_js,
        "go" => walk_go,
        _ => return out,
    };
    walk(root, src, walker, &mut out);
    out
}

type Out = Vec<(usize, Advisory)>;
type Rule = fn(Node, &[u8], &mut Out);

// generic post-order-ish walk applying a per-language rule to every named node
fn walk(node: Node, src: &[u8], rule: Rule, out: &mut Out) {
    rule(node, src, out);
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        walk(ch, src, rule, out);
    }
}

fn push(out: &mut Out, node: Node, construct: &str, message: &str, verdict: bool) {
    out.push((
        node.start_position().row,
        Advisory {
            construct: construct.into(),
            message: message.into(),
            verdict,
        },
    ));
}

fn callee_text<'a>(call: Node, field: &str, src: &'a [u8]) -> Option<&'a str> {
    call.child_by_field_name(field)
        .and_then(|f| f.utf8_text(src).ok())
}

// ------------------------------------------------------------------- python

const METACLASS_LADDER: &str = "\
metaclass — 90% of the time the wrong tool. Lightest sufficient step:
1. configure one attribute → __set_name__ (descriptor)
2. react to subclassing (register/validate/defaults) → __init_subclass__
3. replace the class after it's built → class decorator
4. rewrite the class as it's built, or control instance creation → metaclass";

const MUTABLE_DEFAULT: &str = "\
mutable default argument — the same object is shared across all calls (classic bug).
Use None as the sentinel and create the value inside the function.";

const BARE_EXCEPT: &str = "\
bare `except:` — also swallows SystemExit / KeyboardInterrupt and hides bugs.
Catch the narrowest exception; use `except Exception:` for a deliberate catch-all.";

const EVAL_PY: &str = "\
eval/exec — arbitrary code execution. Lightest sufficient step:
1. parse data → ast.literal_eval / json
2. dispatch by name → a dict or getattr
3. import by path → importlib
4. run arbitrary code → eval/exec (never on untrusted input)";

fn walk_python(node: Node, src: &[u8], out: &mut Out) {
    match node.kind() {
        "class_definition" => {
            if let Some(a) = python_metaclass(node, src) {
                out.push((node.start_position().row, a));
            }
        }
        "default_parameter" | "typed_default_parameter" => {
            if let Some(v) = node.child_by_field_name("value") {
                if matches!(v.kind(), "list" | "dictionary" | "set") {
                    push(out, node, "mutable-default-arg", MUTABLE_DEFAULT, true);
                }
            }
        }
        "except_clause" => {
            // bare `except:` — first named child is the body block, no exception type
            let mut cur = node.walk();
            if node.named_children(&mut cur).next().map(|c| c.kind()) == Some("block") {
                push(out, node, "bare-except", BARE_EXCEPT, true);
            }
        }
        "call" => {
            if matches!(
                callee_text(node, "function", src),
                Some("eval") | Some("exec")
            ) {
                push(out, node, "eval/exec", EVAL_PY, false);
            }
        }
        _ => {}
    }
}

fn python_metaclass(class: Node, src: &[u8]) -> Option<Advisory> {
    let supers = class.child_by_field_name("superclasses")?;
    let (mut uses_meta, mut defines_meta) = (false, false);
    let mut cur = supers.walk();
    for arg in supers.named_children(&mut cur) {
        if arg.kind() == "keyword_argument" {
            if arg
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(src).ok())
                == Some("metaclass")
            {
                uses_meta = true;
            }
        } else if arg.utf8_text(src) == Ok("type") {
            defines_meta = true;
        }
    }
    if !uses_meta && !defines_meta {
        return None;
    }
    let mk = |message: String, verdict: bool| {
        Some(Advisory {
            construct: "metaclass".into(),
            message,
            verdict,
        })
    };
    if defines_meta {
        let methods = class_methods(class, src);
        let warranted = methods
            .iter()
            .any(|m| matches!(m.as_str(), "__new__" | "__prepare__" | "__call__"));
        if warranted {
            return mk(METACLASS_LADDER.to_string(), false);
        }
        let dunders: Vec<&str> = methods
            .iter()
            .filter(|m| m.starts_with("__"))
            .map(|s| s.as_str())
            .collect();
        let over = if dunders.is_empty() {
            "state".to_string()
        } else {
            dunders.join(", ")
        };
        return mk(
            format!("{METACLASS_LADDER}\n\n⚠ this metaclass overrides only {over} — __init_subclass__ (step 2) likely suffices."),
            true,
        );
    }
    mk(METACLASS_LADDER.to_string(), false)
}

fn class_methods(class: Node, src: &[u8]) -> Vec<String> {
    let mut out = vec![];
    if let Some(body) = class.child_by_field_name("body") {
        let mut cur = body.walk();
        for stmt in body.named_children(&mut cur) {
            if stmt.kind() == "function_definition" {
                if let Some(n) = stmt
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                {
                    out.push(n.to_string());
                }
            }
        }
    }
    out
}

// --------------------------------------------------------------------- rust

const UNSAFE_RS: &str = "\
unsafe — you now uphold the invariants the compiler cannot check.
1. can a safe std API / abstraction do it?
2. if not, keep the block minimal and document the invariant it upholds.";

const TRANSMUTE_RS: &str = "\
mem::transmute — the biggest hammer, rarely justified.
1. numeric cast → `as`
2. float/int bits → f32::to_bits / from_bits
3. byte reinterpret → bytemuck / a pointer cast
4. transmute (last resort; same size, well-understood layout)";

fn walk_rust(node: Node, src: &[u8], out: &mut Out) {
    match node.kind() {
        "unsafe_block" => push(out, node, "unsafe", UNSAFE_RS, false),
        "call_expression" => {
            if callee_text(node, "function", src).is_some_and(|t| t.ends_with("transmute")) {
                push(out, node, "transmute", TRANSMUTE_RS, false);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- javascript

const EVAL_JS: &str = "\
eval — arbitrary code execution and a performance deopt.
1. parse data → JSON.parse
2. dynamic property → obj[key]
3. dynamic import → import()
4. eval (avoid; never on untrusted input)";

const WITH_JS: &str = "\
`with` — ambiguous scope resolution; illegal in strict mode / modules.
Destructure or alias the object explicitly instead.";

fn walk_js(node: Node, src: &[u8], out: &mut Out) {
    match node.kind() {
        "with_statement" => push(out, node, "with", WITH_JS, true),
        "call_expression" => {
            if callee_text(node, "function", src) == Some("eval") {
                push(out, node, "eval", EVAL_JS, false);
            }
        }
        _ => {}
    }
}

// ----------------------------------------------------------------------- go

const UNSAFE_GO: &str = "\
unsafe — bypasses Go's type and memory guarantees, breaks across releases.
1. can encoding/binary, generics, or an interface do it?
2. if not, isolate it and document why.";

const REFLECT_GO: &str = "\
reflect — slow, unchecked at compile time, hard to read.
1. interface + type switch
2. generics (Go 1.18+)
3. code generation
4. reflect (last resort)";

fn walk_go(node: Node, src: &[u8], out: &mut Out) {
    if node.kind() == "selector_expression" {
        match node
            .child_by_field_name("operand")
            .and_then(|o| o.utf8_text(src).ok())
        {
            Some("unsafe") => push(out, node, "unsafe", UNSAFE_GO, false),
            Some("reflect") => push(out, node, "reflect", REFLECT_GO, false),
            _ => {}
        }
    }
}
