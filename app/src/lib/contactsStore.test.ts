import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { get } from "svelte/store";

type Handler = (e: { payload: unknown }) => void;
const handlers: Record<string, Handler> = {};
const unlisten = vi.fn();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, h: Handler) => {
    handlers[name] = h;
    return unlisten;
  }),
}));
vi.mock("../contacts", async (orig) => ({
  ...(await orig<typeof import("../contacts")>()),
  contactsList: vi.fn(),
}));

import {
  contacts,
  contactIds,
  contactsError,
  namingQueue,
  refreshContacts,
  expectNaming,
  requestNaming,
  startContacts,
} from "./contactsStore";
import { contactsList, type ContactDto } from "../contacts";

const contact = (id: string, nickname = "Bob"): ContactDto => ({
  node_id: id,
  nickname,
  self_name: "Bobby",
  verified: false,
  added_at_ms: 1,
  last_seen_ms: null,
});
const conn = (id: string, direction: "inbound" | "outbound", state: string, name: string | null = "Bobby") => ({
  node_id: id,
  direction,
  state,
  reason: null,
  display_name: name,
});
const emit = (c: unknown) => handlers["connection_state"]({ payload: c });
const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(async () => {
  vi.useRealTimers();
  contacts.set([]);
  contactsError.set(null);
  namingQueue.set([]);
  unlisten.mockReset();
  vi.mocked(contactsList).mockReset().mockResolvedValue([]);
  await startContacts();
  // Clear any module-level expectation/buffer state from earlier tests.
  for (const id of ["a", "b", "c", "old"]) {
    emit(conn(id, "outbound", "failed"));
  }
});

afterEach(() => vi.useRealTimers());

describe("contactsStore", () => {
  it("refreshContacts fills the store", async () => {
    vi.mocked(contactsList).mockResolvedValue([contact("a")]);
    await refreshContacts();
    expect(get(contacts)).toHaveLength(1);
    expect(get(contactIds).has("a")).toBe(true);
  });

  it("refresh failure keeps the old list and sets an error", async () => {
    vi.mocked(contactsList).mockResolvedValue([contact("a")]);
    await refreshContacts();
    vi.mocked(contactsList).mockRejectedValue({ type: "StoreUnavailable" });
    await refreshContacts();
    expect(get(contacts)).toHaveLength(1);
    expect(get(contactsError)).toBe("Contacts could not be saved right now. Try again.");
  });

  it("expected outbound connected enqueues naming prefilled with the contact's nickname", async () => {
    vi.mocked(contactsList).mockResolvedValue([contact("a", "Peer a")]);
    expectNaming("a");
    emit(conn("a", "outbound", "connected"));
    await flush();
    expect(get(namingQueue)).toEqual([{ node_id: "a", self_name: "Bobby", nickname: "Peer a" }]);
  });

  it("connected event that arrives before expectNaming is still handled", async () => {
    vi.mocked(contactsList).mockResolvedValue([contact("a", "Peer a")]);
    emit(conn("a", "outbound", "connected"));
    expect(get(namingQueue)).toEqual([]);
    expectNaming("a");
    await flush();
    expect(get(namingQueue)).toEqual([{ node_id: "a", self_name: "Bobby", nickname: "Peer a" }]);
  });

  it("buffered events older than the TTL are ignored", async () => {
    vi.useFakeTimers();
    emit(conn("old", "outbound", "connected"));
    vi.advanceTimersByTime(61_000);
    expectNaming("old");
    await vi.advanceTimersByTimeAsync(10);
    expect(get(namingQueue)).toEqual([]);
  });

  it("inbound connected never enqueues naming", async () => {
    expectNaming("a");
    emit(conn("a", "inbound", "connected"));
    await flush();
    expect(get(namingQueue)).toEqual([]);
  });

  it("unexpected outbound connected does not enqueue", async () => {
    emit(conn("b", "outbound", "connected"));
    await flush();
    expect(get(namingQueue)).toEqual([]);
  });

  it("failed outbound forgets the expectation", async () => {
    expectNaming("a");
    emit(conn("a", "outbound", "failed"));
    emit(conn("a", "outbound", "connected"));
    await flush();
    expect(get(namingQueue)).toEqual([]);
  });

  it("requestNaming with no contact enqueues nickname null", async () => {
    await requestNaming("c", "Carol");
    expect(get(namingQueue)).toEqual([{ node_id: "c", self_name: "Carol", nickname: null }]);
  });

  it("requestNaming dedups by node_id", async () => {
    await requestNaming("c", "Carol");
    await requestNaming("c", "Carol");
    expect(get(namingQueue)).toHaveLength(1);
  });

  it("any connected triggers a refresh", async () => {
    vi.mocked(contactsList).mockClear();
    emit(conn("b", "inbound", "connected"));
    await flush();
    expect(contactsList).toHaveBeenCalled();
  });

  it("startContacts unlistens", async () => {
    const off = await startContacts();
    off();
    expect(unlisten).toHaveBeenCalled();
  });
});
