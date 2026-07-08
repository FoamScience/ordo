//! CLI: `ordo order --json < input.json > output.json`.
//! (`ordo review <patch>` is planned — P3/P7.)
use std::io::Read;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("order");
    match cmd {
        "order" => order(),
        "review" => review(&args[2..]),
        "-V" | "--version" | "version" => {
            println!(
                "ordo {} (schema {})",
                env!("CARGO_PKG_VERSION"),
                ordo::SCHEMA_VERSION
            );
        }
        "-h" | "--help" | "help" => {
            eprintln!("usage:\n  ordo order --json < input.json > output.json\n  ordo review [--full-context] [patch]   # patch from arg or stdin\n  ordo --version\n\n--full-context: the patch is a complete diff (git diff -U100000), so modified\n                files get full semantics instead of positional order.");
        }
        other => {
            eprintln!("ordo: unknown command '{other}'\nusage: ordo order --json < input.json | ordo review [--full-context] [patch] | ordo --version");
            exit(2);
        }
    }
}

/// `ordo review [--full-context] [patch]` — read a git/unified diff (file arg or
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
            eprintln!("ordo: cannot read '{f}': {e}");
            exit(1);
        }),
        None => {
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("ordo: failed to read stdin: {e}");
                exit(1);
            }
            buf
        }
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

fn order() {
    let mut buf = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
        eprintln!("ordo: failed to read stdin: {e}");
        exit(1);
    }
    let input: ordo::model::Input = match serde_json::from_str(&buf) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ordo: invalid input json: {e}");
            exit(1);
        }
    };
    emit(ordo::run(input));
}

fn emit(out: ordo::model::Output) {
    match serde_json::to_string_pretty(&out) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("ordo: failed to serialize output: {e}");
            exit(1);
        }
    }
}
