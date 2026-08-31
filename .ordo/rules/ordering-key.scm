; This is where P2 lives. The topological key decides what "definitions before
; uses" means; a change here needs the permutation and def-before-use tests, not
; a spot check.
((call_expression function: (field_expression field: (field_identifier) @m))
 (#any-of? @m "sort_by_key" "min_by_key" "sort_unstable_by_key"))
