/// <reference types="vite/client" />
const files = import.meta.glob("/src/app.css", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

export const appCss: string = files["/src/app.css"];

export const THEME_NAMES = ["midnight", "dark", "light"] as const;
const BLOCK = /\[data-theme="(\w+)"\]\s*\{([^}]*)\}/g;

export function parseTokens(body: string): Record<string, string> {
  const tokens: Record<string, string> = {};
  for (const m of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) tokens[m[1]] = m[2].trim();
  return tokens;
}

/** Custom properties declared in each `[data-theme="x"] { ... }` block of a stylesheet. */
export function themeBlocks(css: string): Record<string, Record<string, string>> {
  const out: Record<string, Record<string, string>> = {};
  for (const m of css.matchAll(BLOCK)) out[m[1]] = parseTokens(m[2]);
  return out;
}

export const withoutThemeBlocks = (css: string) => css.replace(BLOCK, "");

export const REFERENCE_TOKENS = [
  "--bg", "--surface", "--surface-2", "--border", "--border-strong", "--text", "--text-strong",
  "--text-muted", "--accent", "--accent-hover", "--accent-contrast", "--success", "--warning",
  "--danger", "--bubble-in", "--bubble-in-text", "--bubble-out", "--bubble-out-text", "--shadow",
];
