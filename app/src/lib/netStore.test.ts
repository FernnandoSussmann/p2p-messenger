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
vi.mock("../net", () => ({ netPendingIncoming: vi.fn(), netLanPeers: vi.fn() }));

import { startNetEvents, incomingRequests, connections, lanPeers } from "./netStore";
import { netPendingIncoming, netLanPeers } from "../net";

const req = (id: string, name: string | null = "Eve") => ({
  node_id: id,
  display_name: name,
  suggested_nickname: name ?? "Peer abc",
});
const conn = (id: string, direction: "inbound" | "outbound", state: string, reason: string | null = null) => ({
  node_id: id,
  direction,
  state,
  reason,
  display_name: null,
});

beforeEach(() => {
  incomingRequests.set([]);
  connections.set({});
  lanPeers.set([]);
  unlisteners.length = 0;
  vi.mocked(netPendingIncoming).mockReset().mockResolvedValue([]);
  vi.mocked(netLanPeers).mockReset().mockResolvedValue([]);
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

  it("seeds lanPeers and replaces the snapshot on lan_peers_changed", async () => {
    const peer = (id: string) => ({ node_id: id, display_name: null, verified: false, ticket: `t-${id}` });
    vi.mocked(netLanPeers).mockResolvedValue([peer("a")]);
    await startNetEvents();
    expect(get(lanPeers).map((p) => p.node_id)).toEqual(["a"]);
    handlers.lan_peers_changed({ payload: [peer("b"), peer("c")] });
    expect(get(lanPeers).map((p) => p.node_id)).toEqual(["b", "c"]);
    handlers.lan_peers_changed({ payload: [] });
    expect(get(lanPeers)).toEqual([]);
  });

  it("tolerates a failing lan peers snapshot", async () => {
    vi.mocked(netLanPeers).mockRejectedValue({ type: "NotReady" });
    await expect(startNetEvents()).resolves.toBeTypeOf("function");
    expect(get(lanPeers)).toEqual([]);
  });

  it("calls unlisten for every listener", async () => {
    const stop = await startNetEvents();
    stop();
    expect(unlisteners).toHaveLength(3);
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
