import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../contacts", async (orig) => ({
  ...(await orig<typeof import("../contacts")>()),
  contactsRename: vi.fn(),
  contactsSetVerified: vi.fn(),
  contactsRemove: vi.fn(),
}));
vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netConnectTicket: vi.fn(),
}));
vi.mock("../lib/chatStore", async () => {
  const { writable } = await import("svelte/store");
  return { openChat: vi.fn(), unread: writable<Record<string, number>>({}) };
});
vi.mock("../lib/contactsStore", () => ({ refreshContacts: vi.fn(async () => {}) }));

import ContactRow from "./ContactRow.svelte";
import { contactsRename, contactsSetVerified, type ContactDto } from "../contacts";
import { netConnectTicket } from "../net";
import { lanPeers, connections } from "../lib/netStore";
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
const onNotice = vi.fn();
const show = (o: Partial<ContactDto> = {}) => render(ContactRow, { contact: mk(o), onNotice });

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  lanPeers.set([]);
  connections.set({});
  unread.set({});
});

describe("ContactRow", () => {
  it("renders the nickname in a strong element followed by the short NodeId", () => {
    show();
    const nick = screen.getByTestId("contact-nickname");
    expect(nick.tagName).toBe("STRONG");
    const id = screen.getByTestId("node-id-short");
    expect(nick.compareDocumentPosition(id) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(id).toHaveClass("node-id");
    expect(id).toHaveAttribute("title", ID);
  });

  it("does not render the self-asserted name in the row", () => {
    const { container } = show({ self_name: "Mallory" });
    expect(container.textContent).not.toContain("Mallory");
    expect(container.textContent).not.toContain("calls themselves");
  });

  it("unverified contact shows the Unverified badge", () => {
    show();
    expect(screen.getByTestId("badge-unverified")).toBeTruthy();
  });

  it("badge cannot be spoofed by the nickname", () => {
    const names = ["Verified by you", "✓ Verified", '<span data-testid="badge-verified">x</span>', "n".repeat(128), "a​b"];
    for (const nickname of names) {
      const { container, unmount } = show({ nickname, verified: false });
      expect(screen.getAllByTestId("badge-unverified")).toHaveLength(1);
      expect(screen.queryByTestId("badge-verified")).toBeNull();
      expect(container.querySelector(".title")!.firstElementChild).toBe(screen.getByTestId("badge-unverified"));
      unmount();
    }
  });

  it("non-boolean verified renders Unverified", () => {
    show({ verified: "true" as unknown as boolean });
    expect(screen.getByTestId("badge-unverified")).toBeTruthy();
    expect(screen.queryByTestId("badge-verified")).toBeNull();
  });

  it('verified contact shows "Verified by you" and Mark unverified', () => {
    show({ verified: true });
    expect(screen.getByTestId("badge-verified")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Mark unverified" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Mark verified/ })).toBeNull();
  });

  it("names render inert", () => {
    const { container } = show({ nickname: "<img src=x onerror=alert(1)>" });
    expect(container.querySelector("img")).toBeNull();
  });

  it("Connect appears only when the contact is on the LAN and not connected, and dials its LAN ticket", async () => {
    show();
    expect(screen.queryByRole("button", { name: "Connect" })).toBeNull();
    lanPeers.set([{ node_id: ID, display_name: null, ticket: "t-id", verified: true } as never]);
    const btn = await screen.findByRole("button", { name: "Connect" });
    await fireEvent.click(btn);
    expect(netConnectTicket).toHaveBeenCalledWith("t-id");
    connections.set({ [ID]: { node_id: ID, state: "connected", direction: "outbound" } as never });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Connect" })).toBeNull());
  });

  it("rename sends the new nickname", async () => {
    vi.mocked(contactsRename).mockResolvedValue(mk({ nickname: "Robert" }));
    show({ self_name: "Mallory" });
    await fireEvent.click(screen.getByRole("button", { name: "Rename" }));
    expect(screen.getByTestId("peer-name").textContent).toBe("Mallory");
    const input = screen.getByLabelText("Your name for them");
    await fireEvent.input(input, { target: { value: "Robert" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save nickname" }));
    await waitFor(() => expect(contactsRename).toHaveBeenCalledWith(ID, "Robert"));
  });

  it("rename shows InvalidNickname message", async () => {
    vi.mocked(contactsRename).mockRejectedValue({ type: "InvalidNickname", kind: "Empty" });
    show();
    await fireEvent.click(screen.getByRole("button", { name: "Rename" }));
    await fireEvent.click(screen.getByRole("button", { name: "Save nickname" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
  });

  it("Mark unverified calls set_verified(false) without a dialog", async () => {
    vi.mocked(contactsSetVerified).mockResolvedValue(mk());
    show({ verified: true });
    await fireEvent.click(screen.getByRole("button", { name: "Mark unverified" }));
    await waitFor(() => expect(contactsSetVerified).toHaveBeenCalledWith(ID, false));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("message_button_opens_chat", async () => {
    show();
    await fireEvent.click(screen.getByRole("button", { name: "Message" }));
    expect(openChat).toHaveBeenCalledWith(ID);
  });

  it("message_button_shows_unread", () => {
    unread.set({ [ID]: 2 });
    show();
    expect(screen.getByRole("button", { name: "Message (2)" })).toBeTruthy();
  });
});
