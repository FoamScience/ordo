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

const SQL_INJECT: &str = "\
formatted string as a SQL statement — injection by construction.
1. parameterized query → cur.execute(sql, params)
2. a query builder / ORM
3. string interpolation into SQL (never with external input)";

const YAML_LOAD: &str = "\
yaml.load without a safe Loader — constructs arbitrary objects / runs code on untrusted input.
1. yaml.safe_load(...)
2. yaml.load(..., Loader=SafeLoader)
3. an unsafe Loader (only for data you fully trust)";

const TLS_VERIFY: &str = "\
TLS verification disabled — certificates go unchecked, opening a MITM.
1. fix the trust store / pass verify=<ca_bundle>
2. pin the expected certificate
3. verify=False (only ever for a throwaway local script)";

const FIRE_FORGET: &str = "\
fire-and-forget task — the loop keeps only a weak ref, so it can be GC'd mid-flight and vanish.
1. await it, or gather it with others
2. an asyncio.TaskGroup (3.11+)
3. store the task in a set + add_done_callback to keep it alive";

const HALF_CM: &str = "\
half a context-manager protocol — only one of __enter__/__exit__ is defined, so `with` can't use it.
1. @contextlib.contextmanager over a generator
2. implement both halves (__enter__ and __exit__)
3. leave it (only if it is deliberately not a context manager)";

const GETATTRIBUTE_PY: &str = "\
__getattribute__ override — intercepts EVERY access (incl. dunders); easy infinite recursion, big slowdown.
1. __getattr__ (fires only on a missing attribute)
2. property / descriptors for specific attributes
3. __getattribute__ (only for a genuine transparent proxy)";

const SUPPRESS_BROAD: &str = "\
suppress(Exception/BaseException) — the explicit-API twin of bare except; swallows bugs and KeyboardInterrupt.
1. suppress(SpecificError)
2. try/except SpecificError with handling
3. a broad suppress (name the concrete type instead)";

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
            let has = |a: &str| methods.iter().any(|m| m == a);
            if has("__enter__") != has("__exit__") || has("__aenter__") != has("__aexit__") {
                push(out, node, "half-context-manager", HALF_CM, true);
            }
        }
        "function_definition" if callee_text(node, "name", src) == Some("__del__") => {
            push(out, node, "del-finalizer", DEL_PY, true);
        }
        "function_definition" if callee_text(node, "name", src) == Some("__getattribute__") => {
            push(out, node, "getattribute-override", GETATTRIBUTE_PY, false);
        }
        "function_definition" if node.utf8_text(src).is_ok_and(|t| t.starts_with("async")) => {
            scan_async_blocking(node, src, out);
        }
        "decorated_definition" => py_lru_method(node, src, out),
        "expression_statement" if py_fire_and_forget(node, src) => {
            push(out, node, "fire-and-forget-task", FIRE_FORGET, true);
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
                Some("ssl._create_unverified_context") => {
                    push(out, node, "tls-no-verify", TLS_VERIFY, true)
                }
                Some("yaml.load") | Some("yaml.load_all") if !py_safe_loader(node, src) => {
                    push(out, node, "yaml-load", YAML_LOAD, true)
                }
                Some("contextlib.suppress") | Some("suppress") if py_broad_suppress(node, src) => {
                    push(out, node, "broad-suppress", SUPPRESS_BROAD, false)
                }
                _ => {}
            }
            if fname.is_some_and(|f| f.starts_with("subprocess.")) && py_shell_true(node, src) {
                push(out, node, "shell-injection", SHELL_PY, true);
            }
            if fname.is_some_and(py_sql_sink) && py_dynamic_sql(node, src) {
                push(out, node, "sql-injection", SQL_INJECT, true);
            }
            if fname.is_some_and(py_http_callee) && py_kw_is(node, "verify", "False", src) {
                push(out, node, "tls-no-verify", TLS_VERIFY, true);
            }
        }
        _ => {}
    }
}

// a call to `x.execute` / `.executemany` / `.executescript` (a DB cursor sink)
fn py_sql_sink(f: &str) -> bool {
    matches!(
        f.rsplit('.').next(),
        Some("execute" | "executemany" | "executescript")
    ) && f.contains('.')
}

// the first argument to execute* is a built (not literal) string
fn py_dynamic_sql(call: Node, src: &[u8]) -> bool {
    let Some(arg) = call
        .child_by_field_name("arguments")
        .and_then(|a| named(a).into_iter().next())
    else {
        return false;
    };
    match arg.kind() {
        "string" => has_descendant(arg, "interpolation"),
        "binary_operator" => true, // `"..." % x` or `"..." + x`
        "call" => callee_text(arg, "function", src).is_some_and(|c| c.ends_with(".format")),
        _ => false,
    }
}

fn py_http_callee(f: &str) -> bool {
    f.starts_with("requests.")
        || f.starts_with("httpx.")
        || f.contains("ession.") // Session. / session.
        || matches!(
            f.rsplit('.').next(),
            Some("get" | "post" | "put" | "delete" | "patch" | "head" | "request")
        )
}

// call has a keyword argument `name` whose value renders exactly as `val`
fn py_kw_is(call: Node, name: &str, val: &str, src: &[u8]) -> bool {
    call.child_by_field_name("arguments").is_some_and(|args| {
        named(args).iter().any(|a| {
            a.kind() == "keyword_argument"
                && a.child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                    == Some(name)
                && a.child_by_field_name("value")
                    .and_then(|v| v.utf8_text(src).ok())
                    == Some(val)
        })
    })
}

// a yaml.load call that names a Safe loader
fn py_safe_loader(call: Node, src: &[u8]) -> bool {
    call.child_by_field_name("arguments").is_some_and(|args| {
        named(args).iter().any(|a| {
            a.kind() == "keyword_argument"
                && a.child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                    == Some("Loader")
                && a.child_by_field_name("value")
                    .and_then(|v| v.utf8_text(src).ok())
                    .is_some_and(|v| v.contains("Safe"))
        })
    })
}

// suppress(...) covering Exception / BaseException
fn py_broad_suppress(call: Node, src: &[u8]) -> bool {
    call.child_by_field_name("arguments").is_some_and(|args| {
        named(args)
            .iter()
            .any(|a| matches!(a.utf8_text(src), Ok("Exception") | Ok("BaseException")))
    })
}

// asyncio.create_task(...) / ensure_future(...) whose result is discarded
fn py_fire_and_forget(stmt: Node, src: &[u8]) -> bool {
    let kids = named(stmt);
    if kids.len() != 1 || kids[0].kind() != "call" {
        return false;
    }
    callee_text(kids[0], "function", src).is_some_and(|c| {
        c == "asyncio.create_task" || c == "asyncio.ensure_future" || c.ends_with(".create_task")
    })
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

fn has_descendant(node: Node, kind: &str) -> bool {
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        if ch.kind() == kind || has_descendant(ch, kind) {
            return true;
        }
    }
    false
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

const THROW_DTOR: &str = "\
throw in a destructor / noexcept function — if it escapes during unwinding, std::terminate is called.
1. handle and log inside the destructor (never let it escape)
2. move the fallible work to an explicit close()/commit() that may throw
3. throw here (only if you can prove it never fires during unwinding)";

const SETJMP_CPP: &str = "\
setjmp/longjmp in C++ — jumping across frames skips destructors of live objects: undefined behavior.
1. exceptions for error propagation
2. return values → std::optional / std::expected
3. setjmp only in pure-C interop with trivially-destructible state";

const OPERATOR_LOGIC: &str = "\
overloading operator&& / || / , — silently destroys short-circuit and sequencing every caller assumes.
1. a named member function (.both(), .either())
2. a free function with an explicit name
3. never overload these (only defensible in EDSLs you fully control)";

const MEM_FAMILY: &str = "\
memcpy/memset on objects — byte-blitting corrupts non-trivially-copyable types (vtables, ownership).
1. std::copy / std::fill / assignment for objects
2. std::span / vector assignment for buffers
3. memcpy only for trivially-copyable data (guard with static_assert(is_trivially_copyable))";

const SYSTEM_EXEC: &str = "\
system/popen/exec* — shell-out with injection risk and awkward error handling.
1. a library API for the task (<filesystem>, etc. — no shell)
2. posix_spawn/exec with an explicit argv array (no shell parsing)
3. system() only with a fully literal, non-interpolated command";

const DYNAMIC_CAST: &str = "\
dynamic_cast on a normal path — usually a type-switch that should be virtual dispatch; RTTI cost + null traps.
1. add a virtual method and let the vtable dispatch
2. a visitor / std::variant + std::visit
3. dynamic_cast only across a genuine unrelated-hierarchy boundary";

const LAMBDA_REF: &str = "\
[&] default-reference capture — dangles the moment the lambda outlives the scope (stored callback, thread, async).
1. capture the few names you need explicitly (by value or ref)
2. [=] or move-capture ([x = std::move(x)]) when it escapes
3. [&] only for an immediately-used local lambda (sort comparator, for_each)";

const CATCH_VALUE: &str = "\
catch by value — slices a derived exception to its base and copies (the copy can itself throw).
1. catch (const E&)
2. catch (const std::exception&) at boundaries
3. catch by value only for a small error-code value type";

const ALLOCA_CPP: &str = "\
alloca — unchecked stack allocation; overflow is silent UB and the lifetime is the whole function.
1. std::array when the bound is known at compile time
2. std::vector / std::string (or a small-buffer type) for dynamic size
3. alloca only for tiny, bounded, hot-path scratch";

const NONREENTRANT: &str = "\
non-reentrant C runtime — shared static buffers / poor quality; data races and clobbering.
1. <random> (mt19937) for rand; std::string / string_view for strtok
2. the _r/_s reentrant variant, or std::chrono + <format> for the time funcs
3. the bare call (only single-threaded, non-security code)";

const VOLATILE_CPP: &str = "\
volatile as a threading primitive — gives no atomicity, ordering, or cross-thread visibility.
1. std::atomic<T> for flags/counters
2. a mutex-guarded value for compound invariants
3. volatile only for memory-mapped I/O or sig_atomic_t signal handlers";

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
        "type_qualifier" if node.utf8_text(src) == Ok("volatile") => {
            push(out, node, "volatile", VOLATILE_CPP, false)
        }
        "throw_statement" if cpp_throw_unwind(node, src) => {
            push(out, node, "throw-in-destructor", THROW_DTOR, true)
        }
        "lambda_expression" if cpp_lambda_default_ref(node, src) => {
            push(out, node, "lambda-ref-capture", LAMBDA_REF, false)
        }
        "catch_clause" if cpp_catch_by_value(node) => {
            push(out, node, "catch-by-value", CATCH_VALUE, false)
        }
        "function_definition" | "field_declaration" if cpp_bad_operator(node, src) => {
            push(out, node, "operator-logical", OPERATOR_LOGIC, true)
        }
        "call_expression" => cpp_call(node, src, out),
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
            Ok(t) if t.starts_with("dynamic_cast") => {
                push(out, node, "dynamic-cast", DYNAMIC_CAST, false)
            }
            _ => {}
        }
    }
}

// dispatch a C++ call by callee (std:: prefix stripped)
fn cpp_call(node: Node, src: &[u8], out: &mut Out) {
    let Some(f) = callee_text(node, "function", src) else {
        return;
    };
    let base = f.rsplit("::").next().unwrap_or(f);
    if matches!(base, "malloc" | "calloc" | "realloc" | "free") {
        push(out, node, "manual-memory", MALLOC_CPP, false);
    } else if matches!(
        base,
        "memcpy" | "memmove" | "memset" | "memcmp" | "bcopy" | "bzero"
    ) {
        push(out, node, "mem-family", MEM_FAMILY, false);
    } else if matches!(
        base,
        "system" | "popen" | "execl" | "execlp" | "execle" | "execv" | "execvp" | "execvpe"
    ) {
        push(out, node, "shell-exec", SYSTEM_EXEC, false);
    } else if matches!(base, "alloca" | "_alloca" | "_malloca") {
        push(out, node, "alloca", ALLOCA_CPP, false);
    } else if matches!(
        base,
        "rand" | "srand" | "strtok" | "localtime" | "gmtime" | "asctime" | "ctime"
    ) {
        push(out, node, "non-reentrant", NONREENTRANT, false);
    } else if matches!(
        base,
        "setjmp" | "_setjmp" | "sigsetjmp" | "longjmp" | "siglongjmp"
    ) {
        push(out, node, "setjmp-longjmp", SETJMP_CPP, true);
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
                let txt = p.utf8_text(src).unwrap_or("");
                let header = txt.split('{').next().unwrap_or(txt);
                return header.contains('~') || header.contains("noexcept");
            }
            _ => {}
        }
        cur = p;
    }
    false
}

// a [&] default-reference-capture lambda (not [&x], an explicit single capture)
fn cpp_lambda_default_ref(node: Node, src: &[u8]) -> bool {
    let t = node.utf8_text(src).unwrap_or("");
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.split(']').next())
        .unwrap_or("")
        .trim();
    let mut c = inner.chars();
    c.next() == Some('&') && matches!(c.next(), None | Some(','))
}

// catch (E e) by value on a class type — slices; skip catch(const E&) and catch(int)
fn cpp_catch_by_value(node: Node) -> bool {
    let Some(params) = node.child_by_field_name("parameters") else {
        return false; // catch(...) has no parameter
    };
    let Some(decl) = named(params)
        .into_iter()
        .find(|c| c.kind() == "parameter_declaration")
    else {
        return false;
    };
    let by_ref = named(decl).iter().any(|c| {
        matches!(
            c.kind(),
            "reference_declarator" | "pointer_declarator" | "abstract_reference_declarator"
        )
    });
    let class_type = matches!(
        decl.child_by_field_name("type").map(|t| t.kind()),
        Some("type_identifier" | "qualified_identifier" | "template_type")
    );
    !by_ref && class_type
}

// a definition/declaration of operator&& / operator|| / operator,
fn cpp_bad_operator(node: Node, src: &[u8]) -> bool {
    let head = node
        .utf8_text(src)
        .unwrap_or("")
        .split('{')
        .next()
        .unwrap_or("");
    head.contains("operator&&") || head.contains("operator||") || head.contains("operator,")
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
