import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

const handlers = vi.hoisted(() => ({} as Record<string, (e: { payload: unknown }) => void>));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return vi.fn();
  }),
}));
vi.mock("../chat", async (orig) => ({
  ...(await orig<typeof import("../chat")>()),
  chatHistory: vi.fn(),
  chatSend: vi.fn(),
  chatResend: vi.fn(),
}));

import { chatHistory, chatSend, chatResend, type MessageDto } from "../chat";
import {
  closeChat, conversation, loadOlder, openChat, resendMessage, resetConversation, sendMessage, startChatEvents, unread,
} from "./chatStore";
import { screen, showTab } from "./router";

const A = "a".repeat(64);
const B = "b".repeat(64);
let n = 0;
const mk = (o: Partial<MessageDto> = {}): MessageDto => {
  n++;
  return {
    node_id: A, id: `id${n}`, seq: n, cursor: `c${n}`, direction: "in", body: `m${n}`,
    sent_at_ms: 1000 + n, received_at_ms: 1000 + n, delivered_at_ms: null, status: "received", ...o,
  };
};
const ids = () => get(conversation)!.messages.map((m) => m.id);
const status = (id: string) => get(conversation)!.messages.find((m) => m.id === id)!.status;
const emit = (name: string, payload: unknown) => handlers[name]({ payload });
const page = (messages: MessageDto[], next: string | null = null) =>
  vi.mocked(chatHistory).mockResolvedValueOnce({ messages, next_before: next });

beforeEach(async () => {
  vi.resetAllMocks();
  closeChat();
  unread.set({});
  n = 0;
  await startChatEvents();
});

describe("chatStore", () => {
  it("open_loads_newest_page_and_resets_unread", async () => {
    unread.set({ [A]: 3 });
    const m = [mk(), mk()];
    page(m, "tok");
    await openChat(A);
    expect(chatHistory).toHaveBeenCalledWith(A, null, null);
    expect(ids()).toEqual(m.map((x) => x.id));
    expect(get(conversation)!.nextBefore).toBe("tok");
    expect(get(unread)[A]).toBe(0);
  });

  it("load_older_prepends_using_next_before_token", async () => {
    const newer = [mk({ sent_at_ms: 5000 })];
    page(newer, "tok");
    await openChat(A);
    const older = [mk({ sent_at_ms: 100 })];
    page(older, null);
    await loadOlder();
    expect(chatHistory).toHaveBeenLastCalledWith(A, "tok", null);
    expect(ids()).toEqual([older[0].id, newer[0].id]);
    expect(get(conversation)!.nextBefore).toBeNull();
  });

  it("received_message_is_inserted_in_sorted_position", async () => {
    const [x, y] = [mk({ sent_at_ms: 100 }), mk({ sent_at_ms: 300 })];
    page([x, y], "tok");
    await openChat(A);
    const mid = mk({ sent_at_ms: 200 });
    emit("message_received", mid);
    expect(ids()).toEqual([x.id, mid.id, y.id]);
    emit("message_received", mk({ sent_at_ms: 50 }));
    expect(ids()).toHaveLength(3);
  });

  it("received_event_for_other_peer_increments_unread", async () => {
    page([]);
    await openChat(A);
    emit("message_received", mk({ node_id: B }));
    emit("message_received", mk({ node_id: B }));
    expect(get(unread)[B]).toBe(2);
    expect(ids()).toEqual([]);
  });

  it("received_event_for_known_id_is_not_duplicated", async () => {
    const x = mk();
    page([x]);
    await openChat(A);
    emit("message_received", x);
    expect(ids()).toEqual([x.id]);
  });

  it("status_event_updates_message", async () => {
    const x = mk({ direction: "out", status: "sending", delivered_at_ms: null });
    page([x]);
    await openChat(A);
    emit("message_status", { node_id: A, id: x.id, status: "pending" });
    expect(status(x.id)).toBe("pending");
    emit("message_status", { node_id: A, id: x.id, status: "failed" });
    expect(status(x.id)).toBe("failed");
  });

  it("status_event_before_send_reply_is_applied", async () => {
    page([]);
    await openChat(A);
    const sent = mk({ direction: "out", status: "sending", delivered_at_ms: null });
    let resolve!: (m: MessageDto) => void;
    vi.mocked(chatSend).mockReturnValueOnce(new Promise((r) => (resolve = r)));
    const p = sendMessage("hi");
    emit("message_status", { node_id: A, id: sent.id, status: "delivered" });
    resolve(sent);
    await p;
    expect(status(sent.id)).toBe("delivered");
  });

  it("page_load_never_downgrades_delivered", async () => {
    const x = mk({ direction: "out", status: "sending", delivered_at_ms: null });
    page([x]);
    await openChat(A);
    emit("message_status", { node_id: A, id: x.id, status: "delivered" });
    page([{ ...x, status: "pending" }]);
    emit("chat_resync", null);
    await vi.waitFor(() => expect(chatHistory).toHaveBeenCalledTimes(2));
    await Promise.resolve();
    await Promise.resolve();
    expect(status(x.id)).toBe("delivered");
  });

  it("status_event_for_unloaded_id_is_applied_on_page_load", async () => {
    const x = mk({ direction: "out", status: "pending" });
    page([]);
    await openChat(A);
    emit("message_status", { node_id: A, id: x.id, status: "delivered" });
    closeChat();
    page([x]);
    await openChat(A);
    expect(status(x.id)).toBe("delivered");
  });

  it("pending_status_map_is_bounded", async () => {
    const first = mk({ direction: "out", status: "pending" });
    const last = mk({ direction: "out", status: "pending" });
    page([]);
    await openChat(A);
    emit("message_status", { node_id: A, id: first.id, status: "failed" });
    for (let i = 0; i < 300; i++) emit("message_status", { node_id: A, id: `x${i}`, status: "failed" });
    emit("message_status", { node_id: A, id: last.id, status: "failed" });
    closeChat();
    page([first, last]);
    await openChat(A);
    expect(status(first.id)).toBe("pending");
    expect(status(last.id)).toBe("failed");
  });

  it("resync_reloads_open_conversation", async () => {
    const x = mk();
    page([x]);
    await openChat(A);
    const y = mk();
    page([x, y]);
    emit("chat_resync", null);
    await vi.waitFor(() => expect(ids()).toEqual([x.id, y.id]));
  });

  it("loaded_list_is_capped", async () => {
    const first = Array.from({ length: 1000 }, () => mk());
    page(first);
    await openChat(A);
    const extra = mk();
    emit("message_received", extra);
    const c = get(conversation)!;
    expect(c.messages).toHaveLength(1000);
    expect(c.messages[999].id).toBe(extra.id);
    expect(c.messages[0].id).toBe(first[1].id);
    expect(c.nextBefore).toBe(first[1].cursor);
  });

  it("open_and_close_chat_drive_the_screen", async () => {
    page([]);
    await openChat(A);
    expect(get(screen)).toEqual({ kind: "chat", nodeId: A });
    closeChat();
    expect(get(screen)).toEqual({ kind: "tab", tab: "contacts" });
    expect(get(conversation)).toBeNull();
  });

  it("showing_a_tab_clears_the_open_conversation", async () => {
    page([]);
    await openChat(A);
    expect(get(conversation)).not.toBeNull();
    showTab("network");
    expect(get(conversation)).toBeNull();
  });

  it("resend applies the returned status", async () => {
    const x = mk({ direction: "out", status: "failed" });
    page([x]);
    await openChat(A);
    vi.mocked(chatResend).mockResolvedValue({ node_id: A, id: x.id, status: "sending", delivered_at_ms: null });
    await resendMessage(x.id);
    expect(chatResend).toHaveBeenCalledWith(A, x.id);
    expect(status(x.id)).toBe("sending");
  });

  it("resetConversation empties only that peer and clears its unread count", async () => {
    page([mk(), mk()], "tok");
    await openChat(A);
    unread.set({ [A]: 2, [B]: 3 });
    resetConversation(B);
    expect(get(conversation)!.messages).toHaveLength(2);
    expect(get(unread)).toEqual({ [A]: 2, [B]: 0 });
    resetConversation(A);
    const c = get(conversation)!;
    expect([c.messages, c.nextBefore, c.error]).toEqual([[], null, null]);
    expect(get(unread)[A]).toBe(0);
  });
});
