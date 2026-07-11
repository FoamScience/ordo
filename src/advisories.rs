//! Advanced-construct advisories (P14): detect powerful/overusable language
//! constructs and attach escalation-ladder guidance, with a downgrade *verdict*
//! only where the pattern is deterministically wrong. A curated catalog of
//! senior review knowledge — deliberately NOT a style linter.
use crate::lang::{is_test_path, LangSpec};
use crate::model::Advisory;
use tree_sitter::Node;

/// Detect constructs in the parsed *new* tree → (0-based start row, advisory).
pub fn advise(spec: &LangSpec, root: Node, src: &[u8], path: &str) -> Vec<(usize, Advisory)> {
    let mut out = vec![];
    let walker: Rule = match spec.name {
        "python" => walk_python,
        "rust" => walk_rust,
        "javascript" | "typescript" | "tsx" => walk_js,
        "go" => walk_go,
        "c" => walk_c,
        "cpp" => walk_cpp,
        "java" => walk_java,
        _ => return out,
    };
    walk(root, src, path, walker, &mut out);
    out
}

type Out = Vec<(usize, Advisory)>;
type Rule = fn(Node, &[u8], &str, &mut Out);

fn walk(node: Node, src: &[u8], path: &str, rule: Rule, out: &mut Out) {
    rule(node, src, path, out);
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        walk(ch, src, path, rule, out);
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

fn named<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut cur = node.walk();
    node.named_children(&mut cur).collect()
}

const EMPTY_CATCH: &str = "\
empty catch — the error is silently swallowed and failures vanish.
Handle it, re-raise, or at minimum log; never an empty handler.";

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

const ASSERT_PY: &str = "\
assert for validation — assertions are stripped under `python -O`.
Raise a real exception (ValueError/TypeError) for runtime checks; keep assert for invariants.";

const DYNAMIC_TYPE: &str = "\
dynamic type() class creation — opaque to readers and tools.
1. a normal class / @dataclass
2. namedtuple / Enum for simple shapes
3. type() only for genuinely runtime-computed classes";

const DEL_PY: &str = "\
__del__ finalizer — unpredictable timing, skipped at interpreter exit, keeps reference cycles alive.
1. deterministic cleanup → a context manager (__enter__/__exit__ or contextlib)
2. non-deterministic cleanup → weakref.finalize
3. __del__ (last resort; must be exception-safe and side-effect-light)";

const HASH_PY: &str = "\
__eq__ without __hash__ — defining __eq__ makes instances unhashable (can't be a set member or dict key).
1. immutable value → @dataclass(frozen=True) generates both
2. by hand → define __hash__ over the same fields as __eq__
3. intentionally unhashable → set `__hash__ = None` explicitly";

const SYSTEM_PY: &str = "\
os.system — runs a string through the shell (injection), no output capture, no error object.
1. run a program → subprocess.run([...]) with an argument list (no shell)
2. capture output → subprocess.run(..., capture_output=True)
3. os.system (avoid; never with interpolated input)";

const SHELL_PY: &str = "\
subprocess(..., shell=True) — the command string is parsed by the shell (injection).
Pass an argument list and drop shell=True; if a shell is truly required, shlex.quote every interpolated value.";

const PICKLE_PY: &str = "\
pickle load — unpickling untrusted data executes arbitrary code (__reduce__).
1. structured data → json
2. with a schema → pydantic / dataclasses over json
3. cross-language / binary → protobuf / msgpack
4. pickle (only for data you produced and fully trust)";

fn walk_python(node: Node, src: &[u8], path: &str, out: &mut Out) {
    match node.kind() {
        "class_definition" => {
            if let Some(a) = python_metaclass(node, src) {
                out.push((node.start_position().row, a));
            }
            // __eq__ with no __hash__ anywhere in the body → unhashable instances
            let methods = class_methods(node, src);
            let has_hash = node
                .child_by_field_name("body")
                .and_then(|b| b.utf8_text(src).ok())
                .is_some_and(|t| t.contains("__hash__"));
            if methods.iter().any(|m| m == "__eq__") && !has_hash {
                push(out, node, "eq-without-hash", HASH_PY, true);
            }
        }
        "function_definition" if callee_text(node, "name", src) == Some("__del__") => {
            push(out, node, "del-finalizer", DEL_PY, true);
        }
        "default_parameter" | "typed_default_parameter" => {
            if node
                .child_by_field_name("value")
                .is_some_and(|v| matches!(v.kind(), "list" | "dictionary" | "set"))
            {
                push(out, node, "mutable-default-arg", MUTABLE_DEFAULT, true);
            }
        }
        "except_clause" => {
            let kids = named(node);
            if kids.first().map(|c| c.kind()) == Some("block") {
                push(out, node, "bare-except", BARE_EXCEPT, true);
            } else if kids
                .last()
                .is_some_and(|b| b.kind() == "block" && only_pass(*b))
            {
                push(out, node, "empty-catch", EMPTY_CATCH, true);
            }
        }
        "assert_statement" if !is_test_path(path) => {
            push(out, node, "assert-validation", ASSERT_PY, true);
        }
        "call" => {
            let fname = callee_text(node, "function", src);
            match fname {
                Some("eval") | Some("exec") => push(out, node, "eval/exec", EVAL_PY, false),
                Some("type")
                    if node
                        .child_by_field_name("arguments")
                        .map(|a| named(a).len())
                        == Some(3) =>
                {
                    push(out, node, "dynamic-type", DYNAMIC_TYPE, false);
                }
                Some("os.system") => push(out, node, "os-system", SYSTEM_PY, false),
                Some("pickle.load")
                | Some("pickle.loads")
                | Some("cPickle.load")
                | Some("cPickle.loads") => push(out, node, "pickle", PICKLE_PY, false),
                _ => {}
            }
            if fname.is_some_and(|f| f.starts_with("subprocess.")) && py_shell_true(node, src) {
                push(out, node, "shell-injection", SHELL_PY, true);
            }
        }
        _ => {}
    }
}

// a subprocess call carrying `shell=True`
fn py_shell_true(call: Node, src: &[u8]) -> bool {
    call.child_by_field_name("arguments").is_some_and(|args| {
        named(args).iter().any(|a| {
            a.kind() == "keyword_argument"
                && a.child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                    == Some("shell")
                && a.child_by_field_name("value")
                    .and_then(|v| v.utf8_text(src).ok())
                    == Some("True")
        })
    })
}

fn only_pass(block: Node) -> bool {
    let kids = named(block);
    kids.len() == 1 && kids[0].kind() == "pass_statement"
}

fn python_metaclass(class: Node, src: &[u8]) -> Option<Advisory> {
    let supers = class.child_by_field_name("superclasses")?;
    let (mut uses_meta, mut defines_meta) = (false, false);
    for arg in named(supers) {
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
        for stmt in named(body) {
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

const STATIC_MUT_RS: &str = "\
static mut — data races and UB the moment it's touched from >1 place.
1. immutable static / const
2. OnceLock / LazyLock for init-once
3. atomics for counters/flags
4. Mutex/RwLock for shared mutable state";

fn walk_rust(node: Node, src: &[u8], _path: &str, out: &mut Out) {
    match node.kind() {
        "unsafe_block" => push(out, node, "unsafe", UNSAFE_RS, false),
        "static_item" if named(node).iter().any(|c| c.kind() == "mutable_specifier") => {
            push(out, node, "static-mut", STATIC_MUT_RS, true);
        }
        "call_expression"
            if callee_text(node, "function", src).is_some_and(|t| t.ends_with("transmute")) =>
        {
            push(out, node, "transmute", TRANSMUTE_RS, false);
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

const ANY_TS: &str = "\
`any` — opts out of type checking and infects everything it touches.
1. write the real type
2. `unknown` + narrow at the boundary
3. a generic parameter
4. any (last resort, isolate it)";

fn walk_js(node: Node, src: &[u8], _path: &str, out: &mut Out) {
    match node.kind() {
        "with_statement" => push(out, node, "with", WITH_JS, true),
        "predefined_type" if node.utf8_text(src) == Ok("any") => {
            push(out, node, "any", ANY_TS, false)
        }
        "call_expression" if callee_text(node, "function", src) == Some("eval") => {
            push(out, node, "eval", EVAL_JS, false);
        }
        "catch_clause"
            if node
                .child_by_field_name("body")
                .is_some_and(|b| named(b).is_empty()) =>
        {
            push(out, node, "empty-catch", EMPTY_CATCH, true);
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

const PANIC_GO: &str = "\
panic in library code — crashes the caller's whole process.
Return an error and let the caller decide; reserve panic for truly unrecoverable state.";

fn walk_go(node: Node, src: &[u8], path: &str, out: &mut Out) {
    match node.kind() {
        "selector_expression" => match node
            .child_by_field_name("operand")
            .and_then(|o| o.utf8_text(src).ok())
        {
            Some("unsafe") => push(out, node, "unsafe", UNSAFE_GO, false),
            Some("reflect") => push(out, node, "reflect", REFLECT_GO, false),
            _ => {}
        },
        "call_expression"
            if !is_test_path(path) && callee_text(node, "function", src) == Some("panic") =>
        {
            push(out, node, "panic", PANIC_GO, false);
        }
        _ => {}
    }
}

// -------------------------------------------------------------------- c / c++

const GOTO: &str = "\
goto — tangles control flow.
1. structured loops / early return
2. RAII / scope guards for cleanup (C++)
3. a single cleanup label is the rare defensible case (C error paths)";

const REINTERPRET_CAST: &str = "\
reinterpret_cast — reinterprets bits with no checks (often UB).
1. value convert → static_cast
2. type-pun → std::bit_cast (C++20) / memcpy
3. reinterpret_cast (last resort; you own the aliasing/lifetime rules)";

const CONST_CAST: &str = "\
const_cast — casting away const is UB if the underlying object is really const.
1. take the pointer/reference as non-const where it is genuinely mutated
2. a mutable member for a logical-const cache
3. const_cast only to bridge a const-incorrect API you do not own";

const UNSAFE_STR: &str = "\
unsafe string function — no bounds checking; the classic buffer overflow.
1. bounded C → snprintf / strncpy / strncat with an explicit size (mind truncation)
2. C++ → std::string / std::format / std::span
3. never sized from attacker-controlled input";

const NEW_CPP: &str = "\
raw new/delete — ownership leaks on every early return or exception.
1. value semantics / a container (vector, string)
2. unique_ptr via make_unique (single owner)
3. shared_ptr via make_shared (shared owner)
4. raw new only inside a RAII wrapper you fully own";

const MALLOC_CPP: &str = "\
malloc/free in C++ — no constructor or destructor runs, no RAII.
1. a value type or container
2. make_unique / make_shared
3. malloc only for C interop or placement-new arenas";

const CSTYLE_CAST: &str = "\
C-style cast — silently selects static/const/reinterpret; the intent is invisible.
1. numeric / derived→base → static_cast
2. add or drop const → const_cast (rarely)
3. bit reinterpret → reinterpret_cast / std::bit_cast
Name the cast so the reader sees what was meant.";

const USING_STD: &str = "\
`using namespace std` — imports the whole std namespace; in a header it leaks into every includer.
1. qualify names (std::vector) — mandatory in headers
2. using-declarations for the few names used (using std::vector;)
3. a using-directive only inside a .cpp function scope";

const MACRO_CPP: &str = "\
function-like macro — no types, no scope, no debugger; textual substitution surprises.
1. a constant → constexpr
2. a function → inline / constexpr function
3. generic code → a template
4. macros only for token-pasting / conditional compilation";

fn walk_c(node: Node, src: &[u8], _path: &str, out: &mut Out) {
    match node.kind() {
        "goto_statement" => push(out, node, "goto", GOTO, false),
        "call_expression"
            if callee_text(node, "function", src).is_some_and(|f| {
                matches!(
                    f,
                    "strcpy" | "strcat" | "sprintf" | "vsprintf" | "gets" | "scanf"
                )
            }) =>
        {
            push(out, node, "unsafe-str-fn", UNSAFE_STR, true);
        }
        _ => {}
    }
}

fn is_header(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("h" | "hpp" | "hh" | "hxx" | "h++")
    )
}

fn walk_cpp(node: Node, src: &[u8], path: &str, out: &mut Out) {
    walk_c(node, src, path, out);
    match node.kind() {
        "new_expression" | "delete_expression" => push(out, node, "raw-new-delete", NEW_CPP, false),
        "cast_expression" => push(out, node, "c-style-cast", CSTYLE_CAST, false),
        "preproc_function_def" => push(out, node, "function-macro", MACRO_CPP, false),
        "using_declaration"
            if node
                .utf8_text(src)
                .is_ok_and(|t| t.contains("namespace std")) =>
        {
            push(out, node, "using-namespace-std", USING_STD, is_header(path));
        }
        "call_expression"
            if callee_text(node, "function", src)
                .is_some_and(|f| matches!(f, "malloc" | "calloc" | "realloc" | "free")) =>
        {
            push(out, node, "manual-memory", MALLOC_CPP, false);
        }
        _ => {}
    }
    // named-cast keywords lead a call-like expression; no dedicated node kind.
    // ponytail: text-prefix match, may double-report when the cast heads a larger
    // expression — tighten to an exact node kind if that surfaces.
    if node.kind().contains("expression") {
        match node.utf8_text(src) {
            Ok(t) if t.starts_with("reinterpret_cast") => {
                push(out, node, "reinterpret_cast", REINTERPRET_CAST, false)
            }
            Ok(t) if t.starts_with("const_cast") => {
                push(out, node, "const-cast", CONST_CAST, false)
            }
            _ => {}
        }
    }
}

// -------------------------------------------------------------------- java

const REFLECTION_JAVA: &str = "\
reflection (setAccessible/forName) — breaks encapsulation and compile-time safety.
1. an interface + a normal call
2. a factory / ServiceLoader
3. reflection (last resort, e.g. a framework)";

fn walk_java(node: Node, src: &[u8], _path: &str, out: &mut Out) {
    match node.kind() {
        "catch_clause"
            if node
                .child_by_field_name("body")
                .is_some_and(|b| named(b).is_empty()) =>
        {
            push(out, node, "empty-catch", EMPTY_CATCH, true);
        }
        "method_invocation" if callee_text(node, "name", src) == Some("setAccessible") => {
            push(out, node, "reflection", REFLECTION_JAVA, false);
        }
        _ => {}
    }
}
