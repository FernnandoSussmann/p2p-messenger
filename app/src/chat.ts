import { invoke } from "@tauri-apps/api/core";

export type MessageStatus = "sending" | "pending" | "delivered" | "failed" | "received";

export interface MessageDto {
  node_id: string;
  id: string;
  seq: number;
  cursor: string;
  direction: "in" | "out";
  body: string;
  sent_at_ms: number;
  received_at_ms: number | null;
  status: MessageStatus;
}

export interface MessageStatusDto {
  node_id: string;
  id: string;
  status: MessageStatus;
}

export interface HistoryPageDto {
  messages: MessageDto[];
  next_before: string | null;
}

export type ChatCmdError =
  | { type: "NotReady" }
  | { type: "StoreUnavailable" }
  | { type: "InvalidNodeId" }
  | { type: "InvalidMessageId" }
  | { type: "InvalidCursor" }
  | { type: "SelfMessage" }
  | { type: "NotAContact" }
  | { type: "EmptyBody" }
  | { type: "BodyTooLarge" }
  | { type: "OutboxFull" }
  | { type: "NotFound" }
  | { type: "NotResendable" }
  | { type: "InvalidQuery" }
  | { type: "ContactsUnavailable" }
  | { type: "NotConfirmed" }
  | { type: "HistoryClearFailed" }
  | { type: "Internal" };

export const MAX_BODY_BYTES = 16384;

export const utf8Length = (s: string) => new TextEncoder().encode(s).length;

export const chatSend = (nodeId: string, body: string) =>
  invoke<MessageDto>("chat_send", { nodeId, body });

export const chatResend = (nodeId: string, messageId: string) =>
  invoke<MessageStatusDto>("chat_resend", { nodeId, messageId });

export const chatHistory = (nodeId: string, before: string | null, limit: number | null = null) =>
  invoke<HistoryPageDto>("chat_history", { nodeId, before, limit });

export const historySearch = (query: string, nodeId: string | null, limit: number | null = null) =>
  invoke<MessageDto[]>("history_search", { query, nodeId, limit });

export const historyClear = (nodeId: string) => invoke<void>("history_clear", { nodeId, confirm: true });

const messages: Record<ChatCmdError["type"], string> = {
  NotReady: "Messaging is not ready yet. Try again in a moment.",
  StoreUnavailable: "Message history is unavailable right now.",
  InvalidNodeId: "That is not a valid NodeId.",
  InvalidMessageId: "That message could not be found.",
  InvalidCursor: "Could not load older messages. Reopen the conversation and try again.",
  SelfMessage: "You can't message yourself.",
  NotAContact: "You can only message your contacts.",
  EmptyBody: "Write a message first.",
  BodyTooLarge: "That message is too long.",
  OutboxFull: "Too many messages are waiting to be sent. Try again once some are delivered.",
  NotFound: "That message no longer exists.",
  NotResendable: "That message can't be resent.",
  InvalidQuery: "That search is not valid.",
  ContactsUnavailable: "Contacts are unavailable right now.",
  NotConfirmed: "Clearing history needs your confirmation.",
  HistoryClearFailed: "History could not be cleared. Nothing was sent. Try again.",
  Internal: "Something went wrong. Try again.",
};

export function describeChatError(err: unknown): string {
  const type = (err as { type?: string } | undefined)?.type;
  if (type && Object.prototype.hasOwnProperty.call(messages, type)) {
    return messages[type as ChatCmdError["type"]];
  }
  return messages.Internal;
}
