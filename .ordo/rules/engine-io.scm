; The engine is pure: no filesystem, no process, no environment. A client
; (ordo-tui) owns git and config; the engine takes Input and returns Output.
; Breaking this makes `ordo order --json` stop being a function of its
; arguments, and makes the corpus suite stop meaning anything.
((scoped_identifier path: (identifier) @root) (#any-of? @root "fs" "env"))
((scoped_identifier path: (scoped_identifier) @p) (#match? @p "std::(fs|env|process)"))
((call_expression function: (scoped_identifier name: (identifier) @f))
 (#any-of? @f "read_to_string" "write" "create_dir_all" "var" "var_os"))
