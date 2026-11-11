/// <reference types="vite/client" />
import { expect, it } from "vitest";

const sources = import.meta.glob("/src/**/*.svelte", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

it("only MyInfo.svelte uses raw HTML rendering", () => {
  const offenders = Object.entries(sources)
    .filter(([, text]) => text.includes("{@html"))
    .map(([path]) => path);
  expect(offenders).toEqual(["/src/views/MyInfo.svelte"]);
});
