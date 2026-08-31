; A `pub fn` in the engine is API for every consumer, forever — the crate has a
; frozen contract and third-party wrappers (npm, pypi). Private by default;
; public is a decision.
((function_item (visibility_modifier) @vis name: (identifier) @name) (#eq? @vis "pub"))
