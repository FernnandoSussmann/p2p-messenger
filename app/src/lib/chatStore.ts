import { get, writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import {
  chatHistory,
  chatResend,
  chatSend,
  describeChatError,
  type MessageDto,
  type MessageStatus,
  type MessageStatusDto,
} from "../chat";

export interface Conversation {
  nodeId: string;
  messages: MessageDto[];
  nextBefore: string | null;
  loading: boolean;
  error: string | null;
}

const MAX_LOADED = 1000;
const MAX_PENDING_STATUS = 256;

export const activeChat = writable<string | null>(null);
export const conversation = writable<Conversation | null>(null);
export const unread = writable<Record<string, number>>({});

const pendingStatus = new Map<string, MessageStatus>();
let inflight = 0;

const compare = (a: MessageDto, b: MessageDto) => a.sent_at_ms - b.sent_at_ms || a.seq - b.seq;

function mergeStatus(current: MessageStatus, incoming: MessageStatus): MessageStatus {
  return current === "delivered" || incoming === "delivered" ? "delivered" : incoming;
}

function rememberStatus(id: string, status: MessageStatus) {
  pendingStatus.delete(id);
  pendingStatus.set(id, status);
  while (pendingStatus.size > MAX_PENDING_STATUS) {
    pendingStatus.delete(pendingStatus.keys().next().value as string);
  }
}

function withPending(dto: MessageDto, existing?: MessageDto): MessageDto {
  let status = existing ? mergeStatus(existing.status, dto.status) : dto.status;
  const pending = pendingStatus.get(dto.id);
  if (pending !== undefined) {
    status = mergeStatus(status, pending);
    pendingStatus.delete(dto.id);
  }
  return { ...dto, status };
}

function capped(c: Conversation): Conversation {
  if (c.messages.length <= MAX_LOADED) return c;
  const messages = c.messages.slice(c.messages.length - MAX_LOADED);
  return { ...c, messages, nextBefore: messages[0].cursor };
}

function upsert(c: Conversation, dto: MessageDto, skipBeforeOldest: boolean): Conversation {
  const idx = c.messages.findIndex((m) => m.id === dto.id);
  if (idx >= 0) {
    const messages = [...c.messages];
    messages[idx] = withPending(dto, c.messages[idx]);
    return { ...c, messages };
  }
  if (skipBeforeOldest && c.nextBefore !== null && c.messages.length > 0 && compare(dto, c.messages[0]) < 0) {
    return c;
  }
  const next = withPending(dto);
  const messages = [...c.messages];
  let at = messages.length;
  while (at > 0 && compare(next, messages[at - 1]) < 0) at--;
  messages.splice(at, 0, next);
  return capped({ ...c, messages });
}

function updateOpen(nodeId: string, fn: (c: Conversation) => Conversation) {
  conversation.update((c) => (c && c.nodeId === nodeId ? fn(c) : c));
}

export async function openConversation(nodeId: string): Promise<void> {
  unread.update((u) => ({ ...u, [nodeId]: 0 }));
  conversation.set({ nodeId, messages: [], nextBefore: null, loading: true, error: null });
  inflight++;
  try {
    const page = await chatHistory(nodeId, null, null);
    updateOpen(nodeId, (c) => {
      const messages = page.messages.map((m) => withPending(m)).sort(compare);
      return capped({ ...c, messages, nextBefore: page.next_before, loading: false });
    });
  } catch (e) {
    updateOpen(nodeId, (c) => ({ ...c, loading: false, error: describeChatError(e) }));
  } finally {
    inflight--;
  }
}

export function openChat(nodeId: string): Promise<void> {
  activeChat.set(nodeId);
  return openConversation(nodeId);
}

export function closeChat() {
  activeChat.set(null);
  conversation.set(null);
}

export async function loadOlder(): Promise<void> {
  const cur = get(conversation);
  if (!cur || cur.loading || cur.nextBefore === null) return;
  const { nodeId } = cur;
  updateOpen(nodeId, (c) => ({ ...c, loading: true, error: null }));
  inflight++;
  try {
    const page = await chatHistory(nodeId, cur.nextBefore, null);
    updateOpen(nodeId, (c) => {
      const known = new Set(c.messages.map((m) => m.id));
      const older = page.messages.filter((m) => !known.has(m.id)).map((m) => withPending(m));
      return { ...c, messages: [...older, ...c.messages].sort(compare), nextBefore: page.next_before, loading: false };
    });
  } catch (e) {
    updateOpen(nodeId, (c) => ({ ...c, loading: false, error: describeChatError(e) }));
  } finally {
    inflight--;
  }
}

export async function sendMessage(body: string): Promise<void> {
  const cur = get(conversation);
  if (!cur) return;
  const dto = await chatSend(cur.nodeId, body);
  updateOpen(cur.nodeId, (c) => upsert(c, dto, false));
}

export async function resendMessage(id: string): Promise<void> {
  const cur = get(conversation);
  if (!cur) return;
  const res = await chatResend(cur.nodeId, id);
  applyStatus(res);
}

function applyStatus(ev: MessageStatusDto) {
  const cur = get(conversation);
  const idx = cur && cur.nodeId === ev.node_id ? cur.messages.findIndex((m) => m.id === ev.id) : -1;
  if (idx < 0) {
    rememberStatus(ev.id, ev.status);
    return;
  }
  if (inflight > 0) rememberStatus(ev.id, ev.status);
  updateOpen(ev.node_id, (c) => {
    const messages = [...c.messages];
    messages[idx] = { ...messages[idx], status: mergeStatus(messages[idx].status, ev.status) };
    return { ...c, messages };
  });
}

function onReceived(dto: MessageDto) {
  const cur = get(conversation);
  if (cur && cur.nodeId === dto.node_id) {
    updateOpen(dto.node_id, (c) => upsert(c, dto, true));
  } else {
    unread.update((u) => ({ ...u, [dto.node_id]: (u[dto.node_id] ?? 0) + 1 }));
  }
}

async function resync(): Promise<void> {
  const cur = get(conversation);
  if (!cur) return;
  const { nodeId } = cur;
  inflight++;
  try {
    const page = await chatHistory(nodeId, null, null);
    updateOpen(nodeId, (c) => {
      const byId = new Map(c.messages.map((m) => [m.id, m]));
      const fresh = page.messages.map((m) => withPending(m, byId.get(m.id))).sort(compare);
      const oldest = fresh[0];
      const kept = oldest ? c.messages.filter((m) => compare(m, oldest) < 0) : [];
      return capped({
        ...c,
        messages: [...kept, ...fresh],
        nextBefore: kept.length > 0 ? c.nextBefore : page.next_before,
        error: null,
      });
    });
  } catch (e) {
    updateOpen(nodeId, (c) => ({ ...c, error: describeChatError(e) }));
  } finally {
    inflight--;
  }
}

export async function startChatEvents(): Promise<() => void> {
  const unlisteners = [
    await listen<MessageDto>("message_received", (e) => onReceived(e.payload)),
    await listen<MessageStatusDto>("message_status", (e) => applyStatus(e.payload)),
    await listen<null>("chat_resync", () => void resync()),
  ];
  return () => unlisteners.forEach((u) => u());
}
