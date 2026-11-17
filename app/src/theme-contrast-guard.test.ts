import { describe, expect, it } from "vitest";
import { appCss, THEME_NAMES, themeBlocks } from "./theme-css";

const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
const lin = (c: number) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const lum = ([r, g, b]: number[]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const ratio = (a: number[], b: number[]) => {
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
const over = (fg: number[], a: number, bg: number[]) => fg.map((v, i) => v * a + bg[i] * (1 - a));

const blocks = themeBlocks(appCss);
const rgbaOf = (v: string) => {
  const m = v.match(/^rgba\((\d+), (\d+), (\d+), (\.\d+)\)$/);
  if (!m) throw new Error(`not an rgba value: ${v}`);
  return { rgb: [+m[1], +m[2], +m[3]], alpha: +m[4] };
};

describe.each(THEME_NAMES)("%s theme", (name) => {
  const t = blocks[name];
  const c = (token: string) => rgb(t[token]);
  const check = (fg: number[], bg: number[], label: string) =>
    expect(ratio(fg, bg), label).toBeGreaterThanOrEqual(4.5);

  it("text and accent colours reach AA on bg and surface", () => {
    for (const token of ["--text", "--text-strong", "--text-muted", "--accent", "--success", "--warning", "--danger"]) {
      for (const surface of ["--bg", "--surface"]) check(c(token), c(surface), `${token} on ${surface}`);
    }
  });

  it("status colours reach AA on their tint over bg and surface", () => {
    for (const k of ["accent", "success", "warning", "danger"]) {
      const { rgb: tint, alpha } = rgbaOf(t[`--${k}-soft`]);
      for (const surface of ["--bg", "--surface"]) {
        check(c(`--${k}`), over(tint, alpha, c(surface)), `--${k} on soft over ${surface}`);
      }
    }
  });

  it("bubbles and buttons reach AA", () => {
    check(c("--bubble-in-text"), c("--bubble-in"), "incoming bubble");
    check(c("--bubble-out-text"), c("--bubble-out"), "outgoing bubble");
    check(c("--accent-contrast"), c("--accent"), "primary button");
  });

  it("precomputed tints follow the reference formula", () => {
    for (const k of ["accent", "success", "warning", "danger"]) {
      expect(rgbaOf(t[`--${k}-soft`])).toEqual({ rgb: c(`--${k}`), alpha: 0.14 });
    }
    expect(t["--focus"]).toBe(`0 0 0 3px rgba(${c("--accent").join(", ")}, .40)`);
  });
});
