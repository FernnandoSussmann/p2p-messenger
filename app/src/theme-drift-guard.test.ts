/// <reference types="vite/client" />
import { expect, it } from "vitest";
import sheet from "../../design/visual-identity.html?raw";
import { appCss, parseTokens, REFERENCE_TOKENS, THEME_NAMES, themeBlocks } from "./theme-css";

const APP_ONLY = [
  "--accent-soft", "--success-soft", "--warning-soft", "--danger-soft", "--focus",
  "--accent-line", "--success-line", "--warning-line", "--danger-line", "--danger-line-strong",
  "--backdrop", "--qr-quiet",
];

const norm = (v: string) => v.replace(/\s+/g, " ").toLowerCase();

function referenceTheme(name: string): Record<string, string> {
  const head = name === "dark" ? String.raw`:root,\s*\[data-theme="dark"\]` : String.raw`\[data-theme="${name}"\]`;
  const m = sheet.match(new RegExp(`${head}\\s*\\{([^}]*)\\}`));
  if (!m) throw new Error(`reference block for ${name} not found`);
  return parseTokens(m[1]);
}

const app = themeBlocks(appCss);

it.each(THEME_NAMES)("%s matches the reference sheet", (name) => {
  const ref = referenceTheme(name);
  for (const token of REFERENCE_TOKENS) {
    expect(norm(app[name][token] ?? "<missing>"), `${name} ${token}`).toBe(norm(ref[token]));
  }
});

it("every theme declares the same tokens: the reference set plus the app-only list", () => {
  const expected = [...REFERENCE_TOKENS, ...APP_ONLY].sort();
  for (const name of THEME_NAMES) expect(Object.keys(app[name]).sort()).toEqual(expected);
});
