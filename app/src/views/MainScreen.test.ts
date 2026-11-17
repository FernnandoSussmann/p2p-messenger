import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { get } from "svelte/store";
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
import { screen as routerScreen, showChat, showTab } from "../lib/router";
import { incomingRequests, connections } from "../lib/netStore";
import { startContacts } from "../lib/contactsStore";
import { startChatEvents } from "../lib/chatStore";
import { identityMyInfo } from "../identity";
import { netMyTicket, netPendingIncoming } from "../net";

afterEach(cleanup);
beforeEach(() => {
  unlisten.mockClear();
  unlistenContacts.mockClear();
  unlistenChat.mockClear();
  showTab("contacts");
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
    expect(tabs.map((t) => t.textContent)).toEqual(["Contacts", "Discover", "Network", "My Info"]);
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { name: "Contacts" })).toBeTruthy();
  });

  it("tabs use roving tabindex and aria-controls", () => {
    render(MainScreen);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.getAttribute("tabindex"))).toEqual(["0", "-1", "-1", "-1"]);
    for (const t of tabs) {
      const panelId = t.getAttribute("aria-controls")!;
      if (t.getAttribute("aria-selected") === "true") {
        expect(screen.getByRole("tabpanel").id).toBe(panelId);
      }
    }
  });

  it("footer p2p-messenger is on every tab", async () => {
    render(MainScreen);
    for (const name of ["Contacts", "Discover", "Network", "My Info"]) {
      await fireEvent.click(screen.getByRole("tab", { name }));
      expect(screen.getByText("p2p-messenger").tagName).toBe("FOOTER");
    }
  });

  it("arrow keys wrap, select and keep focus on the tab", async () => {
    render(MainScreen);
    const first = screen.getByRole("tab", { name: "Contacts" });
    first.focus();
    await fireEvent.keyDown(first, { key: "ArrowLeft" });
    const last = screen.getByRole("tab", { name: "My Info" });
    await waitFor(() => expect(last.getAttribute("aria-selected")).toBe("true"));
    await waitFor(() => expect(document.activeElement).toBe(last));
    await fireEvent.keyDown(last, { key: "ArrowRight" });
    const home = screen.getByRole("tab", { name: "Contacts" });
    await waitFor(() => expect(document.activeElement).toBe(home));
    expect(home.getAttribute("aria-selected")).toBe("true");
  });

  it("clicking a tab moves focus to the hidden heading marked data-screen-focus", async () => {
    render(MainScreen);
    for (const name of ["Discover", "Network", "My Info", "Contacts"]) {
      await fireEvent.click(screen.getByRole("tab", { name }));
      const marked = (await screen.findByRole("heading", { name, hidden: true })) as HTMLElement;
      expect(marked.hasAttribute("data-screen-focus")).toBe(true);
      expect(marked.tagName).toBe("H2");
      expect(marked.textContent).toBe(name);
      expect(marked.classList.contains("visually-hidden")).toBe(true);
      expect(marked.getAttribute("tabindex")).toBe("-1");
      await waitFor(() => expect(document.activeElement).toBe(marked));
    }
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

  it("shows a seeded prompt on every tab and on the chat screen", async () => {
    vi.mocked(netPendingIncoming).mockResolvedValue([{ node_id: "peer-1", display_name: "Eve", suggested_nickname: "Eve" }]);
    render(MainScreen);
    expect(await screen.findByRole("button", { name: /Accept/ })).toBeTruthy();
    for (const name of ["Discover", "Network", "My Info", "Contacts"]) {
      await fireEvent.click(screen.getByRole("tab", { name }));
      expect(screen.getByRole("button", { name: /Accept/ })).toBeTruthy();
      expect(screen.getByRole("button", { name: "Decline" })).toBeTruthy();
    }
    showChat("peer-1");
    await screen.findByRole("button", { name: "Back" });
    expect(screen.getByRole("button", { name: /Accept/ })).toBeTruthy();
  });

  it("naming prompt is shown on every tab", async () => {
    namingQueue.set([{ node_id: "peer-9", self_name: "Eve", nickname: "Eve" }]);
    render(MainScreen);
    expect(await screen.findByTestId("naming-prompt")).toBeTruthy();
    for (const name of ["Discover", "Network", "My Info", "Contacts"]) {
      await fireEvent.click(screen.getByRole("tab", { name }));
      expect(screen.getByTestId("naming-prompt")).toBeTruthy();
    }
    showChat("peer-1");
    await screen.findByRole("button", { name: "Back" });
    expect(screen.getByTestId("naming-prompt")).toBeTruthy();
  });

  it("runs the net event unlisteners on unmount", async () => {
    const { unmount } = render(MainScreen);
    await waitFor(() => expect(netPendingIncoming).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unlisten.mock.calls.length).toBeGreaterThanOrEqual(2));
  });

  it("chat_screen_replaces_menu_and_footer_but_keeps_prompts", async () => {
    namingQueue.set([{ node_id: "peer-9", self_name: "Eve", nickname: "Eve" }]);
    render(MainScreen);
    showChat("peer-1");
    const back = await screen.findByRole("button", { name: "Back" });
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByText("p2p-messenger")).toBeNull();
    expect(screen.queryByRole("heading", { name: "Contacts" })).toBeNull();
    expect(screen.getByTestId("naming-prompt")).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(back));
    await fireEvent.click(back);
    expect(get(routerScreen)).toEqual({ kind: "tab", tab: "contacts" });
    expect(await screen.findByRole("tablist")).toBeTruthy();
  });

  it("chat_events_started_and_unlistened", async () => {
    const { unmount } = render(MainScreen);
    await waitFor(() => expect(startChatEvents).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unlistenChat).toHaveBeenCalled());
  });
});
