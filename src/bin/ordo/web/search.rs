//! The palette finder: the review's hunks ranked by Okapi BM25 against what
//! the reader typed. One index per loaded review, built when it lands.

use crate::*;
use bm25::{Document, SearchEngine, SearchEngineBuilder, Tokenizer};

/// How many hunks one query answers with.
pub(super) const TOP: usize = 20;

/// Lower-case word pieces of identifiers: split on anything not alphanumeric
/// and on camelCase humps, dropping single characters and bare numbers.
pub(super) fn words(text: &str) -> Vec<String> {
    let mut out = vec![];
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = word.chars().collect();
        let mut start = 0;
        for i in 1..=chars.len() {
            let cut = i == chars.len()
                || (chars[i].is_uppercase()
                    && (chars[i - 1].is_lowercase()
                        || chars.get(i + 1).is_some_and(|c| c.is_lowercase())
                            && chars[i - 1].is_uppercase()));
            if cut {
                let piece: String = chars[start..i].iter().collect::<String>().to_lowercase();
                if piece.chars().count() > 1 && !piece.chars().all(|c| c.is_ascii_digit()) {
                    out.push(piece);
                }
                start = i;
            }
        }
    }
    out
}

#[derive(Default)]
struct Words;

impl Tokenizer for Words {
    fn tokenize(&self, text: &str) -> Vec<String> {
        words(text)
    }
}

pub(super) struct Index(SearchEngine<usize, u32, Words>);

impl Index {
    /// One document per hunk, by its index in the review.
    pub(super) fn new(docs: impl IntoIterator<Item = String>) -> Index {
        let docs: Vec<Document<usize>> = docs
            .into_iter()
            .enumerate()
            .map(|(i, d)| Document::new(i, d))
            .collect();
        Index(
            SearchEngineBuilder::<usize, u32, Words>::with_tokenizer_and_documents(Words, docs)
                .k1(1.2)
                .b(0.75)
                .build(),
        )
    }

    pub(super) fn of(app: &App) -> Index {
        Index::new(app.items.iter().map(|it| hunk_text(it, &app.sources)))
    }

    /// The best `limit` hunks for `query` with their scores, best first and
    /// equal scores by index; a hunk sharing no word with it is left out.
    pub(super) fn search(&self, query: &str, limit: usize) -> Vec<(usize, f32)> {
        // each query word counts once, however often it was typed
        let mut seen = HashSet::new();
        let terms: Vec<String> = words(query)
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .collect();
        let mut hits: Vec<(usize, f32)> = self
            .0
            .search(&terms.join(" "), None)
            .into_iter()
            .map(|r| (r.document.id, r.score))
            .filter(|&(_, score)| score > 0.0)
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        hits.truncate(limit);
        hits
    }
}

/// What a hunk is found by: its path, the definition it sits in, the names it
/// defines, and the lines it changed on both sides.
fn hunk_text(it: &Item, sources: &Sources) -> String {
    let mut parts: Vec<&str> = vec![&it.path];
    parts.extend(it.enclosing.as_deref());
    parts.extend(it.symbols.iter().map(|s| s.name.as_str()));
    if let Some((old, new)) = sources.get(&it.path) {
        for (lines, [a, b]) in [(old, it.old_range), (new, it.new_range)] {
            if a >= 1 && b >= a {
                let changed = lines.get(a - 1..b.min(lines.len())).unwrap_or_default();
                parts.extend(changed.iter().map(String::as_str));
            }
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_identifiers_into_lower_case_pieces() {
        assert_eq!(
            words("getBladeWire HTTPServer parse_json_v2"),
            ["get", "blade", "wire", "http", "server", "parse", "json", "v2"]
        );
        assert_eq!(words("x = a + 42 * 3.5;"), Vec::<String>::new());
        assert_eq!(words("ppump/geom.py:212"), ["ppump", "geom", "py"]);
        assert_eq!(words("XMLHttpRequest2"), ["xml", "http", "request2"]);
    }

    #[test]
    fn hits_rank_by_score_then_index_and_misses_are_dropped() {
        let index = Index::new(
            [
                "blade wire",
                "unrelated text here",
                "blade wire",
                "blade blade wire vertex",
                "get_blade_wire_between_vertexes",
            ]
            .map(String::from),
        );
        let hits = index.search("bladeWire", TOP);
        let order: Vec<usize> = hits.iter().map(|&(i, _)| i).collect();
        assert!(!order.contains(&1), "{hits:?}");
        assert_eq!(order.len(), 4);
        // two equal documents score equal, and the earlier hunk goes first
        let k0 = order.iter().position(|&i| i == 0).unwrap();
        assert_eq!(order[k0 + 1], 2);
        assert_eq!(hits[k0].1, hits[k0 + 1].1);
        assert!(hits.windows(2).all(|w| w[0].1 >= w[1].1));
        // the same query answers the same way every time
        for _ in 0..5 {
            assert_eq!(index.search("bladeWire", TOP), hits);
        }
        assert_eq!(index.search("blade", 2).len(), 2);
        assert_eq!(
            index.search("blade blade BLADE wire", TOP),
            index.search("blade wire", TOP)
        );
        assert!(index.search("", TOP).is_empty());
        assert!(index.search("nothing matches", TOP).is_empty());
    }
}
