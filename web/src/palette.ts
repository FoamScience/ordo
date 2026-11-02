// The `:` command line: what was typed, and what it could become.

import type { Command } from "./types.ts";

export function parseCommand(line: string): { name: string; arg: string } {
  const t = line.trim().replace(/^:/, "").trimStart();
  const sp = t.search(/\s/);
  return sp < 0 ? { name: t, arg: "" } : { name: t.slice(0, sp), arg: t.slice(sp + 1).trim() };
}

/** Commands the browser answers itself, beside the server's own. */
export const CLIENT_COMMANDS: Command[] = [
  { name: "theme", args: "<name>", help: "switch the colour theme (auto follows the system)" },
  { name: "map", args: "", help: "the hunk's whole chain: what it stands on to its left, what rejecting it strands to its right" },
  { name: "summary", args: "", help: "the review on one page: progress, what you wrote, what ordo flagged; again to go back" },
  { name: "size", args: "<small|normal|large|larger|factor>", help: "make the text and rows bigger or smaller, for this browser" },
];

/** Completions for what has been typed: command names, then a command's arguments. */
export function complete(input: string, commands: Command[], args: Record<string, string[]>): string[] {
  const t = input.replace(/^:/, "");
  const sp = t.indexOf(" ");
  if (sp < 0) {
    return commands.map((c) => c.name).filter((n) => n.startsWith(t));
  }
  const name = t.slice(0, sp);
  const typed = t.slice(sp + 1);
  return (args[name] ?? []).filter((a) => a.startsWith(typed)).map((a) => `${name} ${a}`);
}
