import { describe, it, expect } from "vitest";
import { formatLocalTime, guardedIso } from "./time";

describe("formatLocalTime", () => {
  it("formats a valid value via Intl", () => {
    const ms = Date.UTC(2024, 4, 6, 12, 30);
    const expected = new Intl.DateTimeFormat(undefined, { dateStyle: "short", timeStyle: "short" }).format(new Date(ms));
    expect(formatLocalTime(ms)).toBe(expected);
  });

  it("rejects non-finite and out-of-range values", () => {
    for (const v of [NaN, Infinity, 9e15]) expect(formatLocalTime(v)).toBe("unknown time");
    expect(guardedIso(9e15)).toBeUndefined();
    expect(guardedIso(0)).toBe("1970-01-01T00:00:00.000Z");
  });
});
