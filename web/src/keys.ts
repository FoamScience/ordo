// Keys as the terminal reviewer names them (`j`, `C-w`, `S-F3`, `Space`), so
// the binds ordo sends — its preset plus tui.toml's [binds] — apply here as
// written.

import type { Bind } from "./types.ts";

export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

const NAMED: Record<string, string> = {
  " ": "Space",
  Escape: "Esc",
  Enter: "Enter",
  Backspace: "Backspace",
  Tab: "Tab",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
};

/** The key's label, or null for a lone modifier or a key ordo never binds. */
export function keyLabel(e: KeyLike): string | null {
  if (["Shift", "Control", "Alt", "Meta", "CapsLock", "Dead"].includes(e.key)) return null;
  if (e.metaKey) return null;
  const named = NAMED[e.key] ?? (/^F\d{1,2}$/.test(e.key) ? e.key : null);
  const char = !named && [...e.key].length === 1 ? e.key : null;
  if (!named && !char) return null;
  let prefix = "";
  if (e.ctrlKey) prefix += "C-";
  // a character already carries its case; shift only names a special key
  if (e.shiftKey && named && named !== "Space") prefix += "S-";
  if (e.altKey) prefix += "A-";
  return prefix + (named ?? char!);
}

export type Resolved = { act: string } | { pending: true } | { miss: true };

export class Keymap {
  private readonly binds: Bind[];

  constructor(binds: Bind[]) {
    this.binds = binds;
  }

  /** The terminal's rule: a key that opens a chord waits for the next one. */
  resolve(pending: string | null, key: string): Resolved {
    const find = (pre: string | null) => this.binds.find((b) => b.prefix === pre && b.key === key);
    if (pending !== null) {
      const b = find(pending);
      return b ? { act: b.action } : { miss: true };
    }
    if (this.binds.some((b) => b.prefix === key)) return { pending: true };
    const b = find(null);
    return b ? { act: b.action } : { miss: true };
  }

  /** What `key` does on its own, even when it also opens a chord. */
  bare(key: string): string | null {
    return this.binds.find((b) => b.prefix === null && b.key === key)?.action ?? null;
  }

  /** Every chord bound to `action`, as `?` lists them. */
  keysFor(action: string): string[] {
    return this.binds
      .filter((b) => b.action === action)
      .map((b) => (b.prefix ? `${b.prefix}${b.prefix.length > 1 || b.key.length > 1 ? " " : ""}${b.key}` : b.key));
  }
}
