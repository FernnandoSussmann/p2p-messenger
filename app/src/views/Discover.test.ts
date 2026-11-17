import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../lib/contactsStore", async () => {
  const { writable, derived } = await import("svelte/store");
  const contacts = writable<{ node_id: string }[]>([]);
  return {
    contacts,
    contactIds: derived(contacts, (c) => new Set(c.map((x) => x.node_id))),
    refreshContacts: vi.fn().mockResolvedValue(undefined),
    requestNaming: vi.fn(),
  };
});

import Discover from "./Discover.svelte";
import { connections, lanPeers } from "../lib/netStore";
import { contacts, requestNaming } from "../lib/contactsStore";

const conn = (id: string, o: Record<string, unknown> = {}) => ({
  node_id: id,
  direction: "outbound" as const,
  state: "connected" as const,
  reason: null,
  display_name: null,
  ...o,
});

afterEach(cleanup);
beforeEach(() => {
  connections.set({});
  lanPeers.set([]);
  (contacts as any).set([]);
  vi.mocked(requestNaming).mockReset();
});

describe("Discover", () => {
  it("has a hidden heading and the LAN list", () => {
    render(Discover);
    expect(screen.getByRole("heading", { name: "Discover", level: 2 })).toHaveClass("visually-hidden");
    expect(screen.getByText("On this network")).toBeInTheDocument();
  });

  it("hides the Connections group when there are no non-contact connections", () => {
    (contacts as any).set([{ node_id: "p1" }]);
    connections.set({ p1: conn("p1") });
    render(Discover);
    expect(screen.queryByText("Connections")).toBeNull();
    expect(screen.queryByTestId("connection")).toBeNull();
    expect(screen.queryByText("No connections yet.")).toBeNull();
  });

  it("updates the connection list when the store changes", async () => {
    render(Discover);
    expect(screen.queryAllByTestId("connection")).toHaveLength(0);
    connections.set({ abc: conn("abc", { state: "failed", reason: "declined" }) });
    await waitFor(() => expect(screen.getAllByTestId("connection")).toHaveLength(1));
    expect(screen.getByText("abc")).toBeTruthy();
    expect(screen.getByTestId("badge-unverified")).toBeInTheDocument();
    expect(screen.getByTestId("conn-direction")).toHaveTextContent("outbound");
    expect(screen.getByTestId("conn-state")).toHaveTextContent("failed");
    expect(screen.getByTestId("conn-reason")).toHaveTextContent("declined");
    expect(screen.queryByRole("button", { name: "Add contact…" })).toBeNull();
  });

  it("omits the reason when null", async () => {
    connections.set({ abc: conn("abc") });
    render(Discover);
    await screen.findByTestId("connection");
    expect(screen.queryByTestId("conn-reason")).toBeNull();
  });

  it("renders a hostile peer id as inert text", async () => {
    const evil = "<b>x</b>\u202e";
    connections.set({ [evil]: conn(evil, { direction: "inbound" }) });
    const { container } = render(Discover);
    expect(await screen.findByTestId("connection")).toHaveTextContent(evil);
    expect(container.querySelector("li b")).toBeNull();
  });

  it("connected non-contact row offers Add contact and requests naming with its display_name", async () => {
    connections.set({ p1: conn("p1", { display_name: "Pat" }) });
    render(Discover);
    await fireEvent.click(await screen.findByRole("button", { name: "Add contact…" }));
    expect(requestNaming).toHaveBeenCalledWith("p1", "Pat");
  });

  it("contact connections are not listed on Discover", async () => {
    (contacts as any).set([{ node_id: "p1" }]);
    connections.set({ p1: conn("p1", { display_name: "Pat" }), p2: conn("p2") });
    render(Discover);
    expect(await screen.findAllByTestId("connection")).toHaveLength(1);
    expect(screen.queryByText("p1")).toBeNull();
  });
});
