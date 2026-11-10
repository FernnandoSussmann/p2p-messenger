import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const status = vi.fn();
vi.mock("./identity", () => ({
  identityStatus: () => status(),
  describeError: () => "boom",
}));

import App from "./App.svelte";

describe("App view machine", () => {
  beforeEach(() => status.mockReset());
  afterEach(cleanup);

  it("shows the wizard when absent", async () => {
    status.mockResolvedValue({ type: "Absent" });
    render(App);
    await screen.findByText("Welcome");
  });

  it("shows main screen with My Info tab when present", async () => {
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    render(App);
    await screen.findByRole("tab", { name: "My Info" });
  });

  it("blocks on error with retry and never provisions", async () => {
    status.mockResolvedValue({ type: "Error", reason: "locked" });
    render(App);
    await screen.findByText("locked");
    status.mockResolvedValue({ type: "Absent" });
    screen.getByRole("button", { name: "Retry" }).click();
    await waitFor(() => expect(screen.getByText("Welcome")).toBeTruthy());
  });
});
