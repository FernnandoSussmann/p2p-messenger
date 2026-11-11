import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netConnectTicket: vi.fn(),
  netPresenceGet: vi.fn().mockResolvedValue({ enabled: true }),
  netPresenceSet: vi.fn(),
}));

import Network from "./Network.svelte";
import { connections } from "../lib/netStore";
import { netConnectTicket } from "../net";

const BTN = "Connect and remember this device";

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(netConnectTicket).mockReset().mockResolvedValue({ node_id: "n1" });
  connections.set({});
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
      abc: { node_id: "abc", direction: "outbound", state: "failed", reason: "declined" },
    });
    await waitFor(() => expect(screen.getAllByTestId("connection")).toHaveLength(1));
    expect(screen.getByText("abc")).toBeTruthy();
    expect(screen.getByTestId("conn-state")).toHaveTextContent("failed");
    expect(screen.getByTestId("conn-reason")).toHaveTextContent("declined");
  });

  it("renders a hostile peer id as inert text", async () => {
    const evil = "<b>x</b>‮";
    connections.set({
      [evil]: { node_id: evil, direction: "inbound", state: "connected", reason: null },
    });
    const { container } = render(Network);
    expect(await screen.findByTestId("connection")).toHaveTextContent(evil);
    expect(container.querySelector("li b")).toBeNull();
  });
});
