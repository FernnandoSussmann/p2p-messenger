import { describe, it, expect } from "vitest";
import { shortNodeId } from "./nodeId";

describe("shortNodeId", () => {
  it("returns short ids unchanged", () => {
    expect(shortNodeId("abc")).toBe("abc");
    expect(shortNodeId("")).toBe("");
  });
  it("truncates a 64-character id", () => {
    const id = "0123456789" + "x".repeat(48) + "abcdef";
    expect(shortNodeId(id)).toBe("0123456789…abcdef");
  });
  it("keeps 20 characters and truncates 21", () => {
    expect(shortNodeId("a".repeat(20))).toBe("a".repeat(20));
    expect(shortNodeId("a".repeat(21))).toBe(`${"a".repeat(10)}…${"a".repeat(6)}`);
  });
});
