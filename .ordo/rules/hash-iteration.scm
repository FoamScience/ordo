; Output built by walking a HashMap is non-deterministic: same input, different
; order. Everything that reaches `order`, `groups`, `edges` or `clusters` must
; be sorted (or come from a BTreeMap) before it leaves.
((call_expression function: (field_expression field: (field_identifier) @m))
 (#any-of? @m "values" "into_values" "keys" "iter"))
