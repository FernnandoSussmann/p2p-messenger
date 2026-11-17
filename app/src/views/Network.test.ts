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
  };
});

import Network from "./Network.svelte";
import { connections, lanPeers } from "../lib/netStore";
import { contacts, expectNaming } from "../lib/contactsStore";
import { netConnectTicket } from "../net";

const BTN = "Connect and remember this device";

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(netConnectTicket).mockReset().mockResolvedValue({ node_id: "n1" });
  connections.set({});
  lanPeers.set([]);
  (contacts as any).set([]);
  vi.mocked(expectNaming).mockReset();
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

  it("lays out switch, captions, explanation, ticket field, button and paragraph in order", () => {
    const { container } = render(Network);
    const sw = screen.getByRole("switch", { name: "Visible on this network" });
    expect(screen.getByText("Off")).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByText("On")).toHaveAttribute("aria-hidden", "true");
    const explanation = screen.getByText(/You are/);
    const field = screen.getByLabelText("Connect by ticket");
    const button = screen.getByRole("button", { name: BTN });
    const para = screen.getByText(/saved as an unverified contact/);
    const order = [sw, explanation, field, button, para];
    for (let i = 0; i < order.length - 1; i++) {
      expect(order[i].compareDocumentPosition(order[i + 1]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
    expect(container.querySelector("hr.divider")).not.toBeNull();
    expect(screen.getByRole("heading", { name: "Network", level: 2 })).toHaveClass("visually-hidden");
  });

  it("no longer shows Connections", () => {
    connections.set({ p1: { node_id: "p1", direction: "outbound", state: "connected", reason: null, display_name: null } });
    render(Network);
    expect(screen.queryByText("Connections")).toBeNull();
    expect(screen.queryByText("No connections yet.")).toBeNull();
    expect(screen.queryByTestId("connection")).toBeNull();
  });

  it("LAN list is no longer on the Network tab", () => {
    lanPeers.set([{ node_id: "l1", display_name: "Lan", verified: false, ticket: "t" }]);
    render(Network);
    expect(screen.queryByText("On this network")).toBeNull();
    expect(screen.queryByTestId("lan-peer")).toBeNull();
  });
});
