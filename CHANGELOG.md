# Changelog

## [0.8.0](https://github.com/FoamScience/ordo/compare/v0.7.0...v0.8.0) (2026-11-25)


### Features

* **web:** code-pane text copies clean, keeps its selection, and y/Y copy lines and hunk ([ccfd4ff](https://github.com/FoamScience/ordo/commit/ccfd4ff64848b15be365558672db8ca773be9694))
* **web:** icons for the margin, band and pills, with their words in tooltips ([814803f](https://github.com/FoamScience/ordo/commit/814803f2ca788971e25719bd35caee21dfccf903))


### Bug Fixes

* **rationale:** a decorator-only hunk of an added definition says it adds a decorator ([58ce45e](https://github.com/FoamScience/ordo/commit/58ce45eb1015a8d2d5f80bdfc91110fdd32e3aa7))
* **rationale:** a moved definition is added, java member calls and prototype parameters are not uses of a same-named local ([9d8f474](https://github.com/FoamScience/ordo/commit/9d8f4745e6d6a14a37286de1c6c6105823f9b1e1))
* **rationale:** a test that calls a macro, or a function from a helper, uses it instead of testing it ([36bcc0c](https://github.com/FoamScience/ordo/commit/36bcc0cfeacdd0d3ec02e51d15895cd7b96cac44))
* **rationale:** an import hunk that only adds names to a statement names the arrivals, not the names it kept ([b16c5e6](https://github.com/FoamScience/ordo/commit/b16c5e6a1e8a2898e169db953c3ee76c4f5c340b))
* **rationale:** an import whose module changed or whose glob was added or dropped is not formatting, and the glob is named ([d5f556a](https://github.com/FoamScience/ordo/commit/d5f556a4a8df87035d7c3bbca81652d61e24f75c))
* **rationale:** an imported name is compared with its own module, a glob insert names the glob ([3c5a42a](https://github.com/FoamScience/ordo/commit/3c5a42adf8ea3c97e807c9e5fad02260400699bb))
* **rationale:** rename pairing needs real body overlap in data and prose, catches empty-bodied typo renames, and names a rename's other edits ([74778a7](https://github.com/FoamScience/ordo/commit/74778a7c9425aa96a62cb3518fb9f147f94d0317))
* **rationale:** rust attributes stay in the piece of the item they mark, so a large insertion cuts no attribute-only card ([9fa6ad8](https://github.com/FoamScience/ordo/commit/9fa6ad801d8dc8f81122d5791c6e9f4853079f48))

## [0.7.0](https://github.com/FoamScience/ordo/compare/v0.6.0...v0.7.0) (2026-11-22)


### Features

* **corpus:** attention labels for gold (SZZ later_fix + Opus attention) ([6401ba3](https://github.com/FoamScience/ordo/commit/6401ba32248ad955edea81e16bbc1c8fcf198887))
* **corpus:** extend the skip sample and merge new faithful labels without redrawing ([e62d972](https://github.com/FoamScience/ordo/commit/e62d9721d078cf58caf93d9938add7294f912f99))
* **corpus:** gold audit re-run by claude-fable-5-2 on the 30 rows the refresh made stale; dropped sample key resampled ([2f2cf8c](https://github.com/FoamScience/ordo/commit/2f2cf8c528922873e454e5d0689dc984fe045f40))
* **corpus:** gold audit sample labeled by claude-fable-5-2 in labels_audit ([e0c8b8c](https://github.com/FoamScience/ordo/commit/e0c8b8cdb336da01472ea9d90fd0dbc5cbcc36a6))
* **corpus:** gold faithful relabelled by Opus under the clarified wording ([15a455d](https://github.com/FoamScience/ordo/commit/15a455d4ae2644ce349e7a6b58358ae6ec9680e2))
* **corpus:** gold refreshed against the current engine, changed rows relabelled by Opus ([6a48dfc](https://github.com/FoamScience/ordo/commit/6a48dfcad4f2ff46c1b84d321eb55bc468ef02b8))
* **corpus:** gold set labeled with should_skip ([3b6b4fe](https://github.com/FoamScience/ordo/commit/3b6b4fe8e0d3b82ce13699344015b93e140562f6))
* **corpus:** mine GitHub and Gerrit review comments into per-hunk attention and later-fix labels ([a162369](https://github.com/FoamScience/ordo/commit/a162369c0507eaa3cd7e57f0f20beb6cb9e5832d))
* **corpus:** rationale scoreboard re-scores faithful labels against the current engine ([23a38b4](https://github.com/FoamScience/ordo/commit/23a38b4b06a82ef48db0b29397f2433b32f4073f))
* **engine:** --hunch applies the gated policy to noise flags and marks each change ([ed078b6](https://github.com/FoamScience/ordo/commit/ed078b63c1c936553bc56e6bf73e418c99a4c208))
* export per-hunk hunch feature vectors with git history and a hashed schema ([ae804e6](https://github.com/FoamScience/ordo/commit/ae804e600cf61856f1739a541f3d77c0af224fd9))
* **hunch:** --learning-curve retrains each head on label subsets and review repo counts ([8de50d5](https://github.com/FoamScience/ordo/commit/8de50d595fb8da91b170d5313a041a637e9ec537))
* **hunch:** attention head orders the groups the def-&gt;use order freed under --hunch; gate passes (LORO .820 vs file position .559, gold .817 vs .500) ([4c6ae6b](https://github.com/FoamScience/ordo/commit/4c6ae6bb4991d675d369f7af6658130698a446a0))
* **hunch:** faithful head as a shadow guess beside P(skip); doubt gate fails, so no rationale marker yet ([67b465a](https://github.com/FoamScience/ordo/commit/67b465aa933048fd50f6ed6518992529574e7213))
* **hunch:** promotion gate tests the shipped policy; model records its thresholds and verdict ([1f93dfd](https://github.com/FoamScience/ordo/commit/1f93dfd5bd4a3751ae12be84bcc0c51318bb4b09))
* **hunch:** review labels train the attention head as within-snapshot lambdarank, gated leave-one-repo-out on review repos ([e3e7894](https://github.com/FoamScience/ordo/commit/e3e78948c91067322c5b56d872b8f3eea8760636))
* **hunch:** score later_fix on review hunks within each snapshot against file order and the previous model ([b54450f](https://github.com/FoamScience/ordo/commit/b54450feff956ce7847394ab8ecec0d4fdde160d))
* **hunch:** shadow P(skip) guess from build-time model arrays, opt-in via --hunch and options.hunch ([2581117](https://github.com/FoamScience/ordo/commit/25811176e4a7914663f589f6c5284c2e4bba36c2))
* **hunch:** skip promotion gate accepts a tie once the noise flag is near zero, and retrain ships lightgbm ([b923d34](https://github.com/FoamScience/ordo/commit/b923d34f3c3b54291d220a649ae723e02c9c28d1))
* **hunch:** trainer reports the promotion gate (per-repo equal hide rate) and the promoted policy's numbers ([5f9323d](https://github.com/FoamScience/ordo/commit/5f9323d99199a813323aadcc79a9a70b4680db13))
* per-definition readability deltas as rule conditions and a summary line ([0f1693a](https://github.com/FoamScience/ordo/commit/0f1693ab6d61f4a2d802ea3dd181fa3237590614))
* per-definition readability deltas on json hunks, quoted in delta findings and shown in the web reviewer ([6839fc9](https://github.com/FoamScience/ordo/commit/6839fc999859797aa4cdf63168228c383bfdd184))
* **php:** php support, $-spelled variables, properties used through $this, use and require as imports ([f356241](https://github.com/FoamScience/ordo/commit/f356241de64e02dd3956956a9270ef7d1f52fd0e))
* **scripts:** attention-label computes SZZ later_fix and collects Opus 1-3 attention labels ([680ce0f](https://github.com/FoamScience/ordo/commit/680ce0fa6888ef781e47d1aa92c2042184c2f005))
* **scripts:** corpus-fetch deepens clones to cover the hunch history window ([abe597f](https://github.com/FoamScience/ordo/commit/abe597f72e269a3c5ab98dd8958c4f5414496382))
* **scripts:** faithful asks for nothing false or misleading; terse but true is faithful ([44cb5d5](https://github.com/FoamScience/ordo/commit/44cb5d511fad44aa65d25e71a202b4558755e7f2))
* **scripts:** finding-sample labels catalog findings with Opus; finding-eval shows P(finding warranted) does not carry across repos ([d85cf74](https://github.com/FoamScience/ordo/commit/d85cf74cd7729819ea745991ab0d0428bad8a06f))
* **scripts:** gold-label --audit-render/--audit-merge make the model audit reproducible ([c4b91f1](https://github.com/FoamScience/ordo/commit/c4b91f1ed904e1f039e07d5926c696c425b5de8d))
* **scripts:** gold-label --audit-report for a model auditor's labels_audit, with rejected-pool counts ([53c3b90](https://github.com/FoamScience/ordo/commit/53c3b90604569517e35832cfe1420aa631c061cf))
* **scripts:** gold-label audit re-audit mode keeps prior audits, --keys subset render, --audit-replace for dropped sample keys ([480d1c8](https://github.com/FoamScience/ordo/commit/480d1c8c2f995456dfa91f69146a92716c242754))
* **scripts:** gold-sample --refresh reruns the engine on a labeled set and nulls stale labels ([de2608a](https://github.com/FoamScience/ordo/commit/de2608a871fc8ab951c46ddbf1a582129b73d566))
* **scripts:** human audit mode for the gold judge with seeded sample and agreement report ([68ec7e8](https://github.com/FoamScience/ordo/commit/68ec7e8d99f59514e1d7543c745935544bf9bae0))
* **scripts:** hunch history features follow renames ([bb07064](https://github.com/FoamScience/ordo/commit/bb07064f17b153e0c81f0f503d38356fa3ac6776))
* **scripts:** hunch trainer for should_skip with logistic and LightGBM, writing src/hunch.json ([c09052d](https://github.com/FoamScience/ordo/commit/c09052daf4cb5fc08f5200152d94a91caf23fd74))
* **scripts:** hunch-train holds gold out, matches drifted labels by overlap, picks trees on LORO wrong hides ([6ebe347](https://github.com/FoamScience/ordo/commit/6ebe347f7d2cad6d5ce3b381e4d5662743fcc558))
* **scripts:** hunch-train weights extra labels by stratum, stops trees on LORO and ships a trimmed f32 model ([67a6f8a](https://github.com/FoamScience/ordo/commit/67a6f8ac7ed8a0463959baefce29dc79b8159481))
* **scripts:** stratified should_skip sample of the judged sweep for Opus labels ([b70d811](https://github.com/FoamScience/ordo/commit/b70d811387aec9bd0870181295eaa2807b242e6a))
* **scripts:** two-sided should_skip question for every hunk in the judge flow ([7262c55](https://github.com/FoamScience/ordo/commit/7262c559d5424197c5f73a96c69c9200fc55c1fa))
* **web:** :dismiss a catalog finding; repeated dismissals suggest a per-repo rules.toml level ([c68c555](https://github.com/FoamScience/ordo/commit/c68c5558a7ab7ca9e7c1e3fbd5b629873da34ca8))


### Bug Fixes

* **engine:** re-exports are API, not import noise ([b371022](https://github.com/FoamScience/ordo/commit/b3710225026ad2a79d20eff8066cae81fc719723))
* **hunch:** history excludes the change by topology, not timestamp ([6e98cff](https://github.com/FoamScience/ordo/commit/6e98cffbbf26bc95c636c5fc8f288d5fb887db1f))
* **rationale:** a move with several candidate sources names none ([66cfddf](https://github.com/FoamScience/ordo/commit/66cfddf4cb82c54962b49fd62919d019e710cebe))
* **rationale:** an import hunk's 'and N more' counts imports written whole inside it ([8d63f90](https://github.com/FoamScience/ordo/commit/8d63f9028aed7a767acdcb082c9c2a3998e2bba6))
* **rationale:** an import swapped in place replaces, and imports beside code are not an import hunk ([d0d44b5](https://github.com/FoamScience/ordo/commit/d0d44b56d1784b85db6019cf40fee154824cace4))
* **rationale:** self-check import, add, local and count claims against the engine's facts ([c55d7b9](https://github.com/FoamScience/ordo/commit/c55d7b918f5fdae93a7c8ba77193142dffe268c0))
* **rationale:** self-check parameter uses, repeated data-file moves, reindented includes and guard adds ([a9f7ff7](https://github.com/FoamScience/ordo/commit/a9f7ff7fb6423f05e3d76693d9f2b11f45017970))
* **scripts:** hunch history follows a rename made by the sampled commit itself ([4669459](https://github.com/FoamScience/ordo/commit/4669459332354b97376dc213db97301b516635b2))
* slide ambiguous insert/delete hunks onto syntax boundaries ([407cd96](https://github.com/FoamScience/ordo/commit/407cd9631f58e8b1142e55d9476dfed1e98d4d63))
* **web:** light :theme takes its base from the band when the page owns the text ([66cfdb9](https://github.com/FoamScience/ordo/commit/66cfdb948c417a2171b948fcd4fec06b0b7200ae))
* **web:** summary padding, labelled next hunk, row text, dialog focus and listbox semantics ([590d56e](https://github.com/FoamScience/ordo/commit/590d56ebf33ec74fe459ba5d58470d5699c23331))

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
