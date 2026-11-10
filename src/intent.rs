//! Edit intents: what a small edit set out to do, read off the old and new
//! syntax trees rather than off the lines — a guard clause added, statements
//! wrapped in a try, a condition narrowed, a comparison's boundary moved. Each
//! is a structured fact on the hunk (`HunkSem::intent`): the headline words
//! it, and a summary of the whole change can count it.
//!
//! The patterns are fixed; what differs between languages is which node kinds
//! and tokens spell them, and that is data — one `Spelling` per grammar family
//! in `SPELLINGS`, and the operator swaps in `BOUNDARY` and `NEGATED`. The
//! construct catalog's rule language cannot carry them: a rule matches a shape
//! in one tree, and an edit pattern relates two.
use crate::extract::{squeeze, HunkSem, RawHunk};
use crate::lang::{self, LangSpec};
use crate::refine::{Role, SingleChange, Span};
use tree_sitter::{Node, Point};

/// One recognised edit pattern, with the text its headline quotes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// an inserted `if`, or a `let … else`, whose one statement leaves early
    Guard { cond: String, exit: Exit },
    /// the lines that were there now run inside a try: the call they make
    /// when they are one statement making one, what the handlers catch, and
    /// the language's name for the construct
    WrapsTry {
        call: Option<String>,
        lines: usize,
        caught: Option<String>,
        word: &'static str,
    },
    /// an `and`/`or` operand added to or dropped from a condition
    Condition {
        narrows: bool,
        added: bool,
        clause: String,
    },
    /// `<` ↔ `<=`, `>` ↔ `>=`: the operators and the comparison, before and after
    Boundary {
        ops: [String; 2],
        old: String,
        new: String,
    },
    /// the whole condition negated, before and after
    Inverts { old: String, new: String },
    /// a literal replaced by a constant's name
    NamesLiteral { literal: String, name: String },
    /// a parameter with a default appended, so every existing call still
    /// binds; `func` is the function's name when it has one
    DefaultParam { param: String, func: Option<String> },
    /// `async` added to or dropped from a function's signature
    Async { added: bool, func: Option<String> },
    /// the same call, made to another function
    SwapsCall { old: String, new: String },
    /// a loop done in place replaced by one call that the old lines did not
    /// make: the loop when it was the only one, else how many lines went
    ReplacedByCall {
        call: String,
        construct: Option<&'static str>,
        lines: usize,
    },
}

/// How a guard leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    Return,
    Raise,
    Throw,
    Continue,
    Break,
    /// a jump forward to a label, c's error path
    Goto(String),
}

/// How one grammar family spells the patterns: node kinds as each grammar's
/// node-types.json names them, tokens as their text.
struct Spelling {
    langs: &'static [&'static str],
    /// `if` statements: a guard reads their `condition` and `consequence`
    ifs: &'static [&'static str],
    /// statements that leave early, and how
    exits: &'static [(&'static str, Exit)],
    /// a jump and the labelled statement it lands on: a guard when the label
    /// comes later in the function
    gotos: &'static [(&'static str, &'static str)],
    /// a binding whose `alternative` runs when its `pattern` does not match
    /// its `value` (rust's `let … else`)
    let_elses: &'static [&'static str],
    /// what holds statements without changing them: a block, an expression
    /// statement
    blocks: &'static [&'static str],
    /// what holds an expression without changing it: parentheses, c++'s
    /// condition clause
    parens: &'static [&'static str],
    /// conditions that bind a pattern rather than test a value (`if let`):
    /// "returns early when let Some(x) = y" says nothing a reader can use
    binding_conds: &'static [&'static str],
    /// `None` for a language without exceptions
    tries: Option<Tries>,
    /// binary expressions — comparisons, `and`/`or`
    binary: &'static [&'static str],
    and: &'static [&'static str],
    or: &'static [&'static str],
    /// negation tokens, and what holds one: a prefix `not`/`!`, or a
    /// compound comparison operator (`is not`)
    not: &'static [&'static str],
    negations: &'static [&'static str],
    params: &'static [&'static str],
    /// a parameter carries a default when its kind, or one of its fields, is
    /// listed
    defaulted: &'static [&'static str],
    /// parameters a new defaulted one may still go in front of without
    /// moving any caller's argument (python's `**kwargs`)
    keyword_rest: &'static [&'static str],
    async_kw: Option<&'static str>,
    /// loop statements, and what a headline calls each
    loops: &'static [(&'static str, &'static str)],
}

/// How a language spells a try statement.
struct Tries {
    kinds: &'static [&'static str],
    handlers: &'static [&'static str],
    /// from a handler to the type it catches: a field name or a child kind
    /// per step
    caught: &'static [&'static str],
    /// the language's name for a try with handlers
    word: &'static str,
}

const C_LIKE_BINARY: &[&str] = &["binary_expression"];
const C_LIKE_PARENS: &[&str] = &["parenthesized_expression"];

static SPELLINGS: &[Spelling] = &[
    Spelling {
        langs: &["python", "xonsh"],
        ifs: &["if_statement"],
        exits: &[
            ("return_statement", Exit::Return),
            ("raise_statement", Exit::Raise),
            ("continue_statement", Exit::Continue),
            ("break_statement", Exit::Break),
        ],
        gotos: &[],
        let_elses: &[],
        blocks: &["block", "expression_statement"],
        parens: &["parenthesized_expression"],
        binding_conds: &[],
        tries: Some(Tries {
            kinds: &["try_statement"],
            handlers: &["except_clause", "except_group_clause"],
            caught: &["value"],
            word: "try/except",
        }),
        binary: &["comparison_operator", "boolean_operator"],
        and: &["and"],
        or: &["or"],
        not: &["not"],
        negations: &["not_operator", "is not", "not in"],
        params: &["parameters"],
        defaulted: &["default_parameter", "typed_default_parameter"],
        keyword_rest: &["dictionary_splat_pattern"],
        async_kw: Some("async"),
        loops: &[
            ("for_statement", "for loop"),
            ("while_statement", "while loop"),
        ],
    },
    Spelling {
        langs: &["javascript", "typescript", "tsx"],
        ifs: &["if_statement"],
        exits: &[
            ("return_statement", Exit::Return),
            ("throw_statement", Exit::Throw),
            ("continue_statement", Exit::Continue),
            ("break_statement", Exit::Break),
        ],
        gotos: &[],
        let_elses: &[],
        blocks: &["statement_block", "expression_statement"],
        parens: C_LIKE_PARENS,
        binding_conds: &[],
        tries: Some(Tries {
            kinds: &["try_statement"],
            handlers: &["catch_clause"],
            caught: &[],
            word: "try/catch",
        }),
        binary: C_LIKE_BINARY,
        and: &["&&"],
        or: &["||"],
        not: &["!"],
        negations: &["unary_expression"],
        params: &["formal_parameters"],
        defaulted: &["assignment_pattern", "optional_parameter", "value"],
        keyword_rest: &[],
        async_kw: Some("async"),
        loops: &[
            ("for_statement", "for loop"),
            ("for_in_statement", "for loop"),
            ("while_statement", "while loop"),
            ("do_statement", "do/while loop"),
        ],
    },
    Spelling {
        langs: &["rust"],
        ifs: &["if_expression"],
        exits: &[
            ("return_expression", Exit::Return),
            ("continue_expression", Exit::Continue),
            ("break_expression", Exit::Break),
        ],
        gotos: &[],
        let_elses: &["let_declaration"],
        blocks: &["block", "expression_statement"],
        parens: C_LIKE_PARENS,
        binding_conds: &["let_condition", "let_chain"],
        tries: None,
        binary: C_LIKE_BINARY,
        and: &["&&"],
        or: &["||"],
        not: &["!"],
        negations: &["unary_expression"],
        params: &["parameters"],
        defaulted: &[],
        keyword_rest: &[],
        async_kw: Some("async"),
        loops: &[
            ("for_expression", "for loop"),
            ("while_expression", "while loop"),
            ("loop_expression", "loop"),
        ],
    },
    Spelling {
        langs: &["go"],
        ifs: &["if_statement"],
        exits: &[
            ("return_statement", Exit::Return),
            ("continue_statement", Exit::Continue),
            ("break_statement", Exit::Break),
        ],
        gotos: &[],
        let_elses: &[],
        blocks: &["block", "statement_list", "expression_statement"],
        parens: C_LIKE_PARENS,
        binding_conds: &[],
        tries: None,
        binary: C_LIKE_BINARY,
        and: &["&&"],
        or: &["||"],
        not: &["!"],
        negations: &["unary_expression"],
        params: &["parameter_list"],
        defaulted: &[],
        keyword_rest: &[],
        async_kw: None,
        loops: &[("for_statement", "for loop")],
    },
    Spelling {
        langs: &["c"],
        ifs: &["if_statement"],
        exits: &[
            ("return_statement", Exit::Return),
            ("continue_statement", Exit::Continue),
            ("break_statement", Exit::Break),
        ],
        gotos: &[("goto_statement", "labeled_statement")],
        let_elses: &[],
        blocks: &["compound_statement", "expression_statement"],
        parens: C_LIKE_PARENS,
        binding_conds: &[],
        tries: None,
        binary: C_LIKE_BINARY,
        and: &["&&"],
        or: &["||"],
        not: &["!"],
        negations: &["unary_expression"],
        params: &["parameter_list"],
        defaulted: &[],
        keyword_rest: &[],
        async_kw: None,
        loops: &[
            ("for_statement", "for loop"),
            ("while_statement", "while loop"),
            ("do_statement", "do/while loop"),
        ],
    },
    Spelling {
        langs: &["cpp"],
        ifs: &["if_statement"],
        exits: &[
            ("return_statement", Exit::Return),
            ("throw_statement", Exit::Throw),
            ("continue_statement", Exit::Continue),
            ("break_statement", Exit::Break),
        ],
        gotos: &[("goto_statement", "labeled_statement")],
        let_elses: &[],
        blocks: &["compound_statement", "expression_statement"],
        parens: &["parenthesized_expression", "condition_clause"],
        binding_conds: &[],
        tries: Some(Tries {
            kinds: &["try_statement"],
            handlers: &["catch_clause"],
            caught: &["parameters", "parameter_declaration", "type"],
            word: "try/catch",
        }),
        binary: C_LIKE_BINARY,
        and: &["&&", "and"],
        or: &["||", "or"],
        not: &["!", "not"],
        negations: &["unary_expression"],
        params: &["parameter_list"],
        defaulted: &["optional_parameter_declaration"],
        keyword_rest: &[],
        async_kw: None,
        loops: &[
            ("for_statement", "for loop"),
            ("for_range_loop", "for loop"),
            ("while_statement", "while loop"),
            ("do_statement", "do/while loop"),
        ],
    },
    Spelling {
        langs: &["java"],
        ifs: &["if_statement"],
        exits: &[
            ("return_statement", Exit::Return),
            ("throw_statement", Exit::Throw),
            ("continue_statement", Exit::Continue),
            ("break_statement", Exit::Break),
        ],
        gotos: &[],
        let_elses: &[],
        blocks: &["block", "expression_statement"],
        parens: C_LIKE_PARENS,
        binding_conds: &[],
        tries: Some(Tries {
            kinds: &["try_statement"],
            handlers: &["catch_clause"],
            caught: &["catch_formal_parameter", "catch_type"],
            word: "try/catch",
        }),
        binary: C_LIKE_BINARY,
        and: &["&&"],
        or: &["||"],
        not: &["!"],
        negations: &["unary_expression"],
        params: &["formal_parameters"],
        defaulted: &[],
        keyword_rest: &[],
        async_kw: None,
        loops: &[
            ("for_statement", "for loop"),
            ("enhanced_for_statement", "for loop"),
            ("while_statement", "while loop"),
            ("do_statement", "do/while loop"),
        ],
    },
    Spelling {
        langs: &["lua"],
        ifs: &["if_statement"],
        exits: &[
            ("return_statement", Exit::Return),
            ("break_statement", Exit::Break),
        ],
        gotos: &[],
        let_elses: &[],
        blocks: &["block"],
        parens: C_LIKE_PARENS,
        binding_conds: &[],
        tries: None,
        binary: C_LIKE_BINARY,
        and: &["and"],
        or: &["or"],
        not: &["not"],
        negations: &["unary_expression"],
        params: &["parameters"],
        defaulted: &[],
        keyword_rest: &[],
        async_kw: None,
        loops: &[
            ("for_statement", "for loop"),
            ("while_statement", "while loop"),
            ("repeat_statement", "repeat loop"),
        ],
    },
];

/// Comparison operator swaps that move a boundary without turning it round.
const BOUNDARY: &[(&str, &str)] = &[("<", "<="), ("<=", "<"), (">", ">="), (">=", ">")];

/// Comparison operator swaps that negate the comparison.
const NEGATED: &[(&str, &str)] = &[
    ("==", "!="),
    ("!=", "=="),
    ("===", "!=="),
    ("!==", "==="),
    ("==", "~="),
    ("~=", "=="),
    ("<", ">="),
    (">=", "<"),
    (">", "<="),
    ("<=", ">"),
];

/// Longest pure insertion, in lines, still read as one guard clause.
const GUARD_LINES: usize = 8;
/// Longest run of old lines a try is recognised as wrapping.
const TRY_LINES: usize = 40;
/// Longest hunk side, in lines, an added parameter is looked for in.
const PARAM_LINES: usize = 3;

/// Every hunk's intent, where one of the patterns recognises it.
pub(crate) fn fill(
    raw: &[RawHunk],
    sems: &mut [HunkSem],
    (old, new): (&str, &str),
    spec: &LangSpec,
) {
    let spelling = SPELLINGS.iter().find(|s| s.langs.contains(&spec.name));
    let trees = spelling
        .filter(|_| !old.is_empty())
        .and_then(|_| Some((lang::parse(spec, old)?, lang::parse(spec, new)?)));
    let starts = [line_starts(old), line_starts(new)];
    let sides = trees.as_ref().map(|(ot, nt)| {
        [
            Side {
                root: ot.root_node(),
                src: old,
                starts: &starts[0],
            },
            Side {
                root: nt.root_node(),
                src: new,
                starts: &starts[1],
            },
        ]
    });
    for (h, sem) in raw.iter().zip(sems.iter_mut()) {
        let swapped = sem.single.as_ref().and_then(swapped_call);
        sem.intent = swapped.or_else(|| {
            let (sp, sides) = (spelling?, sides.as_ref()?);
            sem.single
                .as_ref()
                .and_then(|c| token_intent(sp, c, sides))
                .or_else(|| hunk_intent(sp, h, sides))
        });
    }
    let (Some(sp), Some(sides)) = (spelling, sides.as_ref()) else {
        return;
    };
    for i in 1..raw.len() {
        if sems[i - 1].intent.is_some() || sems[i].intent.is_some() {
            continue;
        }
        if let Some(wrap) = split_try(sp, sides, &raw[i - 1], &raw[i]) {
            sems[i - 1].intent = Some(wrap.clone());
            sems[i].intent = Some(wrap);
        }
    }
}

/// Where each line of `src` starts, in bytes.
fn line_starts(src: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(src.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// One side of the change: its tree, the source it was parsed from and where
/// that source's lines start.
#[derive(Clone, Copy)]
struct Side<'t> {
    root: Node<'t>,
    src: &'t str,
    starts: &'t [usize],
}

impl<'t> Side<'t> {
    fn text(&self, n: Node) -> &'t str {
        &self.src[n.byte_range()]
    }

    /// The smallest node covering chars `a..b` of `row`.
    fn node_at(&self, row: usize, (a, b): Span) -> Option<Node<'t>> {
        let line = &self.src[self.rows((row, row))?];
        let byte = |c: usize| line.char_indices().nth(c).map_or(line.len(), |(i, _)| i);
        self.root
            .descendant_for_point_range(Point::new(row, byte(a)), Point::new(row, byte(b)))
    }

    /// The bytes of rows `a..=b`, without the last one's line break.
    fn rows(&self, (a, b): (usize, usize)) -> Option<std::ops::Range<usize>> {
        let start = *self.starts.get(a)?;
        let end = self
            .starts
            .get(b + 1)
            .map_or(self.src.len(), |next| next - 1);
        Some(start..end)
    }
}

fn swapped_call(c: &SingleChange) -> Option<Intent> {
    (c.role == Role::Callee && !c.old.is_empty() && !c.new.is_empty()).then(|| Intent::SwapsCall {
        old: c.old.clone(),
        new: c.new.clone(),
    })
}

/// A pattern in the one run of tokens a small edit changed.
fn token_intent(sp: &Spelling, c: &SingleChange, sides: &[Side; 2]) -> Option<Intent> {
    match c.spans {
        [Some(o), Some(n)] => operator_swap(sp, c, sides, [o, n]).or_else(|| names_literal(c)),
        [None, Some(n)] => one_sided(sp, c, sides[1], 1, n),
        [Some(o), None] => one_sided(sp, c, sides[0], 0, o),
        [None, None] => None,
    }
}

/// `<` → `<=` moves a boundary; `==` → `!=` negates the condition it is.
fn operator_swap(
    sp: &Spelling,
    c: &SingleChange,
    sides: &[Side; 2],
    spans: [Span; 2],
) -> Option<Intent> {
    let pair = (c.old.as_str(), c.new.as_str());
    let boundary = BOUNDARY.contains(&pair);
    if !boundary && !NEGATED.contains(&pair) {
        return None;
    }
    let comparison = |k: usize| {
        let op = sides[k].node_at(c.rows[k], spans[k])?;
        let cmp = op.parent()?;
        (op.child_count() == 0 && sp.binary.contains(&cmp.kind())).then_some(cmp)
    };
    let (old, new) = (comparison(0)?, comparison(1)?);
    let texts = || (squeeze(sides[0].text(old)), squeeze(sides[1].text(new)));
    if boundary {
        let (o, n) = texts();
        return Some(Intent::Boundary {
            ops: [c.old.clone(), c.new.clone()],
            old: o,
            new: n,
        });
    }
    (is_condition(sp, old) && is_condition(sp, new)).then(|| {
        let (o, n) = texts();
        Intent::Inverts { old: o, new: n }
    })
}

/// `30` → `TIMEOUT`: a number or string literal replaced by a constant.
fn names_literal(c: &SingleChange) -> Option<Intent> {
    // the whole literal, quotes and all: a string's content changing to
    // something shouted is still a string
    let literal = match c.roles[0] {
        Some(Role::Num) => true,
        Some(Role::Str) => c.old.ends_with(['"', '\'', '`']),
        _ => false,
    };
    let named = !matches!(c.roles[1], Some(Role::Str | Role::Num));
    (literal && named && constant_name(&c.new)).then(|| Intent::NamesLiteral {
        literal: c.old.clone(),
        name: c.new.clone(),
    })
}

/// Spelled like constants but standing for no value of their own.
const NOT_CONSTANTS: &[&str] = &["NULL", "TRUE", "FALSE", "NIL"];

/// A name spelled the way constants are — `MAX_SIZE`, `kMaxSize` —
/// optionally qualified (`Config.TIMEOUT`, `std::f64::consts::PI`).
fn constant_name(t: &str) -> bool {
    let segments: Vec<&str> = t.split("::").flat_map(|s| s.split('.')).collect();
    let ident = |s: &str| {
        s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_alphanumeric() || c == '_')
    };
    let Some(last) = segments.last() else {
        return false;
    };
    let screaming = last.chars().count() >= 2
        && last.chars().any(char::is_uppercase)
        && !last.chars().any(char::is_lowercase);
    let k_prefixed = last
        .strip_prefix('k')
        .is_some_and(|r| r.starts_with(|c: char| c.is_uppercase()));
    segments.iter().all(|s| ident(s)) && (screaming || k_prefixed) && !NOT_CONSTANTS.contains(last)
}

/// A run only one side has: `async`, a negation, an `and`/`or` operand.
/// `k` is that side (1 = the run was added).
fn one_sided(sp: &Spelling, c: &SingleChange, side: Side, k: usize, span: Span) -> Option<Intent> {
    let added = k == 1;
    let run = if added { &c.new } else { &c.old };
    let row = c.rows[k];
    let tok = side.node_at(row, span)?;
    if sp.async_kw == Some(run.as_str()) && tok.kind() == run {
        let holder = tok.parent()?;
        // rust spells it among the function's modifiers
        let func = if holder.kind() == "function_modifiers" {
            holder.parent()?
        } else {
            holder
        };
        return is_function(func.kind()).then(|| Intent::Async {
            added,
            func: function_name(side, func),
        });
    }
    if sp.not.contains(&run.as_str()) {
        let holder = tok.parent()?;
        if !sp.negations.contains(&holder.kind()) {
            return None;
        }
        // a compound operator (`is not`) negates the comparison it sits in
        let negated = if holder.is_named() {
            holder
        } else {
            holder.parent()?
        };
        if !is_condition(sp, negated) {
            return None;
        }
        let this = squeeze(side.text(negated));
        let other = squeeze(&without(side, negated, tok.byte_range()));
        let (old, new) = if added { (other, this) } else { (this, other) };
        return Some(Intent::Inverts { old, new });
    }
    operand(sp, run, side, row, span, added)
}

/// `if a:` → `if a and b:` narrows the condition, `or b` widens it, and
/// dropping either does the reverse. The run must be the operator and its one
/// operand, and nothing between it and the condition may negate it.
fn operand(
    sp: &Spelling,
    run: &str,
    side: Side,
    row: usize,
    (a, b): Span,
    added: bool,
) -> Option<Intent> {
    let bool_op = |n: &Node| {
        n.child_count() == 0 && (sp.and.contains(&n.kind()) || sp.or.contains(&n.kind()))
    };
    let first = side.node_at(row, (a, a + 1)).filter(bool_op);
    let last = side.node_at(row, (b.saturating_sub(1), b)).filter(bool_op);
    let (op, leading) = match (first, last) {
        (Some(f), _) => (f, true),
        (None, Some(l)) => (l, false),
        (None, None) => return None,
    };
    let expr = op.parent()?;
    if !sp.binary.contains(&expr.kind()) || expr.child_by_field_name("operator")?.id() != op.id() {
        return None;
    }
    let operand = expr.child_by_field_name(if leading { "right" } else { "left" })?;
    let covered = if leading {
        op.start_byte()..operand.end_byte()
    } else {
        operand.start_byte()..op.end_byte()
    };
    if squeeze(&side.src[covered]) != squeeze(run) {
        return None;
    }
    let mut at = expr;
    while !is_condition(sp, at) {
        let up = at.parent()?;
        let and_or = sp.binary.contains(&up.kind())
            && up
                .child_by_field_name("operator")
                .is_some_and(|o| bool_op(&o));
        if !and_or && !sp.parens.contains(&up.kind()) {
            return None;
        }
        at = up;
    }
    Some(Intent::Condition {
        narrows: sp.and.contains(&op.kind()) == added,
        added,
        clause: squeeze(side.text(operand)),
    })
}

/// Is `n` the whole condition of an `if`, a loop or a ternary, seen through
/// parentheses?
fn is_condition(sp: &Spelling, n: Node) -> bool {
    let mut at = n;
    while let Some(p) = at.parent() {
        if p.child_by_field_name("condition")
            .is_some_and(|c| c.id() == at.id())
        {
            return true;
        }
        if !sp.parens.contains(&p.kind()) {
            return false;
        }
        at = p;
    }
    false
}

/// The name a function is declared under, or the variable, field or key an
/// anonymous one is bound to; `None` for a callback passed as it is.
fn function_name(side: Side, f: Node) -> Option<String> {
    let own = ["name", "declarator"]
        .iter()
        .find_map(|k| f.child_by_field_name(k));
    let bound = || {
        let p = f.parent()?;
        ["name", "key", "left"]
            .iter()
            .find_map(|k| p.child_by_field_name(k))
            .filter(|n| n.id() != f.id())
    };
    let name = side.text(own.or_else(bound)?);
    name.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | ':' | '.' | '~'))
        .then(|| name.to_string())
}

fn is_function(kind: &str) -> bool {
    ["function", "method", "arrow", "lambda"]
        .iter()
        .any(|f| kind.contains(f))
}

/// A pattern in a hunk's whole shape: what it inserted, or what it wrapped.
fn hunk_intent(sp: &Spelling, h: &RawHunk, sides: &[Side; 2]) -> Option<Intent> {
    let new = rows(h.new_range)?;
    match rows(h.old_range) {
        None => guard(sp, sides[1], new).or_else(|| default_param(sp, sides, None, new)),
        Some(old) => wraps_try(sp, sides, old, new)
            .or_else(|| default_param(sp, sides, Some(old), new))
            .or_else(|| replaced_by_call(sp, sides, old, new)),
    }
}

/// A hunk side's 1-based line range as 0-based rows; `None` when it is empty.
fn rows([a, b]: [usize; 2]) -> Option<(usize, usize)> {
    (a >= 1 && b >= a).then(|| (a - 1, b - 1))
}

/// An inserted `if` with no `else`, testing a value it does not bind, whose
/// one statement returns, raises, leaves the loop or jumps forward; or a
/// binding whose `else` does one of those.
fn guard(sp: &Spelling, side: Side, new: (usize, usize)) -> Option<Intent> {
    if new.1 - new.0 >= GUARD_LINES {
        return None;
    }
    let [stmt] = statements_in(sp, side.root, new)?[..] else {
        return None;
    };
    let (cond, leaves) = if sp.let_elses.contains(&stmt.kind()) {
        let value = squeeze(side.text(stmt.child_by_field_name("value")?));
        let pattern = squeeze(side.text(stmt.child_by_field_name("pattern")?));
        (
            format!("{value} is not {pattern}"),
            stmt.child_by_field_name("alternative")?,
        )
    } else {
        let plain = ["alternative", "initializer"]
            .iter()
            .all(|f| stmt.child_by_field_name(f).is_none());
        if !sp.ifs.contains(&stmt.kind()) || !plain {
            return None;
        }
        let cond = stmt.child_by_field_name("condition")?;
        if sp.binding_conds.contains(&cond.kind()) {
            return None;
        }
        (
            squeeze(side.text(unparen(sp, cond))),
            stmt.child_by_field_name("consequence")?,
        )
    };
    let [exit] = statements(sp, leaves)[..] else {
        return None;
    };
    Some(Intent::Guard {
        cond,
        exit: exit_of(sp, side, exit)?,
    })
}

/// How statement `n` leaves, if it does: a goto only when its label lies
/// further down the same function, where cleanup code sits — a jump back is
/// a loop.
fn exit_of(sp: &Spelling, side: Side, n: Node) -> Option<Exit> {
    if let Some((_, e)) = sp.exits.iter().find(|(k, _)| *k == n.kind()) {
        return Some(e.clone());
    }
    let (_, labelled) = sp.gotos.iter().find(|(k, _)| *k == n.kind())?;
    let label = side.text(n.child_by_field_name("label")?);
    let mut func = n.parent()?;
    while !is_function(func.kind()) {
        func = func.parent()?;
    }
    let mut stack = vec![func];
    while let Some(at) = stack.pop() {
        let lands = at.kind() == *labelled
            && at.start_byte() > n.end_byte()
            && at
                .child_by_field_name("label")
                .is_some_and(|l| side.text(l) == label);
        if lands {
            return Some(Exit::Goto(label.to_string()));
        }
        let mut cur = at.walk();
        stack.extend(at.named_children(&mut cur));
    }
    None
}

/// The new side is one try statement whose body is exactly the old lines,
/// however reindented.
fn wraps_try(
    sp: &Spelling,
    sides: &[Side; 2],
    old: (usize, usize),
    new: (usize, usize),
) -> Option<Intent> {
    let tries = sp.tries.as_ref()?;
    if old.1 - old.0 >= TRY_LINES {
        return None;
    }
    let new_side = sides[1];
    let [t] = spanned(new_side.root, new)?[..] else {
        return None;
    };
    if !tries.kinds.contains(&t.kind()) {
        return None;
    }
    let body = t.child_by_field_name("body")?;
    let (first, last) = ends(body)?;
    let wrapped = &new_side.src[first.start_byte()..last.end_byte()];
    if unspaced(wrapped) != unspaced(&sides[0].src[sides[0].rows(old)?]) {
        return None;
    }
    Some(try_intent(sp, tries, new_side, t, body, old.1 - old.0 + 1))
}

/// A try added around lines left as they were, not reindented: the diff
/// inserts its head and its handlers as two hunks, and the unchanged lines
/// between them are its body.
fn split_try(sp: &Spelling, sides: &[Side; 2], head: &RawHunk, tail: &RawHunk) -> Option<Intent> {
    let tries = sp.tries.as_ref()?;
    let inserted = |h: &RawHunk| {
        rows(h.old_range)
            .is_none()
            .then(|| rows(h.new_range))
            .flatten()
    };
    let ((a, b), (c, d)) = (inserted(head)?, inserted(tail)?);
    if c <= b + 1 || c - b - 1 > TRY_LINES {
        return None;
    }
    let new_side = sides[1];
    let [t] = spanned(new_side.root, (a, d))?[..] else {
        return None;
    };
    if !tries.kinds.contains(&t.kind()) {
        return None;
    }
    let body = t.child_by_field_name("body")?;
    let (first, last) = ends(body)?;
    if first.start_position().row != b + 1 || last_row(last) != c - 1 {
        return None;
    }
    Some(try_intent(sp, tries, new_side, t, body, c - b - 1))
}

/// The first and last named nodes in a try's body.
fn ends(body: Node) -> Option<(Node, Node)> {
    let mut cur = body.walk();
    let inner: Vec<Node> = body.named_children(&mut cur).collect();
    Some((*inner.first()?, *inner.last()?))
}

/// What try statement `t`, wrapping `lines` existing lines in `body`, says:
/// the call the body makes when it is one statement, and what the handlers
/// catch.
fn try_intent(
    sp: &Spelling,
    tries: &Tries,
    new_side: Side,
    t: Node,
    body: Node,
    lines: usize,
) -> Intent {
    let call = match statements(sp, body)[..] {
        [one] => first_call(sp, new_side, one),
        _ => None,
    };
    let mut cur = t.walk();
    let handlers: Vec<Node> = t
        .named_children(&mut cur)
        .filter(|n| tries.handlers.contains(&n.kind()))
        .collect();
    let caught: Vec<String> = handlers
        .iter()
        .filter_map(|h| caught_type(tries, *h))
        .map(|n| {
            let t = squeeze(new_side.text(n));
            t.strip_prefix('(')
                .and_then(|r| r.strip_suffix(')'))
                .map_or(t.clone(), str::to_string)
        })
        .collect();
    Intent::WrapsTry {
        call,
        lines,
        caught: (!caught.is_empty()).then(|| caught.join(", ")),
        word: if handlers.is_empty() {
            "try/finally"
        } else {
            tries.word
        },
    }
}

/// The new side is one statement making a call the old lines did not make,
/// and the old side held a loop: work done in place, now done elsewhere.
/// Without the loop, several statements becoming one call is as often logging
/// or cleanup as it is an extraction.
fn replaced_by_call(
    sp: &Spelling,
    sides: &[Side; 2],
    old: (usize, usize),
    new: (usize, usize),
) -> Option<Intent> {
    let [stmt] = statements_in(sp, sides[1].root, new)?[..] else {
        return None;
    };
    let call = first_call(sp, sides[1], stmt)?;
    let gone = statements_in(sp, sides[0].root, old)?;
    let loops: Vec<&'static str> = gone
        .iter()
        .filter_map(|n| {
            sp.loops
                .iter()
                .find(|(k, _)| *k == n.kind())
                .map(|(_, w)| *w)
        })
        .collect();
    let callee = call.rsplit(['.', ':']).next().unwrap_or(&call);
    if loops.is_empty() || crate::extract::makes_call(&sides[0].src[sides[0].rows(old)?], callee) {
        return None;
    }
    Some(Intent::ReplacedByCall {
        call,
        construct: match loops[..] {
            [one] => Some(one),
            _ => None,
        },
        lines: old.1 - old.0 + 1,
    })
}

/// What the first call statement `n` makes itself calls, as written:
/// everything in front of its arguments (`self.client.get`, `ns::f`). Not a
/// call in a statement or a function nested in it: wrapping `if (a) b();` is
/// not wrapping `b()`.
fn first_call(sp: &Spelling, side: Side, n: Node) -> Option<String> {
    let nested = |k: &str| {
        sp.blocks.contains(&k)
            || sp.ifs.contains(&k)
            || is_function(k)
            || k.ends_with("statement")
            || k.ends_with("clause")
    };
    let mut stack = vec![n];
    while let Some(at) = stack.pop() {
        let is_call = at.kind().contains("call") || at.kind() == "method_invocation";
        if let Some(args) = at.child_by_field_name("arguments").filter(|_| is_call) {
            return Some(squeeze(&side.src[at.start_byte()..args.start_byte()]));
        }
        let mut cur = at.walk();
        let kids: Vec<Node> = at
            .named_children(&mut cur)
            .filter(|k| !nested(k.kind()))
            .collect();
        stack.extend(kids.into_iter().rev());
    }
    None
}

/// What a handler catches, by the spelling's steps; a handler that binds the
/// exception (`as e`) names the type in front of the binding.
fn caught_type<'t>(tries: &Tries, handler: Node<'t>) -> Option<Node<'t>> {
    if tries.caught.is_empty() {
        return None;
    }
    let mut at = handler;
    for step in tries.caught {
        at = child(at, step)?;
    }
    if at.child_by_field_name("alias").is_some() {
        at = at.named_child(0)?;
    }
    Some(at)
}

/// A child by field name or, failing that, the first one of that kind.
fn child<'t>(n: Node<'t>, want: &str) -> Option<Node<'t>> {
    n.child_by_field_name(want).or_else(|| {
        let mut cur = n.walk();
        let found = n.named_children(&mut cur).find(|c| c.kind() == want);
        found
    })
}

/// The hunk adds one defaulted parameter as its function's last (or ahead of
/// only a `**kwargs`) and changes nothing else, so every existing call still
/// binds as before.
fn default_param(
    sp: &Spelling,
    sides: &[Side; 2],
    old: Option<(usize, usize)>,
    new: (usize, usize),
) -> Option<Intent> {
    let short = |(a, b): (usize, usize)| b - a < PARAM_LINES;
    if sp.defaulted.is_empty() || !short(new) || !old.is_none_or(short) {
        return None;
    }
    let new_side = sides[1];
    let lists = overlapping(new_side.root, new, sp.params);
    if lists.is_empty() {
        return None;
    }
    let hunk = new_side.rows(new)?;
    let was = match old {
        Some(o) => unspaced(&sides[0].src[sides[0].rows(o)?]),
        None => String::new(),
    };
    for list in lists {
        let mut cur = list.walk();
        let params: Vec<Node> = list
            .named_children(&mut cur)
            .filter(|p| !p.kind().contains("comment"))
            .collect();
        for (i, p) in params.iter().enumerate() {
            let defaulted = sp
                .defaulted
                .iter()
                .any(|d| p.kind() == *d || p.child_by_field_name(d).is_some());
            let inside = hunk.start <= p.start_byte() && p.end_byte() <= hunk.end;
            let after = &params[i + 1..];
            let last = after.is_empty()
                || matches!(after, [only] if sp.keyword_rest.contains(&only.kind()));
            if !(defaulted && inside && last) {
                continue;
            }
            // the hunk's text without the parameter and the comma that
            // separated it is the old text
            let text = &new_side.src[hunk.clone()];
            let (at, end) = (p.start_byte() - hunk.start, p.end_byte() - hunk.start);
            let (mut before, mut after) = (text[..at].trim_end(), text[end..].trim_start());
            match (before.strip_suffix(','), after.strip_prefix(',')) {
                (Some(b), _) => before = b,
                (None, Some(a)) => after = a,
                (None, None) => {}
            }
            if unspaced(&format!("{before}{after}")) == was {
                let name = ["name", "pattern", "left", "declarator"]
                    .iter()
                    .find_map(|f| p.child_by_field_name(f))
                    .unwrap_or(*p);
                return Some(Intent::DefaultParam {
                    param: squeeze(new_side.text(name)),
                    func: list.parent().and_then(|f| function_name(new_side, f)),
                });
            }
        }
    }
    None
}

/// The nodes of one of `kinds` overlapping rows `r0..=r1`.
fn overlapping<'t>(root: Node<'t>, (r0, r1): (usize, usize), kinds: &[&str]) -> Vec<Node<'t>> {
    let mut out = vec![];
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if last_row(n) < r0 || n.start_position().row > r1 {
            continue;
        }
        if kinds.contains(&n.kind()) {
            out.push(n);
        }
        let mut cur = n.walk();
        stack.extend(n.named_children(&mut cur));
    }
    out
}

/// `outer`'s text with `cut` taken out of it.
fn without(side: Side, outer: Node, cut: std::ops::Range<usize>) -> String {
    let r = outer.byte_range();
    format!(
        "{}{}",
        &side.src[r.start..cut.start],
        &side.src[cut.end..r.end]
    )
}

/// The statements lying wholly on rows `r0..=r1`, seen through blocks.
fn statements_in<'t>(sp: &Spelling, root: Node<'t>, rows: (usize, usize)) -> Option<Vec<Node<'t>>> {
    Some(
        spanned(root, rows)?
            .into_iter()
            .flat_map(|n| statements(sp, n))
            .collect(),
    )
}

/// The statements `n` holds, seen through blocks and expression statements;
/// `n` itself when it is not one of those.
fn statements<'t>(sp: &Spelling, n: Node<'t>) -> Vec<Node<'t>> {
    if !sp.blocks.contains(&n.kind()) {
        return vec![n];
    }
    let mut cur = n.walk();
    let kids: Vec<Node> = n
        .named_children(&mut cur)
        .filter(|c| !c.kind().contains("comment"))
        .collect();
    kids.into_iter().flat_map(|c| statements(sp, c)).collect()
}

/// The outermost named nodes lying wholly on rows `r0..=r1`, comments aside.
/// `None` when a syntax error, or a token of something larger, lies there
/// too: the rows are then not a run of whole statements.
fn spanned<'t>(root: Node<'t>, (r0, r1): (usize, usize)) -> Option<Vec<Node<'t>>> {
    let mut out = vec![];
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let (a, b) = (n.start_position().row, last_row(n));
        if b < r0 || a > r1 {
            continue;
        }
        if r0 <= a && b <= r1 && n.id() != root.id() {
            if !n.is_named() || n.has_error() {
                return None;
            }
            if !n.kind().contains("comment") {
                out.push(n);
            }
            continue;
        }
        let mut cur = n.walk();
        stack.extend(n.children(&mut cur));
    }
    out.sort_by_key(|n| n.start_byte());
    Some(out)
}

/// The last row `n` has text on: a node ending at column 0 ended with the
/// line break before it.
fn last_row(n: Node) -> usize {
    let e = n.end_position();
    if e.column == 0 && e.row > n.start_position().row {
        e.row - 1
    } else {
        e.row
    }
}

/// `n` without the parentheses (or c++ condition clause) around it.
fn unparen<'t>(sp: &Spelling, mut n: Node<'t>) -> Node<'t> {
    while sp.parens.contains(&n.kind()) {
        let inner = n.child_by_field_name("value").or_else(|| {
            (n.named_child_count() == 1)
                .then(|| n.named_child(0))
                .flatten()
        });
        match inner {
            Some(i) => n = i,
            None => break,
        }
    }
    n
}

/// Every whitespace character removed: two texts equal under this differ
/// only in layout.
fn unspaced(t: &str) -> String {
    t.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_loop_kind_is_one_its_grammar_has() {
        for sp in SPELLINGS {
            for name in sp.langs {
                let spec = lang::all().iter().find(|s| s.name == *name).unwrap();
                let language = (spec.language)();
                for (kind, _) in sp.loops {
                    assert_ne!(language.id_for_node_kind(kind, true), 0, "{name}: {kind}");
                }
            }
        }
    }
}
