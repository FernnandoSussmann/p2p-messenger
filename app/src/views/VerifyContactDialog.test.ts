import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../contacts", async (orig) => ({
  ...(await orig<typeof import("../contacts")>()),
  contactsSetVerified: vi.fn(),
}));
vi.mock("../lib/contactsStore", () => ({ refreshContacts: vi.fn(async () => {}) }));

import VerifyContactDialog from "./VerifyContactDialog.svelte";
import { contactsSetVerified } from "../contacts";

const contact = { node_id: "n".repeat(64), nickname: "Bob", self_name: null, verified: false, added_at_ms: 0, last_seen_ms: null };
const onDone = vi.fn();
const onCancel = vi.fn();
const show = () => render(VerifyContactDialog, { contact, onDone, onCancel });

afterEach(cleanup);
beforeEach(() => vi.clearAllMocks());

describe("VerifyContactDialog", () => {
  it("copy states it is the user's statement and not a cryptographic check", () => {
    show();
    const text = screen.getByRole("dialog").textContent!;
    expect(text).toContain("This is your own statement");
    expect(text).toContain("does not check it cryptographically");
    expect(text).not.toMatch(/secure|guarantee|cryptographically verified/i);
  });

  it("shows the full NodeId", () => {
    show();
    expect(screen.getByTestId("verify-node-id").textContent).toBe(contact.node_id);
  });

  it("confirm calls contactsSetVerified(id, true)", async () => {
    vi.mocked(contactsSetVerified).mockResolvedValue({ ...contact, verified: true });
    show();
    await fireEvent.click(screen.getByRole("button", { name: /Yes, I checked/ }));
    await waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(contactsSetVerified).toHaveBeenCalledWith(contact.node_id, true);
  });

  it("cancel calls nothing", async () => {
    show();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalled();
    expect(contactsSetVerified).not.toHaveBeenCalled();
  });
});
