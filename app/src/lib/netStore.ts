import { writable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  netLanPeers,
  netPendingIncoming,
  type ConnectionStateDto,
  type IncomingRequestDto,
  type LanPeerDto,
} from "../net";

export const lanPeers = writable<LanPeerDto[]>([]);
export const incomingRequests = writable<IncomingRequestDto[]>([]);
export const connections = writable<Record<string, ConnectionStateDto>>({});

function enqueue(req: IncomingRequestDto) {
  incomingRequests.update((q) => (q.some((r) => r.node_id === req.node_id) ? q : [...q, req]));
}

export function dropIncoming(nodeId: string) {
  incomingRequests.update((q) => q.filter((r) => r.node_id !== nodeId));
}

function applyConnectionState(c: ConnectionStateDto) {
  dropIncoming(c.node_id);
  connections.update((m) => {
    const next = { ...m };
    if (c.direction === "inbound" && c.state === "failed") {
      delete next[c.node_id];
    } else {
      next[c.node_id] = c;
    }
    return next;
  });
}

export async function startNetEvents(): Promise<() => void> {
  const unlisteners: UnlistenFn[] = [
    await listen<IncomingRequestDto>("incoming_request", (e) => enqueue(e.payload)),
    await listen<ConnectionStateDto>("connection_state", (e) => applyConnectionState(e.payload)),
    await listen<LanPeerDto[]>("lan_peers_changed", (e) => lanPeers.set(e.payload)),
  ];
  try {
    lanPeers.set(await netLanPeers());
  } catch {
    // Not ready yet: lan_peers_changed events supply the snapshot.
  }
  try {
    for (const req of await netPendingIncoming()) enqueue(req);
  } catch {
    // NotReady or a transient failure: the queue starts empty and events fill it.
  }
  return () => unlisteners.forEach((u) => u());
}
