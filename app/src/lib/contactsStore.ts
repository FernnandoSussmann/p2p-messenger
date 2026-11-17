import { derived, get, writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import { contactsList, describeContactsError, type ContactDto } from "../contacts";
import type { ConnectionStateDto } from "../net";

export const contacts = writable<ContactDto[]>([]);
export const contactIds = derived(contacts, (c) => new Set(c.map((x) => x.node_id)));
export const contactsError = writable<string | null>(null);
export const contactsNotice = writable<string | null>(null);

export async function refreshContacts(): Promise<void> {
  try {
    contacts.set(await contactsList());
    contactsError.set(null);
  } catch (err) {
    contactsError.set(describeContactsError(err));
  }
}

export interface NamingRequest {
  node_id: string;
  self_name: string | null;
  nickname: string | null;
}

export const namingQueue = writable<NamingRequest[]>([]);

const RECENT_OUTBOUND_TTL_MS = 60_000;
const EXPECTED_TTL_MS = 120_000;
const RECENT_OUTBOUND_MAX = 32;

interface Recent {
  displayName: string | null;
  at: number;
}

const recentOutbound = new Map<string, Recent>();
const expected = new Map<string, number>();

function pruneRecent(now: number) {
  for (const [id, r] of recentOutbound) {
    if (now - r.at > RECENT_OUTBOUND_TTL_MS) recentOutbound.delete(id);
  }
}

function pruneExpected(now: number) {
  for (const [id, at] of expected) {
    if (now - at > EXPECTED_TTL_MS) expected.delete(id);
  }
}

export function expectNaming(nodeId: string): void {
  const now = Date.now();
  pruneRecent(now);
  const recent = recentOutbound.get(nodeId);
  if (recent) {
    recentOutbound.delete(nodeId);
    void requestNaming(nodeId, recent.displayName);
    return;
  }
  pruneExpected(now);
  expected.set(nodeId, now);
}

export async function requestNaming(nodeId: string, selfName: string | null): Promise<void> {
  await refreshContacts();
  const nickname = get(contacts).find((c) => c.node_id === nodeId)?.nickname ?? null;
  namingQueue.update((q) =>
    q.some((r) => r.node_id === nodeId)
      ? q
      : [...q, { node_id: nodeId, self_name: selfName, nickname }],
  );
}

export function dropNaming(nodeId: string): void {
  namingQueue.update((q) => q.filter((r) => r.node_id !== nodeId));
}

export function onConnectionState(c: ConnectionStateDto): void {
  const now = Date.now();
  if (c.direction === "outbound") {
    if (c.state === "connected") {
      pruneExpected(now);
      if (expected.has(c.node_id)) {
        expected.delete(c.node_id);
        void requestNaming(c.node_id, c.display_name);
      } else {
        pruneRecent(now);
        recentOutbound.delete(c.node_id);
        recentOutbound.set(c.node_id, { displayName: c.display_name, at: now });
        while (recentOutbound.size > RECENT_OUTBOUND_MAX) {
          recentOutbound.delete(recentOutbound.keys().next().value as string);
        }
      }
    } else if (c.state === "failed") {
      expected.delete(c.node_id);
    }
  }
  if (c.state === "connected") void refreshContacts();
}

export async function startContacts(): Promise<() => void> {
  const unlisten = await listen<ConnectionStateDto>("connection_state", (e) =>
    onConnectionState(e.payload),
  );
  await refreshContacts();
  return unlisten;
}
