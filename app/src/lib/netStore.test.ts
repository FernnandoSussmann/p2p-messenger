import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

type Handler = (e: { payload: unknown }) => void;
const handlers: Record<string, Handler> = {};
const unlisteners: ReturnType<typeof vi.fn>[] = [];

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, h: Handler) => {
    handlers[name] = h;
    const u = vi.fn();
    unlisteners.push(u);
    return u;
  }),
}));
vi.mock("../net", () => ({ netPendingIncoming: vi.fn() }));

import { startNetEvents, incomingRequests, connections } from "./netStore";
import { netPendingIncoming } from "../net";

const req = (id: string, name: string | null = "Eve") => ({ node_id: id, display_name: name });
const conn = (id: string, direction: "inbound" | "outbound", state: string, reason: string | null = null) => ({
  node_id: id,
  direction,
  state,
  reason,
});

beforeEach(() => {
  incomingRequests.set([]);
  connections.set({});
  unlisteners.length = 0;
  vi.mocked(netPendingIncoming).mockReset().mockResolvedValue([]);
});

describe("netStore", () => {
  it("dedups queued requests by node_id", async () => {
    await startNetEvents();
    handlers.incoming_request({ payload: req("a") });
    handlers.incoming_request({ payload: req("a") });
    handlers.incoming_request({ payload: req("b") });
    expect(get(incomingRequests).map((r) => r.node_id)).toEqual(["a", "b"]);
  });

  it("removes a prompt on connection_state", async () => {
    await startNetEvents();
    handlers.incoming_request({ payload: req("a") });
    handlers.connection_state({ payload: conn("a", "inbound", "connected") });
    expect(get(incomingRequests)).toEqual([]);
    expect(get(connections).a.state).toBe("connected");
  });

  it("calls unlisten for both listeners", async () => {
    const stop = await startNetEvents();
    stop();
    expect(unlisteners).toHaveLength(2);
    unlisteners.forEach((u) => expect(u).toHaveBeenCalledTimes(1));
  });

  it("seeds from the pending snapshot", async () => {
    vi.mocked(netPendingIncoming).mockResolvedValue([req("a"), req("b")]);
    await startNetEvents();
    expect(get(incomingRequests).map((r) => r.node_id)).toEqual(["a", "b"]);
  });

  it("shows a request present in both snapshot and event once", async () => {
    vi.mocked(netPendingIncoming).mockImplementation(async () => {
      handlers.incoming_request({ payload: req("a") });
      return [req("a")];
    });
    await startNetEvents();
    expect(get(incomingRequests)).toHaveLength(1);
  });

  it("tolerates a NotReady snapshot", async () => {
    vi.mocked(netPendingIncoming).mockRejectedValue({ type: "NotReady" });
    await expect(startNetEvents()).resolves.toBeTypeOf("function");
    expect(get(incomingRequests)).toEqual([]);
  });

  it("keeps no history for inbound failures but keeps outbound ones", async () => {
    await startNetEvents();
    handlers.connection_state({ payload: conn("a", "inbound", "connecting") });
    handlers.incoming_request({ payload: req("a") });
    handlers.connection_state({ payload: conn("a", "inbound", "failed", "declined") });
    expect(get(incomingRequests)).toEqual([]);
    expect(get(connections).a).toBeUndefined();
    handlers.connection_state({ payload: conn("b", "outbound", "failed", "declined") });
    expect(get(connections).b.reason).toBe("declined");
  });
});
