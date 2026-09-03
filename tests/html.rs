//! html, and with it vue: only an element carrying an `id` is a definition —
//! the one handle a stylesheet, a script or a fragment link addresses it by.
//! A `.vue` single-file component needs no grammar of its own.
use ordo::model::{HunkOut, Input};

fn one(path: &str, old: &str, new: &str) -> Vec<HunkOut> {
    ordo::run(
        serde_json::from_value::<Input>(serde_json::json!({
            "changes": [{ "path": path, "old": old, "new": new }]
        }))
        .unwrap(),
    )
    .files
    .into_iter()
    .flat_map(|f| f.hunks)
    .collect()
}

#[test]
fn an_element_with_an_id_is_a_definition() {
    let old = "<body>\n  <section id=\"main\">\n    <h1>Hi</h1>\n  </section>\n</body>\n";
    let new = "<body>\n  <section id=\"main\">\n    <h1>Hi</h1>\n  </section>\n  \
               <footer id=\"foot\">\n    <p>x</p>\n  </footer>\n</body>\n";
    let hs = one("page.html", old, new);
    assert!(
        hs.iter().any(|h| h.defines.contains(&"#foot".to_string())),
        "{hs:?}"
    );
}

#[test]
fn an_element_without_an_id_is_transparent() {
    // a page of anonymous divs contributes no definitions; the hunk still
    // attributes to the nearest element that *does* have one
    let hs = one(
        "page.html",
        "<section id=\"main\">\n  <div><span>a</span></div>\n</section>\n",
        "<section id=\"main\">\n  <div><span>b</span></div>\n</section>\n",
    );
    assert!(hs.iter().all(|h| h.defines.is_empty()), "{hs:?}");
    assert!(
        hs.iter().any(|h| h.enclosing.as_deref() == Some("#main")),
        "{hs:?}"
    );
}

#[test]
fn a_class_is_not_a_use_of_the_css_that_styles_it() {
    // deliberate: `class="btn"` is not tokenized and `btn` would collide with
    // real code symbols — see docs/document-languages-design.md
    let hs = one(
        "page.html",
        "<p class=\"btn card\">a</p>\n",
        "<p class=\"btn card\">b</p>\n",
    );
    assert!(hs.iter().all(|h| h.uses.is_empty()), "{hs:?}");
}

#[test]
fn a_vue_component_needs_no_grammar_of_its_own() {
    // `<script setup lang="ts">`, `@click`, a self-closing component and
    // `{{ }}` all parse with no error nodes
    let old = "<template>\n  <div id=\"card\">\n    <p>{{ msg }}</p>\n  </div>\n</template>\n\n\
               <script setup lang=\"ts\">\nconst msg = 'hi'\n</script>\n";
    let new =
        "<template>\n  <div id=\"card\">\n    <p>{{ msg }}</p>\n    <MyBtn @click=\"pick\" />\n  \
               </div>\n</template>\n\n<script setup lang=\"ts\">\nconst msg = 'hey'\n</script>\n";
    let hs = one("Card.vue", old, new);
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "Card.vue", "old": old, "new": new }]
    }))
    .unwrap();
    assert!(!ordo::run(inp).files[0].unsupported);
    assert!(
        hs.iter().any(|h| h.enclosing.as_deref() == Some("#card")),
        "{hs:?}"
    );
}

#[test]
fn svelte_needs_its_own_grammar_but_reuses_the_shape() {
    // `{#if n > 1}` and `on:click={() => pick()}` each hold a bare `>`, which
    // the html grammar cannot read — so svelte gets its own. The element and
    // attribute kinds are identical, so the id-naming path is unchanged.
    let old = "<div id=\"root\">\n{#if n > 1}\n  <button on:click={() => pick()}>go</button>\n{/if}\n</div>\n";
    let new = "<div id=\"root\">\n{#if n > 2}\n  <button on:click={() => pick()}>stop</button>\n{/if}\n</div>\n";
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "App.svelte", "old": old, "new": new }]
    }))
    .unwrap();
    let out = ordo::run(inp);
    assert!(!out.files[0].unsupported);
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    assert!(
        hs.iter().any(|h| h.enclosing.as_deref() == Some("#root")),
        "{hs:?}"
    );
}
