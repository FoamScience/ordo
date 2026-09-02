; os.path.<fn>(...) — pathlib.Path reads better and keeps path semantics.
; Predicates must sit INSIDE the pattern: a sibling `(#eq? ...)` parses fine
; and silently matches everything.
((call
   function: (attribute
     object: (attribute object: (identifier) @mod attribute: (identifier) @sub)
     attribute: (identifier) @fn)) @call
 (#eq? @mod "os")
 (#eq? @sub "path"))
