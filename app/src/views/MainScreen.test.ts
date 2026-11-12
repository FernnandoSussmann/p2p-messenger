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
const unlistenContacts = vi.fn();
const namingQueue = vi.hoisted(() => {
  let value: unknown[] = [];
  const subs = new Set<(v: unknown[]) => void>();
  return {
    subscribe(fn: (v: unknown[]) => void) {
      subs.add(fn);
      fn(value);
      return () => subs.delete(fn);
    },
    set(v: unknown[]) {
      value = v;
      subs.forEach((f) => f(v));
    },
  };
});
vi.mock("../lib/contactsStore", async () => {
  const { writable } = await import("svelte/store");
  return {
    contacts: writable([]),
    contactsError: writable(null),
    namingQueue,
    dropNaming: vi.fn(),
    refreshContacts: vi.fn(async () => {}),
    startContacts: vi.fn(async () => unlistenContacts),
  };
});
const unlistenChat = vi.fn();
vi.mock("../lib/chatStore", async () => {
  const { writable } = await import("svelte/store");
  return {
    activeChat: writable<string | null>(null),
    closeChat: vi.fn(),
    startChatEvents: vi.fn(async () => unlistenChat),
    conversation: writable(null),
    unread: writable({}),
    openChat: vi.fn(),
    loadOlder: vi.fn(),
    sendMessage: vi.fn(),
    resendMessage: vi.fn(),
  };
});
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn() }));

import MainScreen from "./MainScreen.svelte";
import { activeTab } from "../lib/router";
import { incomingRequests, connections } from "../lib/netStore";
import { startContacts } from "../lib/contactsStore";
import { activeChat, closeChat, startChatEvents } from "../lib/chatStore";
import { identityMyInfo } from "../identity";
import { netMyTicket, netPendingIncoming } from "../net";

afterEach(cleanup);
beforeEach(() => {
  unlisten.mockClear();
  unlistenContacts.mockClear();
  unlistenChat.mockClear();
  vi.mocked(closeChat).mockClear();
  activeChat.set(null);
  activeTab.set("contacts");
  namingQueue.set([]);
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
  it("Contacts is the first and default tab", () => {
    render(MainScreen);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual(["Contacts", "My Info", "Network"]);
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { name: "Contacts" })).toBeTruthy();
  });

  it("starts and stops the contacts listener", async () => {
    const { unmount } = render(MainScreen);
    await waitFor(() => expect(startContacts).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unlistenContacts).toHaveBeenCalled());
  });

  it("lists all tabs and switches between them", async () => {
    render(MainScreen);
    await fireEvent.click(screen.getByRole("tab", { name: "My Info" }));
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

  it("naming prompt is shown on every tab", async () => {
    namingQueue.set([{ node_id: "peer-9", self_name: "Eve", nickname: "Eve" }]);
    render(MainScreen);
    expect(await screen.findByTestId("naming-prompt")).toBeTruthy();
    await fireEvent.click(screen.getByRole("tab", { name: "Network" }));
    expect(screen.getByTestId("naming-prompt")).toBeTruthy();
    await fireEvent.click(screen.getByRole("tab", { name: "My Info" }));
    expect(screen.getByTestId("naming-prompt")).toBeTruthy();
  });

  it("runs the net event unlisteners on unmount", async () => {
    const { unmount } = render(MainScreen);
    await waitFor(() => expect(netPendingIncoming).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unlisten.mock.calls.length).toBeGreaterThanOrEqual(2));
  });

  it("active_chat_replaces_tab_panel", async () => {
    activeChat.set("peer-1");
    render(MainScreen);
    expect(await screen.findByRole("button", { name: "Back" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Contacts" })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(closeChat).toHaveBeenCalled();
  });

  it("tab_click_closes_chat", async () => {
    render(MainScreen);
    await fireEvent.click(screen.getByRole("tab", { name: "Network" }));
    expect(closeChat).toHaveBeenCalled();
  });

  it("chat_events_started_and_unlistened", async () => {
    const { unmount } = render(MainScreen);
    await waitFor(() => expect(startChatEvents).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unlistenChat).toHaveBeenCalled());
  });
});
