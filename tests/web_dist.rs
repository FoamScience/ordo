//! `web/dist/` is committed so building ordo never needs node, which means it
//! can fall behind `web/src/`. `npm run build` stamps the bundle with a hash of
//! the sources it was built from; this recomputes it (the same FNV-1a 64 as
//! `web/hash.mjs`) and fails while the two disagree.
use std::path::{Path, PathBuf};

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).expect("web/src").flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn source_hash(dir: &Path) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    let mut rel: Vec<String> = files(dir)
        .iter()
        .map(|p| {
            p.strip_prefix(dir)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    rel.sort();
    for r in rel {
        feed(r.as_bytes());
        feed(&[0]);
        feed(&std::fs::read(dir.join(&r)).expect("source file"));
        feed(&[0]);
    }
    format!("{h:016x}")
}

#[test]
fn the_committed_web_bundle_was_built_from_the_sources_beside_it() {
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
    let stamped =
        std::fs::read_to_string(web.join("dist/source.hash")).expect("web/dist/source.hash");
    assert_eq!(
        stamped.trim(),
        source_hash(&web.join("src")),
        "web/dist is stale: run `npm run build` in web/"
    );
}
