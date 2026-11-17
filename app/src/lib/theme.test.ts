import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

vi.mock("../identity", () => ({ identityThemeGet: vi.fn(), identityThemeSet: vi.fn() }));

import { applyTheme, loadTheme, chooseTheme, theme, pendingTheme } from "./theme";
import { identityThemeGet, identityThemeSet } from "../identity";

const attr = () => document.documentElement.dataset.theme;

beforeEach(() => {
  vi.resetAllMocks();
  pendingTheme.set(null);
  applyTheme("midnight");
});

describe("applyTheme", () => {
  it.each(["midnight", "dark", "light"])("applies %s", (t) => {
    expect(applyTheme(t)).toBe(t);
    expect(attr()).toBe(t);
    expect(get(theme)).toBe(t);
  });

  it.each(["blue", "", undefined, 'midnight" onload="x'])("falls back for %s", (t) => {
    applyTheme("light");
    expect(applyTheme(t)).toBe("midnight");
    expect(attr()).toBe("midnight");
  });
});

describe("loadTheme", () => {
  it("applies the fetched value", async () => {
    vi.mocked(identityThemeGet).mockResolvedValue({ theme: "dark" });
    await loadTheme();
    expect(attr()).toBe("dark");
  });

  it("falls back to midnight on rejection", async () => {
    applyTheme("light");
    vi.mocked(identityThemeGet).mockRejectedValue({ type: "Internal" });
    await loadTheme();
    expect(attr()).toBe("midnight");
  });

  it("prefers a pending theme", async () => {
    pendingTheme.set("light");
    vi.mocked(identityThemeGet).mockResolvedValue({ theme: "dark" });
    await loadTheme();
    expect(attr()).toBe("light");
  });
});

describe("chooseTheme", () => {
  it("applies, saves and clears pending", async () => {
    pendingTheme.set("light");
    vi.mocked(identityThemeSet).mockResolvedValue({ theme: "dark" });
    expect(await chooseTheme("dark")).toBeNull();
    expect(identityThemeSet).toHaveBeenCalledWith("dark");
    expect(attr()).toBe("dark");
    expect(get(pendingTheme)).toBeNull();
  });

  it("reverts and returns the error on failure", async () => {
    applyTheme("light");
    const err = { type: "Internal" };
    vi.mocked(identityThemeSet).mockRejectedValue(err);
    expect(await chooseTheme("dark")).toBe(err);
    expect(attr()).toBe("light");
    expect(get(theme)).toBe("light");
  });
});
