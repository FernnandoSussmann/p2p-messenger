import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netPresenceGet: vi.fn(),
  netPresenceSet: vi.fn(),
}));

import PresenceToggle from "./PresenceToggle.svelte";
import { netPresenceGet, netPresenceSet } from "../net";

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(netPresenceGet).mockReset().mockResolvedValue({ enabled: true });
  vi.mocked(netPresenceSet).mockReset();
});

describe("PresenceToggle", () => {
  it("loads the persisted state", async () => {
    vi.mocked(netPresenceGet).mockResolvedValue({ enabled: false });
    render(PresenceToggle);
    const sw = screen.getByRole("switch", { name: "Visible on this network" });
    await waitFor(() => expect(sw).toBeEnabled());
    expect(sw).not.toBeChecked();
  });

  it("toggles via netPresenceSet", async () => {
    vi.mocked(netPresenceSet).mockResolvedValue({ enabled: false });
    render(PresenceToggle);
    const sw = screen.getByRole("switch", { name: "Visible on this network" });
    await waitFor(() => expect(sw).toBeEnabled());
    expect(sw).toBeChecked();
    await fireEvent.click(sw);
    await waitFor(() => expect(netPresenceSet).toHaveBeenCalledWith(false));
    await waitFor(() => expect(sw).not.toBeChecked());
  });

  it("reverts and alerts on error", async () => {
    vi.mocked(netPresenceSet).mockRejectedValue({ type: "SettingsUnavailable" });
    render(PresenceToggle);
    const sw = screen.getByRole("switch", { name: "Visible on this network" });
    await waitFor(() => expect(sw).toBeEnabled());
    await fireEvent.click(sw);
    expect(await screen.findByRole("alert")).toHaveTextContent("Settings are unavailable");
    expect(sw).toBeChecked();
  });

  it("explains that invisible is not offline", () => {
    render(PresenceToggle);
    expect(screen.getByText("not", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByText(/anyone who already has your ticket can still connect/)).toBeInTheDocument();
  });
});
