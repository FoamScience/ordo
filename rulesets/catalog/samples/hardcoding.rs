// Exercises the rust half of rulesets/catalog/hardcoding.toml.
fn expired(age: u64) -> bool {
    age > 86400 // magic-number
}

fn main() {
    let _ = expired(1);
}
