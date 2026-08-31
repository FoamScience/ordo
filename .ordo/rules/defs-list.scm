; A language's node-kind lists ARE the symbol identity: `defines`, `symbols`,
; and the persisted review-mark key all derive from them. Changing one silently
; invalidates marks a reviewer already placed.
((field_initializer field: (field_identifier) @f)
 (#any-of? @f "defs" "members" "imports" "locals" "test_blocks"))
