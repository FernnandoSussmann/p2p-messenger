import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../contacts", async (orig) => ({
  ...(await orig<typeof import("../contacts")>()),
  contactsRemove: vi.fn(),
}));
vi.mock("../lib/contactsStore", () => ({ refreshContacts: vi.fn(async () => {}) }));

import RemoveContactDialog from "./RemoveContactDialog.svelte";
import { contactsRemove } from "../contacts";
import { HISTORY_DELETE_GUARANTEE } from "../lib/historyCopy";

const contact = { node_id: "id", nickname: "Bob", self_name: null, verified: false, added_at_ms: 0, last_seen_ms: null };
const onDone = vi.fn();
const onCancel = vi.fn();
const show = () => render(RemoveContactDialog, { contact, onDone, onCancel });
const box = () => screen.getByRole("checkbox") as HTMLInputElement;

afterEach(cleanup);
beforeEach(() => vi.clearAllMocks());

describe("RemoveContactDialog", () => {
  it("history checkbox is unchecked by default", () => {
    show();
    expect(box().checked).toBe(false);
  });

  it("default confirm keeps history", async () => {
    vi.mocked(contactsRemove).mockResolvedValue({ node_id: "id", history_erased: false });
    show();
    expect(screen.getByRole("dialog").textContent).toContain("will be kept");
    expect(screen.getByRole("dialog").textContent).not.toContain(HISTORY_DELETE_GUARANTEE);
    await fireEvent.click(screen.getByRole("button", { name: "Remove contact" }));
    await waitFor(() => expect(onDone).toHaveBeenCalledWith({ node_id: "id", history_erased: false }));
    expect(contactsRemove).toHaveBeenCalledWith("id", false);
  });

  it("checking shows the irreversible outcome and changes the button", async () => {
    vi.mocked(contactsRemove).mockResolvedValue({ node_id: "id", history_erased: true });
    show();
    await fireEvent.click(box());
    const text = screen.getByRole("dialog").textContent!;
    expect(text).toContain("permanently deleted");
    expect(text).toContain("cannot be undone");
    expect(text).toContain(HISTORY_DELETE_GUARANTEE);
    await fireEvent.click(screen.getByRole("button", { name: "Remove contact and delete history" }));
    await waitFor(() => expect(contactsRemove).toHaveBeenCalledWith("id", true));
  });

  it("reopening resets the checkbox", async () => {
    const first = show();
    await fireEvent.click(box());
    first.unmount();
    show();
    expect(box().checked).toBe(false);
  });

  it("HistoryEraseFailed keeps the dialog open with the not-removed message", async () => {
    vi.mocked(contactsRemove).mockRejectedValue({ type: "HistoryEraseFailed" });
    show();
    await fireEvent.click(box());
    await fireEvent.click(screen.getByRole("button", { name: "Remove contact and delete history" }));
    expect((await screen.findByRole("alert")).textContent).toContain("so the contact was not removed");
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(box().checked).toBe(true);
    expect(onDone).not.toHaveBeenCalled();
  });
});
