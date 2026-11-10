/// <reference types="vite/client" />
import { expect, it } from "vitest";

const sources = import.meta.glob("/src/**/*.{svelte,ts}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

it("only MyInfo.svelte touches the clipboard", () => {
  const offenders = Object.entries(sources)
    .filter(([path]) => !path.endsWith(".test.ts"))
    .filter(([, text]) => /writeText|clipboard/.test(text))
    .map(([path]) => path);
  expect(offenders).toEqual(["/src/views/MyInfo.svelte"]);
});

it("MyInfo copies only the ticket or node id", () => {
  const text = sources["/src/views/MyInfo.svelte"];
  const calls = text.match(/writeText\(([^)]*)\)/g) ?? [];
  expect(calls).toEqual(["writeText(value)"]);
  expect(text).toMatch(/copy\(info\.ticket,/);
  expect(text).toMatch(/copy\(info\.node_id,/);
  expect(text.match(/copy\(info\./g)).toHaveLength(2);
});
