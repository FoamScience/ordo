//! Advanced-construct advisories (P14): detect powerful/overusable language
//! constructs and attach escalation-ladder guidance, with a downgrade *verdict*
//! only where the pattern is deterministically wrong. A curated catalog of
//! senior review knowledge — deliberately NOT a style linter.
//!
//! **Most of the catalog is data now** — `rulesets/catalog/*.toml`, compiled
//! into the engine by `src/catalog.rs` (tasks-9sj.33). What is left here is
//! what the rule language cannot state, and each one is here for a reason
//! worth reading before trying again:
//!
//! *Needs a fact about a sibling or an ancestor.* A tree-sitter query has no
//! descendant axis and `when.with`/`without` see direct children only, so
//! `blocking-in-async` (a blocking call anywhere inside an `async def`),
//! `throw-in-destructor` (a throw whose *nearest enclosing function* is a
//! destructor) and `half-context-manager` (`__enter__` XOR `__exit__`) cannot
//! be written as one pattern.
//!
//! *The message depends on what was found.* `metaclass` names the dunders the
//! class actually overrides; a rule carries one fixed message.
//!
//! *The level depends on the path.* `using-namespace-std` is a verdict in a
//! header and a note elsewhere, and one rule has one level.
//!
//! *Matches a node kind the grammar does not name.* `reinterpret_cast`,
//! `const-cast` and `dynamic-cast` are matched by text prefix across every
//! node whose kind merely *contains* `expression`; `when.kind` takes exact
//! kinds, so a rule would fire on a different set of rows.
//!
//! *The row is not the node's own.* `lru-cache-on-method` is decided on the
//! decorated definition but reported on the `def` line inside it, and a rule
//! reports the node it matched.
//!
//! What *looked* like the same problem — `yaml-load` unless a Safe loader is
//! named, `shell-injection` (`shell=True`), `tls-no-verify` (`verify=False`),
//! `broad-suppress`, `dynamic-type` (exactly three arguments), `sql-injection`
//! (a built, not literal, first argument), `fire-and-forget-task`,
//! `bare-except` and `empty-catch` — turned out statable: a keyword argument
//! is a `(keyword_argument name: … value: (true))` pattern, a child count is
//! a query with anchors, and the one text test left (yaml's Safe loader) is
//! a `text-not` regex. They live in `rulesets/catalog/python.toml` now, on
//! the same rows, which `corpus/catalog.txt` records.
//!
//! `operator-logical` is the one remaining case that is merely awkward rather
//! than impossible: it needs to match `operator&&` in a signature but not in a
//! body, which a `text` regex cannot anchor to the header.
use crate::lang::LangSpec;
use crate::model::{Finding, FindingSource, Level};
use tree_sitter::Node;

/// Detect constructs in the parsed *new* tree → (0-based start row, advisory).
pub fn advise(spec: &LangSpec, root: Node, src: &[u8], path: &str) -> Vec<(usize, Finding)> {
    let mut out = vec![];
    let walker: Rule = match spec.name {
        "python" | "xonsh" => walk_python,
        "cpp" => walk_cpp,
        _ => return out,
    };
    walk(root, src, path, walker, &mut out);
    out
}

type Out = Vec<(usize, Finding)>;
type Rule = fn(Node, &[u8], &str, &mut Out);

// An explicit stack, not recursion: a long chain of binary expressions — a
// minified bundle is the usual source — nests deep enough to take the process
// down with it, and a stack overflow is not something `run` can degrade from.
fn walk(node: Node, src: &[u8], path: &str, rule: Rule, out: &mut Out) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        rule(n, src, path, out);
        // pushed in reverse so popping walks the children left to right, the
        // order the recursive version emitted advisories in
        let mut cur = n.walk();
        let kids: Vec<Node> = n.named_children(&mut cur).collect();
        stack.extend(kids.into_iter().rev());
    }
}

fn push(out: &mut Out, node: Node, construct: &str, message: &str, verdict: bool) {
    out.push((
        node.start_position().row,
        Finding {
            source: FindingSource::Catalog,
            name: construct.into(),
            message: message.into(),
            // `verdict` was a bool meaning "a concrete downgrade is suggested";
            // it is the third level now (see `model::Level`)
            level: if verdict { Level::Verdict } else { Level::Note },
        },
    ));
}

fn callee_text<'a>(call: Node, field: &str, src: &'a [u8]) -> Option<&'a str> {
    call.child_by_field_name(field)
        .and_then(|f| f.utf8_text(src).ok())
}

// true if the node's raw bytes start with `prefix`, without UTF-8-validating
// the node's whole (possibly deeply-nested) span just to check a few bytes.
fn starts_with_at(node: Node, src: &[u8], prefix: &[u8]) -> bool {
    src.get(node.start_byte()..)
        .is_some_and(|s| s.starts_with(prefix))
}

// bytes from node's start up to its first `{` (or its end if none) — the
// "header" a caller wants to grep, without validating the whole body as utf8.
fn header_bytes<'a>(node: Node, src: &'a [u8]) -> &'a [u8] {
    let start = node.start_byte();
    let end = node.end_byte().min(src.len());
    let bytes = src.get(start..end).unwrap_or(&[]);
    let brace = bytes.iter().position(|&b| b == b'{').unwrap_or(bytes.len());
    &bytes[..brace]
}

fn contains_bytes(hay: &[u8], needle: &[u8]) -> bool {
    // `windows(0)` panics; no caller passes an empty needle, and none should
    // have to know that to stay safe
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn named<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut cur = node.walk();
    node.named_children(&mut cur).collect()
}

// ------------------------------------------------------------------- python

const METACLASS_LADDER: &str = "\
metaclass — 90% of the time the wrong tool. Lightest sufficient step:
1. configure one attribute → __set_name__ (descriptor)
2. react to subclassing (register/validate/defaults) → __init_subclass__
3. replace the class after it's built → class decorator
4. rewrite the class as it's built, or control instance creation → metaclass";

const BLOCKING_ASYNC: &str = "\
blocking call in an async function — stalls the whole event loop, defeating async.
1. the async equivalent (asyncio.sleep, httpx/aiohttp, aiofiles)
2. offload to a thread → await asyncio.to_thread(...) / loop.run_in_executor
3. a blocking call inline (only if the loop truly has nothing else to do)";

const LRU_METHOD: &str = "\
lru_cache/cache on a method — the cache holds `self`, pinning every instance forever (leak).
1. per-instance memoization → functools.cached_property
2. cache a module-level function taking only hashable args
3. lru_cache on the bound method (only if instances are singletons)";

const HALF_CM: &str = "\
half a context-manager protocol — only one of __enter__/__exit__ is defined, so `with` can't use it.
1. @contextlib.contextmanager over a generator
2. implement both halves (__enter__ and __exit__)
3. leave it (only if it is deliberately not a context manager)";

fn walk_python(node: Node, src: &[u8], _path: &str, out: &mut Out) {
    match node.kind() {
        "class_definition" => {
            if let Some(a) = python_metaclass(node, src) {
                out.push((node.start_position().row, a));
            }
            let methods = class_methods(node, src);
            let has = |a: &str| methods.iter().any(|m| m == a);
            if has("__enter__") != has("__exit__") || has("__aenter__") != has("__aexit__") {
                push(out, node, "half-context-manager", HALF_CM, true);
            }
        }
        "function_definition" if starts_with_at(node, src, b"async") => {
            scan_async_blocking(node, src, out);
        }
        "decorated_definition" => py_lru_method(node, src, out),
        _ => {}
    }
}

// lru_cache/cache decorating a method whose first parameter is self/cls
fn py_lru_method(dec: Node, src: &[u8], out: &mut Out) {
    let kids = named(dec);
    let Some(func) = kids.iter().find(|c| c.kind() == "function_definition") else {
        return;
    };
    let decs: Vec<&str> = kids
        .iter()
        .filter(|c| c.kind() == "decorator")
        .filter_map(|d| d.utf8_text(src).ok())
        .collect();
    if decs.iter().any(|d| d.contains("staticmethod")) {
        return;
    }
    let cached = decs.iter().any(|d| {
        let d = d.trim_start_matches('@').trim_start_matches("functools.");
        d.starts_with("lru_cache") || d.starts_with("cache")
    });
    let on_method = func
        .child_by_field_name("parameters")
        .and_then(|p| named(p).into_iter().next())
        .and_then(|p| p.utf8_text(src).ok())
        .is_some_and(|p| p == "self" || p == "cls");
    if cached && on_method {
        push(out, *func, "lru-cache-on-method", LRU_METHOD, true);
    }
}

// scan an async function body for known-blocking sync calls, not descending
// into nested function/lambda scopes.
const BLOCKING: &[&str] = &[
    "time.sleep",
    "requests.get",
    "requests.post",
    "requests.put",
    "requests.delete",
    "requests.patch",
    "requests.head",
    "urllib.request.urlopen",
    "subprocess.run",
    "subprocess.call",
    "subprocess.check_output",
    "subprocess.Popen",
];

fn scan_async_blocking(node: Node, src: &[u8], out: &mut Out) {
    for ch in named(node) {
        match ch.kind() {
            "function_definition" | "lambda" => continue, // its own scope
            "call" => {
                if callee_text(ch, "function", src).is_some_and(|c| BLOCKING.contains(&c)) {
                    push(out, ch, "blocking-in-async", BLOCKING_ASYNC, true);
                }
                scan_async_blocking(ch, src, out);
            }
            _ => scan_async_blocking(ch, src, out),
        }
    }
}

fn python_metaclass(class: Node, src: &[u8]) -> Option<Finding> {
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
        Some(Finding {
            source: FindingSource::Catalog,
            name: "metaclass".into(),
            message,
            level: if verdict { Level::Verdict } else { Level::Note },
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

// ---------------------------------------------------------------- javascript

// ----------------------------------------------------------------------- go

// -------------------------------------------------------------------- c / c++

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

const USING_STD: &str = "\
`using namespace std` — imports the whole std namespace; in a header it leaks into every includer.
1. qualify names (std::vector) — mandatory in headers
2. using-declarations for the few names used (using std::vector;)
3. a using-directive only inside a .cpp function scope";

const THROW_DTOR: &str = "\
throw in a destructor / noexcept function — if it escapes during unwinding, std::terminate is called.
1. handle and log inside the destructor (never let it escape)
2. move the fallible work to an explicit close()/commit() that may throw
3. throw here (only if you can prove it never fires during unwinding)";

const OPERATOR_LOGIC: &str = "\
overloading operator&& / || / , — silently destroys short-circuit and sequencing every caller assumes.
1. a named member function (.both(), .either())
2. a free function with an explicit name
3. never overload these (only defensible in EDSLs you fully control)";

const DYNAMIC_CAST: &str = "\
dynamic_cast on a normal path — usually a type-switch that should be virtual dispatch; RTTI cost + null traps.
1. add a virtual method and let the vtable dispatch
2. a visitor / std::variant + std::visit
3. dynamic_cast only across a genuine unrelated-hierarchy boundary";

fn is_header(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("h" | "hpp" | "hh" | "hxx" | "h++")
    )
}

fn walk_cpp(node: Node, src: &[u8], path: &str, out: &mut Out) {
    match node.kind() {
        "using_declaration"
            if node
                .utf8_text(src)
                .is_ok_and(|t| t.contains("namespace std")) =>
        {
            push(out, node, "using-namespace-std", USING_STD, is_header(path));
        }
        "throw_statement" if cpp_throw_unwind(node, src) => {
            push(out, node, "throw-in-destructor", THROW_DTOR, true)
        }
        "function_definition" | "field_declaration" if cpp_bad_operator(node, src) => {
            push(out, node, "operator-logical", OPERATOR_LOGIC, true)
        }
        _ => {}
    }
    // named-cast keywords lead a call-like expression; no dedicated node kind.
    // ponytail: text-prefix match, may double-report when the cast heads a larger
    // expression — tighten to an exact node kind if that surfaces.
    if node.kind().contains("expression") {
        if starts_with_at(node, src, b"reinterpret_cast") {
            push(out, node, "reinterpret_cast", REINTERPRET_CAST, false)
        } else if starts_with_at(node, src, b"const_cast") {
            push(out, node, "const-cast", CONST_CAST, false)
        } else if starts_with_at(node, src, b"dynamic_cast") {
            push(out, node, "dynamic-cast", DYNAMIC_CAST, false)
        }
    }
}

// a throw whose nearest enclosing function is a destructor or noexcept (→ terminate
// if it escapes during unwinding). Stops at a nested lambda/function scope.
fn cpp_throw_unwind(node: Node, src: &[u8]) -> bool {
    let mut cur = node;
    while let Some(p) = cur.parent() {
        match p.kind() {
            "lambda_expression" => return false,
            "function_definition" => {
                let header = header_bytes(p, src);
                return header.contains(&b'~') || contains_bytes(header, b"noexcept");
            }
            _ => {}
        }
        cur = p;
    }
    false
}

// a definition/declaration of operator&& / operator|| / operator,
fn cpp_bad_operator(node: Node, src: &[u8]) -> bool {
    let head = header_bytes(node, src);
    contains_bytes(head, b"operator&&")
        || contains_bytes(head, b"operator||")
        || contains_bytes(head, b"operator,")
}

// -------------------------------------------------------------------- java
