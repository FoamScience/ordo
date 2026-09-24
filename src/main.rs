//! CLI: `ordo-engine order --json < input.json > output.json`.
use std::io::Read;
use std::process::exit;

const USAGE: &str = "usage:
  ordo-engine order  [--only-comments] [--sarif] --json < input.json > output.json
  ordo-engine pack   [--only-comments] --json < input.json  # compact LLM-ready review context
  ordo-engine review [--full-context] [--sarif] [patch]     # patch from arg or stdin
  ordo-engine --version

--full-context: the patch is a complete diff (git diff -U100000), so modified
                files get full semantics instead of positional order.
--only-comments: drop every non-comment/docstring hunk before ordering, so
                 order/groups/edges/clusters cover only comment changes.
--sarif: print the findings as SARIF 2.1.0 instead of the engine's own JSON,
         for whatever already reads analyzer output. Findings only — SARIF has
         no vocabulary for the reading order or the def→use graph.";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rest = args.get(2..).unwrap_or(&[]);
    let cmd = args.get(1).map(String::as_str).unwrap_or("order");
    match cmd {
        "order" => order(rest),
        "pack" => pack(rest),
        "review" => review(rest),
        "-V" | "--version" | "version" => {
            println!(
                "ordo-engine {} (schema {})",
                env!("CARGO_PKG_VERSION"),
                ordo::SCHEMA_VERSION
            );
        }
        "-h" | "--help" | "help" => println!("{USAGE}"),
        other => {
            eprintln!("ordo-engine: unknown command '{other}'\n{USAGE}");
            exit(2);
        }
    }
}

/// `ordo-engine review [--full-context] [patch]` — read a git/unified diff (file arg or
/// stdin), split per file, print the engine's JSON ordering. Modified-file hunks
/// are positional unless `--full-context` asserts a complete patch (see
/// docs/diff-input-design.md); additions get full semantics either way.
fn review(args: &[String]) {
    let mut full_context = false;
    let mut as_sarif = false;
    let mut path: Option<&str> = None;
    for a in args {
        if a == "--full-context" {
            full_context = true;
        } else if a == "--sarif" {
            as_sarif = true;
        } else if a.starts_with('-') {
            eprintln!("ordo-engine review: unknown flag '{a}'\n{USAGE}");
            exit(2);
        } else {
            path = Some(a);
        }
    }
    let src = match path {
        Some(f) => std::fs::read_to_string(f).unwrap_or_else(|e| {
            eprintln!("ordo-engine: cannot read '{f}': {e}");
            exit(1);
        }),
        None => read_stdin(),
    };
    let changes = ordo::split_patch(&src)
        .into_iter()
        .map(|(path, diff)| ordo::model::Change {
            path,
            old: None,
            new: None,
            diff: Some(diff),
        })
        .collect();
    let input = ordo::model::Input {
        changes,
        options: ordo::model::Options {
            full_context,
            ..Default::default()
        },
        consumers: vec![],
    };
    let out = ordo::run(input);
    if as_sarif {
        println!("{}", ordo::sarif(&out));
    } else {
        emit(out);
    }
}

fn read_stdin() -> String {
    let mut buf = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
        eprintln!("ordo-engine: failed to read stdin: {e}");
        exit(1);
    }
    buf
}

/// `--only-comments` is the only flag `order`/`pack` share; anything else
/// starting with `-` is a typo, and silently ordering the whole diff instead of
/// the comment subset the caller asked for is worse than refusing. `--sarif`
/// is `order`'s alone: `pack` renders review context, which SARIF has no
/// vocabulary for, so accepting and ignoring it would be the same lie.
fn only_comments_flag(args: &[String], sarif_ok: bool) -> bool {
    for a in args {
        // `--json` names the input format these two already require; it is
        // accepted so the documented invocation works, and means nothing.
        if a != "--only-comments" && a != "--json" && !(sarif_ok && a == "--sarif") {
            eprintln!("ordo-engine: unknown flag '{a}'\n{USAGE}");
            exit(2);
        }
    }
    args.iter().any(|a| a == "--only-comments")
}

fn read_input(only_comments: bool) -> ordo::model::Input {
    let buf = read_stdin();
    let mut input: ordo::model::Input = serde_json::from_str(&buf).unwrap_or_else(|e| {
        eprintln!("ordo-engine: invalid input json: {e}");
        exit(1);
    });
    if only_comments {
        input.options.only_comments = true;
    }
    input
}

fn order(args: &[String]) {
    let as_sarif = args.iter().any(|a| a == "--sarif");
    let input = read_input(only_comments_flag(args, true));
    let out = ordo::run(input);
    if as_sarif {
        println!("{}", ordo::sarif(&out));
    } else {
        emit(out);
    }
}

/// `ordo-engine pack --json < input.json` — compact, LLM-ready review context.
fn pack(args: &[String]) {
    let input = read_input(only_comments_flag(args, false));
    print!("{}", ordo::pack(&ordo::run(input)));
}

fn emit(out: ordo::model::Output) {
    match serde_json::to_string_pretty(&out) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("ordo-engine: failed to serialize output: {e}");
            exit(1);
        }
    }
}
