; A panic in the engine takes down every consumer — an editor, a CI bot, a
; review server — for one malformed input. The engine's own style is to degrade
; (Option/None, a flagged `degraded` file) rather than abort.
((call_expression function: (field_expression field: (field_identifier) @m))
 (#any-of? @m "unwrap" "expect"))
((macro_invocation macro: (identifier) @m)
 (#any-of? @m "panic" "todo" "unimplemented" "unreachable"))
