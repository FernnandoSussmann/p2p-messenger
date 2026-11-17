import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { get } from "svelte/store";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../chat", async (orig) => ({
  ...(await orig<typeof import("../chat")>()),
  chatHistory: vi.fn(),
  chatSend: vi.fn(),
  chatResend: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));

import Chat from "./Chat.svelte";
import { chatHistory, type MessageDto } from "../chat";
import { contacts } from "../lib/contactsStore";
import { connections } from "../lib/netStore";
import { closeChat, openChat } from "../lib/chatStore";
import { resetScreen, screen as screenStore, showChat } from "../lib/router";

const ID = "a".repeat(64);
const mk = (i: number, o: Partial<MessageDto> = {}): MessageDto => ({
  node_id: ID, id: `m${i}`, seq: i, cursor: `c${i}`, direction: "in", body: `body ${i}`,
  sent_at_ms: 1_700_000_000_000 + i * 1000, received_at_ms: null, delivered_at_ms: null, status: "received", ...o,
});
const contact = { node_id: ID, nickname: "Bob", self_name: null, verified: false, added_at_ms: 0, last_seen_ms: null };
const onBack = vi.fn();

async function show(messages: MessageDto[], next: string | null = null) {
  vi.mocked(chatHistory).mockResolvedValue({ messages, next_before: next });
  render(Chat, { nodeId: ID, onBack });
  await openChat(ID);
}

afterEach(() => {
  cleanup();
  closeChat();
});
beforeEach(() => {
  vi.clearAllMocks();
  resetScreen();
  contacts.set([contact]);
  connections.set({ [ID]: { node_id: ID, state: "connected", direction: "outbound" } as never });
});

describe("Chat", () => {
  it("renders_messages_newest_at_bottom", async () => {
    await show([mk(1), mk(2, { direction: "out", status: "delivered" }), mk(3)]);
    const items = await screen.findAllByTestId("message");
    expect(items.map((i) => i.querySelector(".body")!.textContent)).toEqual(["body 1", "body 2", "body 3"]);
    expect(screen.getByRole("log")).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
    expect(screen.getByText("Bob").closest(".conv-header")).not.toBeNull();
    expect(screen.getByTestId("badge-unverified")).toBeTruthy();
  });

  it("load_older_button_visible_while_cursor", async () => {
    await show([mk(2)], "tok");
    const btn = await screen.findByRole("button", { name: "Load older messages" });
    vi.mocked(chatHistory).mockResolvedValue({ messages: [mk(1)], next_before: null });
    await fireEvent.click(btn);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Load older messages" })).toBeNull());
    expect(screen.getAllByTestId("message")).toHaveLength(2);
  });

  it("not_connected_note_explains_reconnect", async () => {
    connections.set({});
    await show([]);
    expect(screen.getByText("Not connected")).toBeTruthy();
    expect(screen.getByRole("status").textContent?.replace(/\s+/g, " ")).toContain("Connect in their contact info or paste their ticket in Network.");
  });

  it("removed_contact_renders_nothing_and_goes_to_contacts", async () => {
    showChat(ID);
    await show([]);
    contacts.set([]);
    await waitFor(() => expect(get(screenStore)).toEqual({ kind: "tab", tab: "contacts" }));
    expect(screen.queryByRole("region", { name: "Conversation" })).toBeNull();
    expect(screen.queryByText("This contact was removed.")).toBeNull();
    expect(screen.queryByLabelText("Message")).toBeNull();
  });

  it("contact_removed_while_open_goes_to_contacts", async () => {
    showChat(ID);
    await show([]);
    expect(get(screenStore).kind).toBe("chat");
    contacts.set([{ ...contact, node_id: "b".repeat(64) }]);
    await waitFor(() => expect(get(screenStore)).toEqual({ kind: "tab", tab: "contacts" }));
  });

  it("header_has_contact_info_button", async () => {
    await show([]);
    await fireEvent.click(screen.getByRole("button", { name: "Contact info" }));
    expect(get(screenStore)).toEqual({ kind: "contact-info", nodeId: ID });
    expect(screen.queryByLabelText("Search this conversation")).toBeNull();
    expect(screen.queryByRole("button", { name: "Clear history…" })).toBeNull();
  });

  it("back_calls_onBack", async () => {
    await show([]);
    await fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalled();
  });
});
