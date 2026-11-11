import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../contacts", async (orig) => ({
  ...(await orig<typeof import("../contacts")>()),
  contactsAdd: vi.fn(),
  contactsRename: vi.fn(),
  contactsRemove: vi.fn(),
}));
vi.mock("../lib/contactsStore", async () => {
  const { writable } = await import("svelte/store");
  const namingQueue = writable<unknown[]>([]);
  return {
    namingQueue,
    refreshContacts: vi.fn().mockResolvedValue(undefined),
    dropNaming: vi.fn((id: string) =>
      namingQueue.update((q) => q.filter((r) => (r as { node_id: string }).node_id !== id)),
    ),
  };
});

import NamingPrompt from "./NamingPrompt.svelte";
import { contactsAdd, contactsRename, contactsRemove } from "../contacts";
import { namingQueue, refreshContacts } from "../lib/contactsStore";

const queue = (nickname: string | null, self_name: string | null = "Eve", node_id = "n1") =>
  (namingQueue as any).set([{ node_id, self_name, nickname }]);
const field = () => screen.getByLabelText("Your name for them") as HTMLInputElement;
const save = () => screen.getByRole("button", { name: "Save contact" });

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(contactsAdd).mockReset().mockResolvedValue({} as never);
  vi.mocked(contactsRename).mockReset().mockResolvedValue({} as never);
  vi.mocked(contactsRemove).mockReset();
  vi.mocked(refreshContacts).mockClear();
  (namingQueue as any).set([]);
});

describe("NamingPrompt", () => {
  it("prefills the auto-added default nickname and shows the self-asserted name as unverified context", () => {
    queue("Eve");
    render(NamingPrompt);
    expect(field().value).toBe("Eve");
    expect(screen.getByTestId("badge-unverified")).toBeInTheDocument();
    expect(screen.getByTestId("self-name-context")).toHaveTextContent("chosen by them, not verified");
  });

  it("Save with unchanged nickname only dismisses", async () => {
    queue("Eve");
    render(NamingPrompt);
    await fireEvent.click(save());
    await waitFor(() => expect(screen.queryByTestId("naming-prompt")).toBeNull());
    expect(contactsRename).not.toHaveBeenCalled();
    expect(contactsAdd).not.toHaveBeenCalled();
  });

  it("Save with a new nickname renames", async () => {
    queue("Eve");
    render(NamingPrompt);
    await fireEvent.input(field(), { target: { value: "Evie" } });
    await fireEvent.click(save());
    await waitFor(() => expect(contactsRename).toHaveBeenCalledWith("n1", "Evie"));
    await waitFor(() => expect(screen.queryByTestId("naming-prompt")).toBeNull());
    expect(refreshContacts).toHaveBeenCalled();
  });

  it("Not now keeps the contact (no remove/rename call)", async () => {
    queue("Eve");
    render(NamingPrompt);
    await fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(screen.queryByTestId("naming-prompt")).toBeNull();
    expect(contactsRename).not.toHaveBeenCalled();
    expect(contactsRemove).not.toHaveBeenCalled();
  });

  it("no existing contact → Save adds", async () => {
    queue(null);
    render(NamingPrompt);
    expect(field().value).toBe("");
    await fireEvent.input(field(), { target: { value: "Eve" } });
    await fireEvent.click(save());
    await waitFor(() => expect(contactsAdd).toHaveBeenCalledWith("n1", "Eve"));
    await waitFor(() => expect(screen.queryByTestId("naming-prompt")).toBeNull());
  });

  it("AlreadyExists dequeues quietly", async () => {
    vi.mocked(contactsAdd).mockRejectedValue({ type: "AlreadyExists" });
    queue(null);
    render(NamingPrompt);
    await fireEvent.input(field(), { target: { value: "Eve" } });
    await fireEvent.click(save());
    await waitFor(() => expect(screen.queryByTestId("naming-prompt")).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("InvalidNickname keeps the prompt", async () => {
    vi.mocked(contactsRename).mockRejectedValue({ type: "InvalidNickname", kind: "InvisibleCharacter" });
    queue("Eve");
    render(NamingPrompt);
    await fireEvent.input(field(), { target: { value: "E​ve" } });
    await fireEvent.click(save());
    expect(await screen.findByRole("alert")).toHaveTextContent("invisible characters");
    expect(screen.getByTestId("naming-prompt")).toBeInTheDocument();
  });

  it("hostile names inert", () => {
    const evil = "<img src=x onerror=alert(1)>‮evil";
    queue("safe", evil);
    const { container } = render(NamingPrompt);
    expect(screen.getByTestId("peer-name")).toHaveTextContent(evil);
    expect(container.querySelector("img")).toBeNull();
  });
});
