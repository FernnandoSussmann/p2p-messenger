/// <reference types="vite/client" />
import { expect, it } from "vitest";
import indexHtml from "../index.html?raw";
import { appCss, withoutThemeBlocks } from "./theme-css";

const svelte = import.meta.glob("/src/**/*.svelte", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const scripts = import.meta.glob(["/src/**/*.ts", "/src/**/*.svelte"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;
const cssFiles = import.meta.glob("/src/**/*.css");

const COLOUR_WORD =
  /(?<![\w-])(white|black|red|green|blue|gray|grey|orange|yellow|purple|pink|brown|navy|teal|silver|maroon|olive|lime|aqua|fuchsia)(?![\w-])/i;
const HEX = /#[0-9a-f]{3,8}\b/i;
const COLOUR_FN = /\b(rgba?|hsla?)\(/i;

it("no component carries its own styles", () => {
  const offenders = Object.entries(svelte)
    .filter(([, t]) => /<style/.test(t) || /\sstyle=/.test(t) || /\sstyle:/.test(t))
    .map(([p]) => p);
  expect(offenders).toEqual([]);
});

it("app.css is the only stylesheet and main.ts the only importer", () => {
  expect(Object.keys(cssFiles)).toEqual(["/src/app.css"]);
  const importers = Object.entries(scripts)
    .filter(([p]) => !p.endsWith(".test.ts"))
    .filter(([, t]) => /^\s*import\s[^;]*["'][^"']+\.css(\?[^"']*)?["']/m.test(t))
    .map(([p]) => p);
  expect(importers).toEqual(["/src/main.ts"]);
});

it("index.html defaults to Midnight and has no inline or linked styles", () => {
  expect(indexHtml).toContain('data-theme="midnight"');
  expect(indexHtml).not.toContain("<style");
  expect(indexHtml).not.toContain('rel="stylesheet"');
});

it("app.css has no colour literal outside the theme blocks", () => {
  const rest = withoutThemeBlocks(appCss).replace(/\/\*[\s\S]*?\*\//g, "");
  expect(rest).not.toMatch(HEX);
  expect(rest).not.toMatch(COLOUR_FN);
  expect(rest).not.toMatch(COLOUR_WORD);
});

it("app.css loads nothing remote", () => {
  expect(appCss).not.toMatch(/@import|url\(|https?:\/\//);
});

it("components have no colour literal", () => {
  const offenders = Object.entries(svelte)
    .filter(([, t]) => HEX.test(t.replace(/&#\d+;/g, "")) || COLOUR_FN.test(t))
    .map(([p]) => p);
  expect(offenders).toEqual([]);
});

it("no color-mix or :has in the stylesheet or components", () => {
  for (const text of [appCss, ...Object.values(svelte)]) {
    expect(text).not.toContain("color-mix(");
    expect(text).not.toContain(":has(");
  }
});

it("the only runtime style write is the composer height", () => {
  const writes = Object.entries(scripts)
    .filter(([p]) => !p.endsWith(".test.ts"))
    .flatMap(([p, t]) => [...t.matchAll(/\.style\.(\w+)/g)].map((m) => `${p}:${m[1]}`));
  expect(new Set(writes)).toEqual(new Set(["/src/views/Composer.svelte:height"]));
});
