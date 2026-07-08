//! CLI: `ordo order --json < input.json > output.json`.
//! (`ordo review <patch>` is planned — P3/P7.)
use std::io::Read;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("order");
    match cmd {
        "order" => order(),
        "review" => review(args.get(2).map(String::as_str)),
        "-V" | "--version" | "version" => {
            println!(
                "ordo {} (schema {})",
                env!("CARGO_PKG_VERSION"),
                ordo::SCHEMA_VERSION
            );
        }
        "-h" | "--help" | "help" => {
            eprintln!("usage:\n  ordo order --json < input.json > output.json\n  ordo review [patch]   # patch from arg or stdin\n  ordo --version");
        }
        other => {
            eprintln!("ordo: unknown command '{other}'\nusage: ordo order --json < input.json | ordo review [patch] | ordo --version");
            exit(2);
        }
    }
}

/// `ordo review [patch]` — read a git/unified diff (file arg or stdin), split
/// per file, and print the engine's JSON ordering. Modified-file hunks are
/// positional (see patch.rs ceiling); additions get full semantics.
fn review(path: Option<&str>) {
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
        options: Default::default(),
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
