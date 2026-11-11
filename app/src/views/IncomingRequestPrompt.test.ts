import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../net", () => ({ netRespondIncoming: vi.fn() }));

import IncomingRequestPrompt from "./IncomingRequestPrompt.svelte";
import { incomingRequests, connections } from "../lib/netStore";
import { netRespondIncoming } from "../net";

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(netRespondIncoming).mockReset().mockResolvedValue(undefined);
  incomingRequests.set([]);
  connections.set({});
});

describe("IncomingRequestPrompt", () => {
  it("renders nothing without requests", () => {
    render(IncomingRequestPrompt);
    expect(screen.queryByTestId("incoming-prompt")).toBeNull();
  });

  it("shows UNVERIFIED, the name and the full NodeId", () => {
    incomingRequests.set([{ node_id: "node-full-id-1", display_name: "Eve" }]);
    render(IncomingRequestPrompt);
    expect(screen.getByTestId("unverified-badge")).toHaveTextContent("Unverified");
    expect(screen.getByText(/calls themselves/)).toHaveTextContent("Eve");
    expect(screen.getByTestId("peer-node-id")).toHaveTextContent("node-full-id-1");
  });

  it("shows (no name) when there is none", () => {
    incomingRequests.set([{ node_id: "n1", display_name: null }]);
    render(IncomingRequestPrompt);
    expect(screen.getByText(/\(no name\)/)).toBeTruthy();
  });

  it("Accept and Decline call netRespondIncoming with exact args", async () => {
    incomingRequests.set([
      { node_id: "n1", display_name: "A" },
      { node_id: "n2", display_name: "B" },
    ]);
    render(IncomingRequestPrompt);
    await fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    expect(netRespondIncoming).toHaveBeenLastCalledWith("n1", true);
    await waitFor(() => expect(screen.getByTestId("peer-node-id")).toHaveTextContent("n2"));
    await fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    expect(netRespondIncoming).toHaveBeenLastCalledWith("n2", false);
  });

  it("keeps same-name different-NodeId requests distinct", async () => {
    incomingRequests.set([
      { node_id: "n1", display_name: "Same" },
      { node_id: "n2", display_name: "Same" },
    ]);
    render(IncomingRequestPrompt);
    expect(screen.getByTestId("peer-node-id")).toHaveTextContent("n1");
    expect(screen.getByText("+1 more")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    await waitFor(() => expect(screen.getByTestId("peer-node-id")).toHaveTextContent("n2"));
    expect(screen.queryByText(/more/)).toBeNull();
  });

  it("disappears when the request is removed from the queue", async () => {
    incomingRequests.set([{ node_id: "n1", display_name: "A" }]);
    render(IncomingRequestPrompt);
    incomingRequests.set([]);
    await waitFor(() => expect(screen.queryByTestId("incoming-prompt")).toBeNull());
  });

  it("drops the prompt silently on NoSuchRequest", async () => {
    vi.mocked(netRespondIncoming).mockRejectedValue({ type: "NoSuchRequest" });
    incomingRequests.set([{ node_id: "n1", display_name: "A" }]);
    render(IncomingRequestPrompt);
    await fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    await waitFor(() => expect(screen.queryByTestId("incoming-prompt")).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("renders a hostile name as inert text", () => {
    const evil = "<img src=x onerror=alert(1)>‮evil";
    incomingRequests.set([{ node_id: "n1", display_name: evil }]);
    const { container } = render(IncomingRequestPrompt);
    expect(screen.getByTestId("peer-name")).toHaveTextContent(evil);
    expect(container.querySelector("img")).toBeNull();
  });
});
