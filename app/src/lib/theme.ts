import { get, writable } from "svelte/store";
import { identityThemeGet, identityThemeSet } from "../identity";

export const THEMES = ["midnight", "dark", "light"] as const;
export type Theme = (typeof THEMES)[number];
const DEFAULT: Theme = "midnight";
const isTheme = (v: unknown): v is Theme =>
  typeof v === "string" && (THEMES as readonly string[]).includes(v);

/** The theme currently applied (for the pickers). */
export const theme = writable<Theme>(DEFAULT);
/** Setup-only, in memory: chosen before an account exists, or not yet saved. */
export const pendingTheme = writable<Theme | null>(null);

export function applyTheme(t: unknown): Theme {
  const next = isTheme(t) ? t : DEFAULT;
  document.documentElement.dataset.theme = next;
  theme.set(next);
  return next;
}

export async function loadTheme(): Promise<void> {
  const pending = get(pendingTheme);
  try {
    applyTheme(pending ?? (await identityThemeGet()).theme);
  } catch {
    applyTheme(pending ?? DEFAULT);
  }
}

/** Optimistic: applies now, persists, reverts on failure and returns the error. */
export async function chooseTheme(t: Theme): Promise<unknown | null> {
  const previous = get(theme);
  applyTheme(t);
  try {
    await identityThemeSet(t);
    pendingTheme.set(null);
    return null;
  } catch (e) {
    applyTheme(previous);
    return e;
  }
}
