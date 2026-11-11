import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

const unlisten = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => unlisten),
}));
vi.mock("../identity", async (orig) => ({
  ...(await orig<typeof import("../identity")>()),
  identityMyInfo: vi.fn(),
}));
vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netMyTicket: vi.fn(),
  netPendingIncoming: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn() }));

import MainScreen from "./MainScreen.svelte";
import { activeTab } from "../lib/router";
import { incomingRequests, connections } from "../lib/netStore";
import { identityMyInfo } from "../identity";
import { netMyTicket, netPendingIncoming } from "../net";

afterEach(cleanup);
beforeEach(() => {
  unlisten.mockClear();
  activeTab.set("my-info");
  incomingRequests.set([]);
  connections.set({});
  vi.mocked(identityMyInfo).mockResolvedValue({
    display_name: "Alice",
    node_id: "me-node",
    ticket: "t",
    qr_svg: "<svg></svg>",
  });
  vi.mocked(netMyTicket).mockResolvedValue({ ticket: "t", qr_svg: "<svg></svg>" });
  vi.mocked(netPendingIncoming).mockResolvedValue([]);
});

describe("MainScreen", () => {
  it("lists both tabs and switches between them", async () => {
    render(MainScreen);
    expect(screen.getByRole("tab", { name: "My Info" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("tab", { name: "Network" }));
    expect(screen.getByLabelText("Connect by ticket")).toBeTruthy();
    await fireEvent.click(screen.getByRole("tab", { name: "My Info" }));
    expect(await screen.findByRole("heading", { name: "My Info" })).toBeTruthy();
    expect(screen.queryByLabelText("Connect by ticket")).toBeNull();
  });

  it("shows a seeded prompt on both tabs", async () => {
    vi.mocked(netPendingIncoming).mockResolvedValue([{ node_id: "peer-1", display_name: "Eve", suggested_nickname: "Eve" }]);
    render(MainScreen);
    expect(await screen.findByRole("button", { name: /Accept/ })).toBeTruthy();
    await fireEvent.click(screen.getByRole("tab", { name: "Network" }));
    expect(screen.getByRole("button", { name: /Accept/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Decline" })).toBeTruthy();
  });

  it("runs the net event unlisteners on unmount", async () => {
    const { unmount } = render(MainScreen);
    await waitFor(() => expect(netPendingIncoming).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unlisten.mock.calls.length).toBeGreaterThanOrEqual(2));
  });
});
