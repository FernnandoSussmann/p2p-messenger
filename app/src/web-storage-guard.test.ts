/// <reference types="vite/client" />
import { expect, it } from "vitest";

const sources = import.meta.glob(["/src/**/*.ts", "/src/**/*.svelte"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

it("nothing persists data in web storage", () => {
  const offenders = Object.entries(sources)
    .filter(([path]) => !path.endsWith(".test.ts"))
    .filter(([, t]) => /localStorage|sessionStorage|document\.cookie|indexedDB/.test(t))
    .map(([path]) => path);
  expect(offenders).toEqual([]);
});
