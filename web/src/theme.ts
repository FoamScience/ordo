// ordo's themes as CSS variables. A role a theme leaves unset (null) keeps
// this page's own default for the light or dark base.

import type { Api } from "./api.ts";
import type { Theme } from "./types.ts";

let themes: Record<string, Theme> = {};

/** The theme roles ordo names, and the variable each one sets here. */
export function cssVar(role: string): string | null {
  if (role.startsWith("syntax-")) return `--syn-${role.slice(7)}`;
  const own: Record<string, string> = { border: "--line", "border-focus": "--focus" };
  if (own[role]) return own[role];
  return ["fg", "dim", "accent", "reviewed", "warn"].includes(role) || role.endsWith("-bg") ? `--${role}` : null;
}

/** Light or dark, from how bright the theme's text is. */
export function baseOf(theme: Theme): "light" | "dark" | null {
  const fg = theme["fg"];
  if (!fg || !/^#[0-9a-f]{6}$/i.test(fg)) return null;
  const n = parseInt(fg.slice(1), 16);
  const lum = (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
  return lum > 0.5 ? "dark" : "light";
}

export async function loadThemes(api: Api): Promise<void> {
  try {
    const t = await api.themes();
    // the page has its own look; an ordo theme applies only when asked for
    themes = t.themes;
  } catch {
    themes = {};
  }
}

export function themeNames(): string[] {
  return Object.keys(themes).filter((n) => n !== "dark" && n !== "light");
}

export function applyTheme(name: string): void {
  const root = document.documentElement;
  // a theme owns the colours; `--ui` is the size, which `:size` owns
  for (const p of [...root.style]) if (p.startsWith("--") && p !== "--ui") root.style.removeProperty(p);
  root.removeAttribute("data-base");
  const t = themes[name];
  if (!t || name === "auto") return;
  const base = baseOf(t);
  if (base) root.dataset.base = base;
  for (const [role, value] of Object.entries(t)) {
    const v = cssVar(role);
    if (v && value) root.style.setProperty(v, value);
  }
}
