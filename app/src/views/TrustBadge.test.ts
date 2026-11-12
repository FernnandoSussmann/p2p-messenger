import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/svelte";
import TrustBadge from "./TrustBadge.svelte";

afterEach(cleanup);

describe("TrustBadge", () => {
  it.each([false, undefined, null, "true", 1])("Unverified for %s", (v) => {
    render(TrustBadge, { verified: v });
    expect(screen.getByTestId("badge-unverified").textContent).toBe("Unverified");
    expect(screen.queryByTestId("badge-verified")).toBeNull();
  });

  it("Verified by you only for true", () => {
    render(TrustBadge, { verified: true });
    expect(screen.getByTestId("badge-verified").textContent).toBe("Verified by you");
    expect(screen.queryByTestId("badge-unverified")).toBeNull();
  });

  it("verified copy disclaims cryptographic checking", () => {
    render(TrustBadge, { verified: true });
    const title = screen.getByTestId("badge-verified").getAttribute("title") ?? "";
    expect(title).toContain("You marked this contact as verified");
    expect(title).toContain("did not check this cryptographically");
  });
});
