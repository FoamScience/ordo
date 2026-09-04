// Compiles the one grammar this crate vendors rather than takes as a
// dependency: Go templates (Helm), which no crate publishes for a current
// tree-sitter. See grammars/tree-sitter-go-template/README.md.
fn main() {
    let dir = std::path::Path::new("grammars/tree-sitter-go-template/src");
    println!("cargo:rerun-if-changed={}", dir.join("parser.c").display());
    cc::Build::new()
        .include(dir)
        .file(dir.join("parser.c"))
        // generated parsers trip these; upstream ships them the same way
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .compile("tree-sitter-go-template");
}
