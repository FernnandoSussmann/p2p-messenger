import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netConnectTicket: vi.fn(),
}));

import LanPeers from "./LanPeers.svelte";
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
  vi.mocked(netConnectTicket).mockReset().mockResolvedValue({ node_id: "x" });
});

describe("LanPeers", () => {
  it("shows the empty state", () => {
    render(LanPeers);
    expect(screen.getByText("No peers found on this network.")).toBeInTheDocument();
  });

  it("renders name, Unverified badge and full NodeId", () => {
    lanPeers.set([peer("a".repeat(64), "Alice")]);
    render(LanPeers);
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("Unverified")).toBeInTheDocument();
    expect(screen.getByText("a".repeat(64))).toBeInTheDocument();
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
    const evil = "<img src=x onerror=alert(1)>‮evil";
    lanPeers.set([peer("n1", evil)]);
    const { container } = render(LanPeers);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByTestId("lan-peer-name").textContent).toBe(evil);
  });

  it("connects with that peer's ticket", async () => {
    lanPeers.set([peer("id-1", "Bob", "ticket-1"), peer("id-2", "Bob", "ticket-2")]);
    render(LanPeers);
    await fireEvent.click(screen.getAllByRole("button", { name: "Connect" })[1]);
    await waitFor(() => expect(netConnectTicket).toHaveBeenCalledWith("ticket-2"));
  });

  it("shows the connect error", async () => {
    vi.mocked(netConnectTicket).mockRejectedValue({ type: "SelfDial" });
    lanPeers.set([peer("id-1", "Bob")]);
    render(LanPeers);
    await fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("That's your own ticket.");
  });
});
