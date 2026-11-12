import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netConnectTicket: vi.fn(),
  netPresenceGet: vi.fn().mockResolvedValue({ enabled: true }),
  netPresenceSet: vi.fn(),
}));

vi.mock("../lib/contactsStore", async () => {
  const { writable, derived } = await import("svelte/store");
  const contacts = writable<{ node_id: string }[]>([]);
  return {
    contacts,
    contactIds: derived(contacts, (c) => new Set(c.map((x) => x.node_id))),
    expectNaming: vi.fn(),
    requestNaming: vi.fn(),
  };
});

import Network from "./Network.svelte";
import { connections, lanPeers } from "../lib/netStore";
import { contacts, expectNaming, requestNaming } from "../lib/contactsStore";
import { netConnectTicket } from "../net";

const BTN = "Connect and remember this device";

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(netConnectTicket).mockReset().mockResolvedValue({ node_id: "n1" });
  connections.set({});
  lanPeers.set([]);
  (contacts as any).set([]);
  vi.mocked(expectNaming).mockReset();
  vi.mocked(requestNaming).mockReset();
});

describe("Network", () => {
  it("disables submit when empty", () => {
    render(Network);
    expect(screen.getByRole("button", { name: BTN })).toBeDisabled();
  });

  it("submits the trimmed ticket", async () => {
    render(Network);
    await fireEvent.input(screen.getByLabelText("Connect by ticket"), {
      target: { value: "  tkt-123 \n" },
    });
    await fireEvent.click(screen.getByRole("button", { name: BTN }));
    await waitFor(() => expect(netConnectTicket).toHaveBeenCalledWith("tkt-123"));
  });

  it("shows an alert for InvalidTicket and keeps the field value", async () => {
    vi.mocked(netConnectTicket).mockRejectedValue({ type: "InvalidTicket" });
    render(Network);
    const field = screen.getByLabelText("Connect by ticket") as HTMLTextAreaElement;
    await fireEvent.input(field, { target: { value: "bad" } });
    await fireEvent.click(screen.getByRole("button", { name: BTN }));
    expect(await screen.findByRole("alert")).toHaveTextContent("That ticket is not valid.");
    expect(field.value).toBe("bad");
  });

  it("shows the self-dial message", async () => {
    vi.mocked(netConnectTicket).mockRejectedValue({ type: "SelfDial" });
    render(Network);
    await fireEvent.input(screen.getByLabelText("Connect by ticket"), { target: { value: "me" } });
    await fireEvent.click(screen.getByRole("button", { name: BTN }));
    expect(await screen.findByRole("alert")).toHaveTextContent("That's your own ticket.");
  });

  it("updates the connection list when the store changes", async () => {
    render(Network);
    expect(screen.queryAllByTestId("connection")).toHaveLength(0);
    connections.set({
      abc: { node_id: "abc", direction: "outbound", state: "failed", reason: "declined", display_name: null },
    });
    await waitFor(() => expect(screen.getAllByTestId("connection")).toHaveLength(1));
    expect(screen.getByText("abc")).toBeTruthy();
    expect(screen.getByTestId("conn-state")).toHaveTextContent("failed");
    expect(screen.getByTestId("conn-reason")).toHaveTextContent("declined");
  });

  it("renders a hostile peer id as inert text", async () => {
    const evil = "<b>x</b>‮";
    connections.set({
      [evil]: { node_id: evil, direction: "inbound", state: "connected", reason: null, display_name: null },
    });
    const { container } = render(Network);
    expect(await screen.findByTestId("connection")).toHaveTextContent(evil);
    expect(container.querySelector("li b")).toBeNull();
  });

  it("successful connect expects naming for the dialed node", async () => {
    render(Network);
    const field = screen.getByLabelText("Connect by ticket") as HTMLTextAreaElement;
    await fireEvent.input(field, { target: { value: "tkt" } });
    await fireEvent.click(screen.getByRole("button", { name: BTN }));
    await waitFor(() => expect(expectNaming).toHaveBeenCalledWith("n1"));
    expect(field.value).toBe("");
  });

  it("copy says the device is remembered as an unverified contact", () => {
    render(Network);
    expect(screen.getByText(/saved as an unverified contact under the name they gave/)).toBeInTheDocument();
  });

  const conn = (id: string, name: string | null) => ({
    node_id: id,
    direction: "outbound" as const,
    state: "connected" as const,
    reason: null,
    display_name: name,
  });

  it("connected non-contact row offers Add contact and requests naming with its display_name", async () => {
    connections.set({ p1: conn("p1", "Pat") });
    render(Network);
    await fireEvent.click(await screen.findByRole("button", { name: "Add contact…" }));
    expect(requestNaming).toHaveBeenCalledWith("p1", "Pat");
  });

  it("contact rows do not offer Add contact", async () => {
    (contacts as any).set([{ node_id: "p1" }]);
    connections.set({ p1: conn("p1", "Pat") });
    render(Network);
    await screen.findByTestId("connection");
    expect(screen.queryByRole("button", { name: "Add contact…" })).toBeNull();
  });

  it("LAN list is no longer on the Network tab", () => {
    lanPeers.set([{ node_id: "l1", display_name: "Lan", verified: false, ticket: "t" }]);
    render(Network);
    expect(screen.queryByText("On this network")).toBeNull();
    expect(screen.queryByTestId("lan-peer")).toBeNull();
  });
});
