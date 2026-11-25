// The short forms of sentences the right sidebar and the band used to print,
// for an icon and a number; the sentence itself goes to the tooltip.

/** What the watcher's "stale · 3 files changed · r reloads" boils down to: `3`, `w2` for a wave, `HEAD` for a moved base. */
export function staleShort(line: string): string {
  const files = line.match(/(\d+) files?/);
  if (files) return files[1];
  const wave = line.match(/wave (\d+)/);
  if (wave) return `w${wave[1]}`;
  return /HEAD/.test(line) ? "HEAD" : "";
}

/** How many commits touched the file, as `8 commits in 6 months`. */
export function commitsText(n: number): string {
  return `${n} commit${n === 1 ? "" : "s"} in 6 months`;
}

/** How often these lines changed before, as `changed 3 times` (`5+` when the count was capped). */
export function changedText(commits: number, capped: boolean): string {
  return `changed ${commits}${capped ? "+" : ""} time${commits === 1 && !capped ? "" : "s"}`;
}

/** Executable lines under a test, as `tests 12/14 lines`. */
export function coverageText(executed: [number, number]): string {
  return executed.every(Number.isFinite) ? `tests ${executed[0]}/${executed[1]} lines` : "tests –";
}

/** The stale line as the band says it, `stale 3`, and as words that start with that. */
export function staleWords(line: string): { short: string; words: string } {
  const n = staleShort(line);
  return { short: n ? `stale ${n}` : "stale", words: `${n ? `stale ${n}` : "stale"}, ${line.replace(/^stale · /, "")}` };
}
