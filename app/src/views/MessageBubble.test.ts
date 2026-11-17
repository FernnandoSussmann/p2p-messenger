import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../lib/chatStore", () => ({ resendMessage: vi.fn() }));

import MessageBubble from "./MessageBubble.svelte";
import { resendMessage } from "../lib/chatStore";
import { formatLocalTime } from "../lib/time";
import type { MessageDto } from "../chat";

const mk = (o: Partial<MessageDto> = {}): MessageDto => ({
  node_id: "a".repeat(64), id: "m1", seq: 1, cursor: "c", direction: "out", body: "hello",
  sent_at_ms: 1_700_000_000_000, received_at_ms: null, delivered_at_ms: null, status: "delivered", ...o,
});
const show = (o: Partial<MessageDto> = {}) => render(MessageBubble, { message: mk(o) });

afterEach(cleanup);
beforeEach(() => vi.clearAllMocks());

describe("MessageBubble", () => {
  it("own_and_peer_alignment", () => {
    const { container, unmount } = show({ direction: "out" });
    expect(container.querySelector("li")).toHaveClass("own");
    unmount();
    const r = show({ direction: "in", status: "received" });
    expect(r.container.querySelector("li")).toHaveClass("peer");
    expect(r.container.querySelector(".status")).toBeNull();
  });

  it("shows_local_time_from_sent_at_ms", () => {
    const { container } = show();
    const t = container.querySelector("time")!;
    expect(t.textContent).toBe(formatLocalTime(1_700_000_000_000));
    expect(t.getAttribute("datetime")).toBe("2023-11-14T22:13:20.000Z");
  });

  it("status_labels_for_each_status", () => {
    const expected: Record<string, string> = {
      sending: "Sending…",
      delivered: "Delivered",
      pending: "Pending: will be sent when connected",
      failed: "Not delivered",
    };
    for (const [status, label] of Object.entries(expected)) {
      const { container, unmount } = show({ status: status as MessageDto["status"] });
      const el = container.querySelector(".status")!;
      expect(el.textContent).toBe(label);
      expect(el.getAttribute("data-status")).toBe(status);
      unmount();
    }
  });

  it("failed_shows_resend_and_calls_chatResend", async () => {
    vi.mocked(resendMessage).mockResolvedValue();
    show({ status: "failed" });
    await fireEvent.click(screen.getByRole("button", { name: "Resend" }));
    expect(resendMessage).toHaveBeenCalledWith("m1");
  });

  it("resend error is shown inline", async () => {
    vi.mocked(resendMessage).mockRejectedValue({ type: "OutboxFull" });
    show({ status: "failed" });
    await fireEvent.click(screen.getByRole("button", { name: "Resend" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Too many messages"));
  });

  it("no Resend unless failed", () => {
    show({ status: "pending" });
    expect(screen.queryByRole("button", { name: "Resend" })).toBeNull();
  });

  it("renders_html_and_markdown_as_inert_text", () => {
    const body = '<img src=x onerror=alert(1)><script>x</script><strong>b</strong> **md** [l](http://e.com)';
    const { container } = show({ body });
    for (const sel of ["img", "script", "strong", "a"]) expect(container.querySelector(sel)).toBeNull();
    expect(container.querySelector(".body")!.textContent).toBe(body);
  });

  it("urls_are_not_clickable", () => {
    const { container } = show({ body: "see https://example.com now" });
    expect(container.querySelector("a")).toBeNull();
    expect(container.querySelector("[href]")).toBeNull();
    expect(container.querySelector(".body")!.textContent).toBe("see https://example.com now");
  });

  it("body_is_isolated_from_metadata", () => {
    const { container } = show({ body: "x\u{FFFD}y" });
    const bdi = container.querySelector("bdi")!;
    expect(bdi.getAttribute("dir")).toBe("auto");
    expect(bdi.contains(container.querySelector("time"))).toBe(false);
    expect(bdi.contains(container.querySelector(".status"))).toBe(false);
    expect(bdi.textContent).toBe("x\u{FFFD}y");
  });

  it("multiline_emoji_and_zwnj_preserved", () => {
    const body = "hi 👋\nline 2\nمی‌خواهم";
    const { container } = show({ body });
    expect(container.querySelector(".body")!.textContent).toBe(body);
  });

  it("outgoing_shows_you_incoming_does_not", () => {
    const out = show({ direction: "out" });
    expect(out.container.querySelector(".who")!.textContent).toBe("You");
    out.unmount();
    const inn = show({ direction: "in", status: "received" });
    expect(inn.container.querySelector(".who")).toBeNull();
  });

  it("incoming_shows_sent_and_received_times", () => {
    const { container } = show({ direction: "in", status: "received", received_at_ms: 1_700_000_060_000 });
    const t = container.querySelectorAll("time");
    expect(t).toHaveLength(2);
    expect(t[0].getAttribute("title")).toBe("Sent (their clock)");
    expect(t[1].getAttribute("title")).toBe("Received (this device)");
    expect(t[1].textContent).toBe(formatLocalTime(1_700_000_060_000));
    expect(t[1].getAttribute("datetime")).toBe("2023-11-14T22:14:20.000Z");
    expect(container.querySelector(".stamp")?.textContent).toContain(" | ");
    expect(container.querySelector(".stamp [aria-hidden]")).toBeNull();
  });

  it("incoming_without_valid_received_time_shows_only_sent", () => {
    for (const received_at_ms of [null, 1e20]) {
      const { container, unmount } = show({ direction: "in", status: "received", received_at_ms });
      expect(container.querySelectorAll("time")).toHaveLength(1);
      unmount();
    }
  });
});
