; A snake_case string literal in these files is almost always a tree-sitter node
; kind — the one class of claim this project checks against the grammar's own
; node-types.json rather than trusting memory.
((string_literal (string_content) @s) (#match? @s "^[a-z]+(_[a-z]+)+$"))
