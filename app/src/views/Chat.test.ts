import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../chat", async (orig) => ({
  ...(await orig<typeof import("../chat")>()),
  chatHistory: vi.fn(),
  chatSend: vi.fn(),
  chatResend: vi.fn(),
  historySearch: vi.fn(),
  historyClear: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));

import Chat from "./Chat.svelte";
import { chatHistory, historyClear, historySearch, type MessageDto } from "../chat";
import { contacts } from "../lib/contactsStore";
import { connections } from "../lib/netStore";
import { closeChat, openChat } from "../lib/chatStore";

const ID = "a".repeat(64);
const mk = (i: number, o: Partial<MessageDto> = {}): MessageDto => ({
  node_id: ID, id: `m${i}`, seq: i, cursor: `c${i}`, direction: "in", body: `body ${i}`,
  sent_at_ms: 1_700_000_000_000 + i * 1000, received_at_ms: null, status: "received", ...o,
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
    expect(screen.getByRole("status").textContent?.replace(/\s+/g, " ")).toContain("sent automatically once you are connected again");
  });

  it("removed_contact_disables_composer", async () => {
    contacts.set([]);
    await show([]);
    expect(screen.getByRole("status").textContent).toBe("This contact was removed.");
    expect(screen.getByLabelText("Message")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  });

  it("search_shows_results_as_text", async () => {
    await show([]);
    const hit = mk(9, { body: "<b>needle</b>" });
    vi.mocked(historySearch).mockResolvedValue([hit]);
    const box = screen.getByLabelText("Search this conversation");
    await fireEvent.input(box, { target: { value: "needle" } });
    const list = await screen.findByTestId("search-results");
    expect(historySearch).toHaveBeenCalledWith("needle", ID);
    expect(list.querySelector("b")).toBeNull();
    expect(list.textContent).toContain("<b>needle</b>");
    await fireEvent.keyDown(box, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("search-results")).toBeNull());
    expect((box as HTMLInputElement).value).toBe("");
  });

  it("back_calls_onBack", async () => {
    await show([]);
    await fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalled();
  });

  for (const [label, known] of [["contacts", true], ["removed contacts", false]] as const) {
    it(`clear_history_is_reachable_for_${label.replace(" ", "_")}`, async () => {
      if (!known) contacts.set([]);
      await show([mk(1)]);
      expect(screen.getByRole("button", { name: "Clear history…" })).toBeEnabled();
    });
  }

  it("clear_history_empties_log_and_search", async () => {
    vi.mocked(historyClear).mockResolvedValue(undefined);
    await show([mk(1), mk(2)]);
    vi.mocked(historySearch).mockResolvedValue([mk(1)]);
    await fireEvent.input(screen.getByLabelText("Search this conversation"), { target: { value: "body" } });
    await screen.findByTestId("search-results");
    const open = screen.getByRole("button", { name: "Clear history…" });
    await fireEvent.click(open);
    expect(open).toBeDisabled();
    expect(screen.getByRole("dialog", { name: "Clear history" }).textContent).toContain("Bob");
    await fireEvent.click(screen.getByRole("button", { name: "Clear history" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("History cleared."));
    expect(historyClear).toHaveBeenCalledWith(ID);
    expect(screen.queryAllByTestId("message")).toHaveLength(0);
    expect(screen.queryByTestId("search-results")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
