/// <reference types="vite/client" />
import { expect, it } from "vitest";

const sources = import.meta.glob("/src/views/{Chat,MessageBubble,Composer}.svelte", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

it("chat_components_have_no_links", () => {
  expect(Object.keys(sources)).toHaveLength(3);
  for (const [path, text] of Object.entries(sources)) {
    for (const forbidden of ["<a", "href", "window.open", "location"]) {
      expect(text.includes(forbidden), `${path} contains ${forbidden}`).toBe(false);
    }
  }
});
