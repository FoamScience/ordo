// `:dismiss` from the why pane: a catalog finding on the selected hunk can be
// dismissed, and dismissed again to undo it. Other sources are the reader's
// own rules or analyzers, which they tune where they wrote them.

import type { Finding } from "./types.ts";

export interface DismissToggle {
  label: string;
  title: string;
  command: string;
}

/** the toggle for `f` on a hunk whose dismissed catalog rules are `dismissed`; null unless `f` is a catalog finding */
export function dismissToggle(f: Finding, dismissed: string[]): DismissToggle | null {
  if (f.source !== "catalog") return null;
  const on = dismissed.includes(f.name);
  return {
    label: on ? "dismissed" : "dismiss",
    title: on
      ? "Undo the dismissal (:dismiss)"
      : "Not worth a look here (:dismiss). After a few in this repository, ordo suggests a level for .ordo/rules.toml",
    command: `dismiss ${f.name}`,
  };
}
