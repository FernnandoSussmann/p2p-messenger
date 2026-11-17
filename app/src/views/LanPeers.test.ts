import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netConnectTicket: vi.fn(),
}));

vi.mock("../contacts", async (orig) => ({
  ...(await orig<typeof import("../contacts")>()),
  contactsAdd: vi.fn(),
}));
vi.mock("../lib/contactsStore", async () => {
  const { writable, derived } = await import("svelte/store");
  const contacts = writable<{ node_id: string }[]>([]);
  return {
    contacts,
    contactIds: derived(contacts, (c) => new Set(c.map((x) => x.node_id))),
    refreshContacts: vi.fn().mockResolvedValue(undefined),
  };
});

import LanPeers from "./LanPeers.svelte";
import { contactsAdd } from "../contacts";
import { contacts, refreshContacts } from "../lib/contactsStore";
import { lanPeers } from "../lib/netStore";
import { netConnectTicket } from "../net";

const peer = (id: string, name: string | null, ticket = `t-${id}`) => ({
  node_id: id,
  display_name: name,
  verified: false,
  ticket,
});

afterEach(cleanup);
beforeEach(() => {
  lanPeers.set([]);
  (contacts as any).set([]);
  vi.mocked(contactsAdd).mockReset().mockResolvedValue({} as never);
  vi.mocked(refreshContacts).mockClear();
  vi.mocked(netConnectTicket).mockReset().mockResolvedValue({ node_id: "x" });
});

describe("LanPeers", () => {
  it("shows the empty state", () => {
    render(LanPeers);
    expect(screen.getByText("No one new on this network.")).toBeInTheDocument();
  });

  it("hides peers that are already contacts", () => {
    lanPeers.set([peer("c1", "Carol"), peer("n1", "Nina")]);
    (contacts as any).set([{ node_id: "c1" }]);
    render(LanPeers);
    expect(screen.getAllByTestId("lan-peer")).toHaveLength(1);
    expect(screen.getByTestId("lan-peer-name")).toHaveTextContent("Nina");
    expect(screen.queryByText("Carol")).toBeNull();
  });

  it("always shows Unverified even if the DTO says verified", () => {
    lanPeers.set([{ ...peer("n1", "Eve"), verified: true }]);
    render(LanPeers);
    expect(screen.getByTestId("badge-unverified")).toBeInTheDocument();
    expect(screen.queryByTestId("badge-verified")).toBeNull();
  });

  it("shows (no name) when the peer has none", () => {
    lanPeers.set([peer("n1", null)]);
    render(LanPeers);
    expect(screen.getByText("(no name)")).toBeInTheDocument();
  });

  it("keeps same-named peers distinct by NodeId", () => {
    lanPeers.set([peer("id-1", "Bob"), peer("id-2", "Bob")]);
    render(LanPeers);
    expect(screen.getAllByTestId("lan-peer")).toHaveLength(2);
    expect(screen.getByText("id-1")).toBeInTheDocument();
    expect(screen.getByText("id-2")).toBeInTheDocument();
  });

  it("renders HTML and bidi names inert", () => {
    const evil = "<img src=x onerror=alert(1)>\u202eevil";
    lanPeers.set([peer("n1", evil)]);
    const { container } = render(LanPeers);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByTestId("lan-peer-name").textContent).toBe(evil);
  });

  it("form is inline without an Add… button; Add is disabled until a nickname is typed", async () => {
    lanPeers.set([peer("n1", "Bob")]);
    render(LanPeers);
    expect(screen.queryByRole("button", { name: "Add…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
    const field = screen.getByLabelText("Your name for them") as HTMLInputElement;
    expect(field.value).toBe("");
    const add = screen.getByRole("button", { name: "Add contact and connect" });
    expect(add).toBeDisabled();
    await fireEvent.input(field, { target: { value: "Bobby" } });
    expect(add).toBeEnabled();
  });

  async function submit(nick: string) {
    await fireEvent.input(screen.getAllByLabelText("Your name for them")[1], { target: { value: nick } });
    await fireEvent.click(screen.getAllByRole("button", { name: "Add contact and connect" })[1]);
  }

  it("Add sends the typed nickname then dials the peer's ticket", async () => {
    lanPeers.set([peer("id-1", "Bob", "ticket-1"), peer("id-2", "Bob", "ticket-2")]);
    render(LanPeers);
    await submit("Bobby");
    await waitFor(() => expect(netConnectTicket).toHaveBeenCalledWith("ticket-2"));
    expect(contactsAdd).toHaveBeenCalledWith("id-2", "Bobby");
    const order = [
      vi.mocked(contactsAdd).mock.invocationCallOrder[0],
      vi.mocked(refreshContacts).mock.invocationCallOrder[0],
      vi.mocked(netConnectTicket).mock.invocationCallOrder[0],
    ];
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });

  it("Add failure does not dial", async () => {
    vi.mocked(contactsAdd).mockRejectedValue({ type: "Full" });
    lanPeers.set([peer("id-1", "Bob"), peer("id-2", "Bob")]);
    render(LanPeers);
    await submit("Bobby");
    expect(await screen.findByRole("alert")).toHaveTextContent("Your contact list is full.");
    expect(netConnectTicket).not.toHaveBeenCalled();
  });

  it("dial failure after add reports both facts", async () => {
    vi.mocked(netConnectTicket).mockRejectedValue({ type: "SelfDial" });
    lanPeers.set([peer("id-1", "Bob"), peer("id-2", "Bob")]);
    render(LanPeers);
    await submit("Bobby");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Added “Bobby”, but connecting failed: That's your own ticket.",
    );
  });
});
