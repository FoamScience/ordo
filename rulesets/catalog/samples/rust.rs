// Exercises every rule in rulesets/catalog/rust.toml.
static mut COUNTER: u32 = 0; // static-mut

fn bits(x: i32) -> u32 {
    unsafe {
        // unsafe
        std::mem::transmute(x) // transmute
    }
}

fn main() {
    let _ = bits(1);
}
