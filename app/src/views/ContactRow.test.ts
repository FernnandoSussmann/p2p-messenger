import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../lib/chatStore", async () => {
  const { writable } = await import("svelte/store");
  return { openChat: vi.fn(), unread: writable<Record<string, number>>({}) };
});

import ContactRow from "./ContactRow.svelte";
import type { ContactDto } from "../contacts";
import { connections } from "../lib/netStore";
import { openChat, unread } from "../lib/chatStore";

const ID = "a".repeat(64);
const mk = (o: Partial<ContactDto> = {}): ContactDto => ({
  node_id: ID,
  nickname: "Bob",
  self_name: null,
  verified: false,
  added_at_ms: 0,
  last_seen_ms: null,
  ...o,
});
const show = (o: Partial<ContactDto> = {}) => render(ContactRow, { contact: mk(o) });

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  connections.set({});
  unread.set({});
});

describe("ContactRow", () => {
  it("is one button containing badge, nickname and connection text", () => {
    show();
    const row = screen.getByTestId("contact-row");
    expect(row.tagName).toBe("BUTTON");
    expect(row).toContainElement(screen.getByTestId("badge-unverified"));
    expect(row).toContainElement(screen.getByTestId("contact-nickname"));
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });

  it("has no action buttons or NodeId line", () => {
    show();
    for (const n of [/Message/, "Connect", "Rename", /Mark verified/, "Mark unverified", /Remove/]) {
      expect(screen.queryByRole("button", { name: n })).toBeNull();
    }
    expect(screen.queryByTestId("node-id-short")).toBeNull();
  });

  it("click opens the conversation", async () => {
    show();
    await fireEvent.click(screen.getByTestId("contact-row"));
    expect(openChat).toHaveBeenCalledWith(ID);
  });

  it("does not render the self-asserted name", () => {
    const { container } = show({ self_name: "Mallory" });
    expect(container.textContent).not.toContain("Mallory");
  });

  it("verified contact shows the Verified badge", () => {
    show({ verified: true });
    expect(screen.getByTestId("badge-verified")).toBeTruthy();
  });

  it("badge cannot be spoofed by the nickname", () => {
    const names = ["Verified by you", "✓ Verified", '<span data-testid="badge-verified">x</span>', "n".repeat(128)];
    for (const nickname of names) {
      const { unmount } = show({ nickname, verified: false });
      expect(screen.getAllByTestId("badge-unverified")).toHaveLength(1);
      expect(screen.queryByTestId("badge-verified")).toBeNull();
      unmount();
    }
  });

  it("non-boolean verified renders Unverified", () => {
    show({ verified: "true" as unknown as boolean });
    expect(screen.getByTestId("badge-unverified")).toBeTruthy();
    expect(screen.queryByTestId("badge-verified")).toBeNull();
  });

  it("names render inert, inside bdi", () => {
    const { container } = show({ nickname: "<img src=x onerror=alert(1)>" });
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByTestId("contact-nickname").querySelector("bdi")).not.toBeNull();
  });

  it("shows the connection line from the store, or Not connected", async () => {
    show();
    expect(screen.getByTestId("contact-connection")).toHaveTextContent("Not connected");
    connections.set({ [ID]: { node_id: ID, state: "connected", direction: "outbound", reason: null } as never });
    await waitFor(() => expect(screen.getByTestId("contact-connection")).toHaveTextContent(/^outbound \| connected$/));
    connections.set({ [ID]: { node_id: ID, state: "failed", direction: "inbound", reason: "timeout" } as never });
    await waitFor(() => expect(screen.getByTestId("contact-connection")).toHaveTextContent(/^inbound \| failed \| timeout$/));
  });

  it("shows the unread chip only when unread > 0", async () => {
    show();
    expect(screen.queryByText(/unread/)).toBeNull();
    unread.set({ [ID]: 2 });
    expect(await screen.findByText("2 unread")).toBeTruthy();
    expect(screen.getByRole("button", { name: /Bob/ })).toHaveTextContent("2 unread");
  });
});
