import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../net", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../net")>()),
  netRespondIncoming: vi.fn(),
}));

vi.mock("../lib/contactsStore", () => ({ refreshContacts: vi.fn().mockResolvedValue(undefined) }));

import { refreshContacts } from "../lib/contactsStore";
import IncomingRequestPrompt from "./IncomingRequestPrompt.svelte";
import { incomingRequests, connections } from "../lib/netStore";
import { netRespondIncoming } from "../net";

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(netRespondIncoming).mockReset().mockResolvedValue(undefined);
  incomingRequests.set([]);
  connections.set({});
  vi.mocked(refreshContacts).mockClear();
});

const req = (id: string, name: string | null, suggested = name ?? "Peer abc…") => ({
  node_id: id,
  display_name: name,
  suggested_nickname: suggested,
});
const field = () => screen.getByLabelText("Your name for them") as HTMLInputElement;
const accept = () => screen.getByRole("button", { name: "Accept and add contact" });

describe("IncomingRequestPrompt", () => {
  it("renders nothing without requests", () => {
    render(IncomingRequestPrompt);
    expect(screen.queryByTestId("incoming-prompt")).toBeNull();
  });

  it("shows UNVERIFIED and the full NodeId", () => {
    incomingRequests.set([req("node-full-id-1", "Eve")]);
    render(IncomingRequestPrompt);
    expect(screen.getByTestId("badge-unverified")).toHaveTextContent("Unverified");
    expect(screen.getByTestId("peer-node-id")).toHaveTextContent("node-full-id-1");
  });

  it("the self-asserted name is shown as context", () => {
    incomingRequests.set([req("n1", "Eve")]);
    render(IncomingRequestPrompt);
    expect(screen.getByTestId("self-name-context")).toHaveTextContent(
      "They call themselves “Eve” (chosen by them, not verified)",
    );
  });

  it("says when there is no name", () => {
    incomingRequests.set([req("n1", null)]);
    render(IncomingRequestPrompt);
    expect(screen.getByText("They did not send a name")).toBeTruthy();
  });

  it("the nickname field is prefilled with suggested_nickname", () => {
    incomingRequests.set([req("n1", "Eve", "Eve")]);
    const { unmount } = render(IncomingRequestPrompt);
    expect(field().value).toBe("Eve");
    unmount();
    incomingRequests.set([req("n1", null, "Peer abc…")]);
    render(IncomingRequestPrompt);
    expect(field().value).toBe("Peer abc…");
  });

  it("nothing is sent until Accept is clicked", () => {
    incomingRequests.set([req("n1", "Eve")]);
    render(IncomingRequestPrompt);
    expect(netRespondIncoming).not.toHaveBeenCalled();
  });

  it("accepting the untouched prefill sends it", async () => {
    incomingRequests.set([req("n1", "Eve", "Eve")]);
    render(IncomingRequestPrompt);
    await fireEvent.click(accept());
    expect(netRespondIncoming).toHaveBeenLastCalledWith("n1", true, "Eve");
  });

  it("the prefill is editable and the edited value is sent", async () => {
    incomingRequests.set([req("n1", "Eve", "Eve")]);
    render(IncomingRequestPrompt);
    await fireEvent.input(field(), { target: { value: "" } });
    await fireEvent.input(field(), { target: { value: "Bobby" } });
    await fireEvent.click(accept());
    expect(netRespondIncoming).toHaveBeenLastCalledWith("n1", true, "Bobby");
    await waitFor(() => expect(screen.queryByTestId("incoming-prompt")).toBeNull());
  });

  it("successful accept refreshes contacts", async () => {
    incomingRequests.set([req("n1", "Eve", "Eve")]);
    render(IncomingRequestPrompt);
    await fireEvent.click(accept());
    await waitFor(() => expect(refreshContacts).toHaveBeenCalledTimes(1));
  });

  it("prefill still comes from suggested_nickname", () => {
    incomingRequests.set([req("n1", "Eve", "Evie")]);
    render(IncomingRequestPrompt);
    expect(field().value).toBe("Evie");
  });

  it("clearing the field disables Accept", async () => {
    incomingRequests.set([req("n1", "Eve", "Eve")]);
    render(IncomingRequestPrompt);
    for (const blank of ["", "   "]) {
      await fireEvent.input(field(), { target: { value: blank } });
      expect(accept()).toBeDisabled();
      await fireEvent.click(accept());
    }
    expect(netRespondIncoming).not.toHaveBeenCalled();
  });

  it("Decline sends no nickname", async () => {
    incomingRequests.set([req("n1", "A"), req("n2", "B")]);
    render(IncomingRequestPrompt);
    await fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    expect(netRespondIncoming).toHaveBeenLastCalledWith("n1", false);
    await waitFor(() => expect(screen.getByTestId("peer-node-id")).toHaveTextContent("n2"));
  });

  it("InvalidNickname keeps the prompt and shows the message", async () => {
    vi.mocked(netRespondIncoming).mockRejectedValue({
      type: "InvalidNickname",
      kind: "InvisibleCharacter",
    });
    incomingRequests.set([req("n1", "Eve", "Eve")]);
    render(IncomingRequestPrompt);
    await fireEvent.click(accept());
    expect(await screen.findByRole("alert")).toHaveTextContent("invisible characters");
    expect(screen.getByTestId("incoming-prompt")).toBeTruthy();
    expect(accept()).not.toBeDisabled();
  });

  it("ContactsFull and ContactsUnavailable keep the prompt", async () => {
    incomingRequests.set([req("n1", "Eve", "Eve")]);
    render(IncomingRequestPrompt);
    vi.mocked(netRespondIncoming).mockRejectedValue({ type: "ContactsFull" });
    await fireEvent.click(accept());
    expect(await screen.findByRole("alert")).toHaveTextContent("Your contact list is full.");
    vi.mocked(netRespondIncoming).mockRejectedValue({ type: "ContactsUnavailable" });
    await fireEvent.click(accept());
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Contacts are unavailable"),
    );
    expect(screen.getByTestId("incoming-prompt")).toBeTruthy();
  });

  it("drops the prompt silently on NoSuchRequest", async () => {
    vi.mocked(netRespondIncoming).mockRejectedValue({ type: "NoSuchRequest" });
    incomingRequests.set([req("n1", "A")]);
    render(IncomingRequestPrompt);
    await fireEvent.click(accept());
    await waitFor(() => expect(screen.queryByTestId("incoming-prompt")).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps same-name different-NodeId requests distinct and resets the form", async () => {
    incomingRequests.set([req("n1", "Same", "Same"), req("n2", "Same", "Same")]);
    render(IncomingRequestPrompt);
    expect(screen.getByTestId("peer-node-id")).toHaveTextContent("n1");
    expect(screen.getByText("+1 more")).toBeTruthy();
    await fireEvent.input(field(), { target: { value: "Edited" } });
    await fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    await waitFor(() => expect(screen.getByTestId("peer-node-id")).toHaveTextContent("n2"));
    expect(field().value).toBe("Same");
    expect(screen.queryByText(/more/)).toBeNull();
  });

  it("disappears when the request is removed from the queue", async () => {
    incomingRequests.set([req("n1", "A")]);
    render(IncomingRequestPrompt);
    incomingRequests.set([]);
    await waitFor(() => expect(screen.queryByTestId("incoming-prompt")).toBeNull());
  });

  it("hostile self-asserted names render inert", () => {
    const evil = "<img src=x onerror=alert(1)>‮evil";
    incomingRequests.set([req("n1", evil, "safe")]);
    const { container } = render(IncomingRequestPrompt);
    expect(screen.getByTestId("peer-name")).toHaveTextContent(evil);
    expect(container.querySelector("img")).toBeNull();
  });
});
