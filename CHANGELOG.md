# Changelog

## [0.6.1](https://github.com/FoamScience/ordo/compare/v0.6.0...v0.6.1) (2026-11-11)


### Bug Fixes

* slide ambiguous insert/delete hunks onto syntax boundaries ([407cd96](https://github.com/FoamScience/ordo/commit/407cd9631f58e8b1142e55d9476dfed1e98d4d63))

## [0.6.0](https://github.com/FoamScience/ordo/compare/v0.5.0...v0.6.0) (2026-11-10)


### ⚠ BREAKING CHANGES

* drop the terminal UI; ordo <rev> serves the web reviewer and reads ordo.toml

### Features

* **census:** ordo-engine swept over 50 repositories' newest commits under time and memory caps ([28ea004](https://github.com/FoamScience/ordo/commit/28ea004e9ee943c914cc231b9ac92ed4efc6ee1d))
* drop the terminal UI; ordo &lt;rev&gt; serves the web reviewer and reads ordo.toml ([25a7a8b](https://github.com/FoamScience/ordo/commit/25a7a8b7ff96725fa4a39ccce750443e72ceb358))
* **engine:** a change-level summary labels the change and says whether it preserves behaviour, and an edit's intent leads its headline over the helper it uses ([79579ca](https://github.com/FoamScience/ordo/commit/79579cafcb0a34405eb2e38e1da5daa3f33a0ead))
* **engine:** a constant or config key given a new literal shows both values ([4d385d2](https://github.com/FoamScience/ordo/commit/4d385d29ecf4a6de42e962e765d600269d095f8d))
* **engine:** a definition added or deleted whole swallows the detail lines of what is inside it ([22f5ea3](https://github.com/FoamScience/ordo/commit/22f5ea307858e508aedd21b7e7f764f286f03af1))
* **engine:** a hunk touching only the file header's copyright notice is noise ([dce9c96](https://github.com/FoamScience/ordo/commit/dce9c96c25635c5eb36183f17676d7fe067d20b3))
* **engine:** a rust let-else and a forward goto to a cleanup label read as guard clauses ([efd2d44](https://github.com/FoamScience/ordo/commit/efd2d44c27d277f7885ee412814775f74125a135))
* **engine:** an inserted whole #if block inside a function or type names its conditional ([30f0fb3](https://github.com/FoamScience/ordo/commit/30f0fb33bcb5dee13df97c93d2c31da976835a56))
* **engine:** edit patterns read off both trees (guard, try wrap, narrowed condition, boundary, optional param, async) name a hunk's intent ([33385bc](https://github.com/FoamScience/ordo/commit/33385bc5bba3bb674d4620672de3d39696661c77))
* **engine:** extent says whether a hunk is whitespace, formatting, edited, rewritten, added or removed ([9a40966](https://github.com/FoamScience/ordo/commit/9a40966ada0decc0f94c945b1f0c4d2687f8e617))
* **engine:** formatting only compares grammar tokens and their ancestry, so re-wraps and trailing commas count ([cc82a6f](https://github.com/FoamScience/ordo/commit/cc82a6f6923fae5d628db3ce55536bdef9f31dcf))
* **engine:** hunks of one file at most 9 unchanged lines apart share a cluster ([00de5f6](https://github.com/FoamScience/ordo/commit/00de5f6297119c2481382f8329f0c16709b7882d))
* **engine:** one edit repeated across files reads as one same-edit group ([c06e63d](https://github.com/FoamScience/ordo/commit/c06e63d04c625d3fd3e7310b536d1c81abd01361))
* **engine:** one edit repeated in several definitions of a file reads as one same-edit group ([c8d1704](https://github.com/FoamScience/ordo/commit/c8d17049296a0742ba959aafc97baa53d5c7b159))
* **engine:** summary counts a literal named by its constant and an order-free reorder as preserving, a move only its moved definitions, says when a rename changes outside callers' API, and pack shows it ([b0023a2](https://github.com/FoamScience/ordo/commit/b0023a23cc0a1c3afc6d700912ea25d07b567f58))
* **engine:** the same token run swapped on lines that differ around it reads as one same edit ([35f298c](https://github.com/FoamScience/ordo/commit/35f298cf7392dfc695cc0e68f61bcefa3f759e00))
* **git:** either end of a range takes a GitButler branch or commit ID ([eeabf4f](https://github.com/FoamScience/ordo/commit/eeabf4fb4c6c057e75a4f02972b7dfaacc633ff9))
* **refine:** moved text marked apart from changed, and comments, strings and prose compared word by word ([5a48dab](https://github.com/FoamScience/ordo/commit/5a48dab4974af2f8f544d55408a39e810a3d737b))
* **refine:** re-wrapped text reads as unchanged, and a file no grammar reads is compared word by word ([25da720](https://github.com/FoamScience/ordo/commit/25da720fa2195bcf285c86ac318c175b153b14d5))
* **web:** :summary puts the review on one page, progress, what you wrote, what ordo flagged, file by file, to copy or print ([d3adea6](https://github.com/FoamScience/ordo/commit/d3adea6913c114e7aca46a0ba3ad438b0ba50792))
* **web:** a caret in the code line, moved by the terminal's column motions ([0b7738f](https://github.com/FoamScience/ordo/commit/0b7738fa109f6afebfe67fe853e6eab23bd024df))
* **web:** a line edited in place reads once, what it lost struck through where it stood, docstrings included ([736cd38](https://github.com/FoamScience/ordo/commit/736cd3842f2422792bb6ec22797b6a2941040c47))
* **web:** a line that only gained text reads as one line with its insertion marked, and next is an arrow ([b244d08](https://github.com/FoamScience/ordo/commit/b244d084ecc0c41c748ac948d696bf947e489a75))
* **web:** a lone hunk sits under its symbol's heading too, so every named hunk reads the same way ([73793c5](https://github.com/FoamScience/ordo/commit/73793c5fa45c1631fd7516c9913c09ab897f4aaf))
* **web:** a reading page — the order as a thread with dependency arcs, one hunk set as prose with its code, the why in the margin ([9cb80bb](https://github.com/FoamScience/ordo/commit/9cb80bbdd2de9e2ca21f13a20268508a2c2b30c8))
* **web:** browser client with status band, centred reading list, code and why panes, themes and the vim keymap ([ee14b13](https://github.com/FoamScience/ordo/commit/ee14b1335f54fe20a4440b8e430bb59b16cf910a))
* **web:** Catppuccin Mocha is the page's default look, graphite under a light :theme ([64666c1](https://github.com/FoamScience/ordo/commit/64666c1f2c44f19d439e8442e74c317883b0c243))
* **web:** dependency canvas, settings, symbol hover, open-in-editor, and a selection that follows commands ([975b182](https://github.com/FoamScience/ordo/commit/975b1824c40cab549b9d0ef4c4c51854bca6fbc9))
* **web:** gD twice or :map draws a hunk's whole dependency chain, what it stands on to the left and what rejecting it strands to the right ([8f28b3a](https://github.com/FoamScience/ordo/commit/8f28b3a1e0b0ab28880c21f925d3766b1a5fdc54))
* **web:** hide empty sections in the why margin ([ece654e](https://github.com/FoamScience/ordo/commit/ece654e8c0851a493ee5ba48b4f8f5dc0cc3f0b2))
* **web:** larger text by default, and :size to scale it per browser ([2f33936](https://github.com/FoamScience/ordo/commit/2f339360f90d52970155ef25443f537284eb9592))
* **web:** ordo web serves a review to the browser from localhost, behind a session token ([f73b30b](https://github.com/FoamScience/ordo/commit/f73b30b5ce553f6d1c46d79c2ae651df13defdda))
* **web:** redesign the reviewer on Blueprint sheets, keylines and a progress dimension string ([fa6b0e8](https://github.com/FoamScience/ordo/commit/fa6b0e83a6817ed140a52b92328780f645efc201))
* **web:** resting on a name peeks at it and the hunks that change it, and a strip beside the code maps the file ([3d9a66a](https://github.com/FoamScience/ordo/commit/3d9a66a440771d73568c7cef7f8ca4a16c120582))
* **web:** s finds a hunk anywhere in the review, ranked by BM25 over identifier pieces ([7929673](https://github.com/FoamScience/ordo/commit/7929673958f93f506826990df26548e0e3fab342))
* **web:** set the page in Geist Mono, bundled, in place of Literata and IBM Plex Mono ([589788d](https://github.com/FoamScience/ordo/commit/589788dc94d96260e01078d2af30c8d0185bfe21))
* **web:** show the change summary above the reading order ([26885b9](https://github.com/FoamScience/ordo/commit/26885b918f2f27ab0afba993eb9fd63907acf367))
* **web:** the reading order groups a symbol's hunks under its name and says each by what sets it apart ([6db56c9](https://github.com/FoamScience/ordo/commit/6db56c93c95087433f57cfbb57356180eec3f125))
* **web:** the summary groups ordo's findings into one card per rule ([853a16a](https://github.com/FoamScience/ordo/commit/853a16a06bd59b3749cebac8e4cc5403e9603fe5))
* **web:** the summary page runs full width, with ordo's findings as cards and no print button ([2701cbb](https://github.com/FoamScience/ordo/commit/2701cbb9fda13307fd2485659e199dae541221ab))


### Bug Fixes

* **engine:** a preprocessor conditional inside a function or type is transparent to its enclosing definition ([4b329eb](https://github.com/FoamScience/ordo/commit/4b329eb11d5fbb35375272b6d76ed51354d657bb))
* **engine:** a rust pattern binds its inner names, not its variant paths or constants, and a js destructuring key or default is not a local ([e00da6a](https://github.com/FoamScience/ordo/commit/e00da6a1b64857f9d0c34814485a620abf024c5e))
* find the home directory on windows too ([8ece059](https://github.com/FoamScience/ordo/commit/8ece059e0055ff393a39c3f08d021f523d506ac2))
* keep a helper extracted next to its source an insertion ([2efe07e](https://github.com/FoamScience/ordo/commit/2efe07e1c98afe4250107bcc5a20d1d045d77daf))
* keep rename, deletion-row and sentence facts per file ([45be4a5](https://github.com/FoamScience/ordo/commit/45be4a53faa42cf027b937810f21587ed930627e))
* name a loop replaced by a helper call instead of its call arguments ([948dce5](https://github.com/FoamScience/ordo/commit/948dce5ebbc2060e0c441ec5ed9708d90cad28b6))
* read a try added without reindenting as a wrap ([126f9d5](https://github.com/FoamScience/ordo/commit/126f9d5ac804a994377339e21aaf8b6cf5dc6544))
* **rules:** lru-cache-on-method matches the cache and lru_cache decorators by name, not [@cached](https://github.com/cached)_property ([40c5f10](https://github.com/FoamScience/ordo/commit/40c5f10856cbfd0e4647e78d9e7694fc95be6544))
* **rules:** mutable-global-state reports the assignment, not the module's first line, and spares import-like calls ([7018f22](https://github.com/FoamScience/ordo/commit/7018f22a07e299e2ca7db989ab75a46ffbd7f4dc))
* **rules:** the rules count includes the built-in catalog, and the findings note says how many rules apply to the file's language ([c07f330](https://github.com/FoamScience/ordo/commit/c07f3305dbfe69ccc7da2c8a5615d0d220de4149))
* split a def added above an edited header into its own hunk ([a3b61bf](https://github.com/FoamScience/ordo/commit/a3b61bfd2a4bd0eeccc85df4bed721974fd648a2))
* **tui:** a mark, note or comment that cannot be written says so ([2644dc0](https://github.com/FoamScience/ordo/commit/2644dc068f76827306969c000ffaa6aedd68819a))
* **tui:** a reviewed mark merges into the marks file instead of overwriting another reviewer's ([d05e7a4](https://github.com/FoamScience/ordo/commit/d05e7a4a97372da5927a9d9cf48ac95b932932ed))
* **tui:** notes and line comments merge into their files and are written atomically ([97bf87b](https://github.com/FoamScience/ordo/commit/97bf87bbf7a48a1a81ffd695cd11a523a229ed85))
* **web:** a link pasted into an open tab takes its new token, and a refused token says which URL to open ([9ed347c](https://github.com/FoamScience/ordo/commit/9ed347cdb01dfdd123eebaedef9cadd82ec1ffe4))
* **web:** a quiet row says what it hides, rows under a heading drop its name, line numbers sit on every row, and an import gaining a name reads as one edited line ([f44fe2e](https://github.com/FoamScience/ordo/commit/f44fe2eaebc4b9466ecfcef19fd1bd0a973517da))
* **web:** a symbol heading's rows hang from a tree guide that closes at its last hunk ([d969795](https://github.com/FoamScience/ordo/commit/d969795aa6b976e4111d98b277de4fff76c4d077))
* **web:** accept a local Host on any port, so a forwarded port reaches the server ([32a2b0e](https://github.com/FoamScience/ordo/commit/32a2b0eb2823d64b8ff621f9f983877be39d677a))


### Performance Improvements

* analyse changed files in parallel ([2da482f](https://github.com/FoamScience/ordo/commit/2da482fc1d05c1032b7d05030e3d3b9e3f1ce427))
* **engine:** details, relocations, renames, rename leftovers, loosened tests and visibility read each file once instead of once per definition ([53015d8](https://github.com/FoamScience/ordo/commit/53015d8fddb7edc03157b006b4a7c1a005d6a757))
* index the summary and intent scans instead of rescanning per hunk ([c8d0127](https://github.com/FoamScience/ordo/commit/c8d0127ed915410db440fc8f6c1856deff00568e))
* run the per-file halves of the later passes in parallel ([b34d2b5](https://github.com/FoamScience/ordo/commit/b34d2b591d52b52d16dfe6a71ec31e29f16dceb9))
* **web:** a file is highlighted outside the review lock, and a review no tab follows is let go ([7dd288b](https://github.com/FoamScience/ordo/commit/7dd288b6211dcb0a611ec615d94dcec38885d670))
