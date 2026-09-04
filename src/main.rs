//! CLI: `ordo-engine order --json < input.json > output.json`.
//! (`ordo-engine review <patch>` is planned — P3/P7.)
use std::io::Read;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("order");
    match cmd {
        "order" => order(&args[2..]),
        "pack" => pack(&args[2..]),
        "review" => review(&args[2..]),
        "-V" | "--version" | "version" => {
            println!(
                "ordo-engine {} (schema {})",
                env!("CARGO_PKG_VERSION"),
                ordo::SCHEMA_VERSION
            );
        }
        "-h" | "--help" | "help" => {
            eprintln!("usage:\n  ordo-engine order [--only-comments] --json < input.json > output.json\n  ordo-engine pack  [--only-comments] --json < input.json  # compact LLM-ready review context\n  ordo-engine review [--full-context] [patch]   # patch from arg or stdin\n  ordo --version\n\n--full-context: the patch is a complete diff (git diff -U100000), so modified\n                files get full semantics instead of positional order.\n--only-comments: drop every non-comment/docstring hunk before ordering, so\n                 order/groups/edges/clusters cover only comment changes.");
        }
        other => {
            eprintln!("ordo-engine: unknown command '{other}'\nusage: ordo-engine order --json < input.json | ordo-engine review [--full-context] [patch] | ordo --version");
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
    let mut path: Option<&str> = None;
    for a in args {
        if a == "--full-context" {
            full_context = true;
        } else if !a.starts_with('-') {
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
    };
    emit(ordo::run(input));
}

fn read_stdin() -> String {
    let mut buf = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
        eprintln!("ordo-engine: failed to read stdin: {e}");
        exit(1);
    }
    buf
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
    emit(ordo::run(read_input(
        args.iter().any(|a| a == "--only-comments"),
    )));
}

/// `ordo-engine pack --json < input.json` — compact, LLM-ready review context.
fn pack(args: &[String]) {
    let input = read_input(args.iter().any(|a| a == "--only-comments"));
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
