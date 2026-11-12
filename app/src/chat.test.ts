import { describe, it, expect, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { describeChatError, historyClear, utf8Length, type ChatCmdError } from "./chat";

const ALL: ChatCmdError["type"][] = [
  "NotReady", "StoreUnavailable", "InvalidNodeId", "InvalidMessageId", "InvalidCursor", "SelfMessage",
  "NotAContact", "EmptyBody", "BodyTooLarge", "OutboxFull", "NotFound", "NotResendable", "InvalidQuery",
  "ContactsUnavailable", "NotConfirmed", "HistoryClearFailed", "Internal",
];

describe("chat", () => {
  it("describeChatError has a distinct sentence for every error type", () => {
    const texts = ALL.map((type) => describeChatError({ type }));
    texts.forEach((t) => expect(t.length).toBeGreaterThan(5));
    expect(new Set(texts).size).toBe(ALL.length);
    expect(describeChatError({ type: "SelfMessage" })).toBe("You can't message yourself.");
    expect(describeChatError(undefined)).toBe(describeChatError({ type: "Internal" }));
  });

  it("historyClear sends confirm true", async () => {
    invoke.mockResolvedValue(undefined);
    await historyClear("abc");
    expect(invoke).toHaveBeenCalledWith("history_clear", { nodeId: "abc", confirm: true });
  });

  it("utf8Length counts an emoji as 4 bytes", () => {
    expect(utf8Length("👋")).toBe(4);
    expect(utf8Length("abc")).toBe(3);
  });
});
