import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { get } from "svelte/store";
import { render, screen, cleanup } from "@testing-library/svelte";

vi.mock("../lib/contactsStore", async () => {
  const { writable } = await import("svelte/store");
  return {
    contacts: writable([]),
    contactsError: writable(null),
    contactsNotice: writable<string | null>(null),
    refreshContacts: vi.fn(async () => {}),
  };
});

import Contacts from "./Contacts.svelte";
import { contacts, contactsNotice } from "../lib/contactsStore";

const mk = (node_id: string, nickname: string) => ({ node_id, nickname, self_name: null, verified: false, added_at_ms: 0, last_seen_ms: null });

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  contacts.set([]);
  contactsNotice.set(null);
});

describe("Contacts", () => {
  it("lists contacts from the store", () => {
    contacts.set([mk("a", "Ann"), mk("b", "Bob")]);
    render(Contacts);
    expect(screen.getAllByTestId("contact-nickname").map((e) => e.textContent)).toEqual(["Ann", "Bob"]);
  });

  it("has no LAN list", () => {
    render(Contacts);
    expect(screen.queryByText("On this network")).toBeNull();
  });

  it("shows the empty state with the three ways to add", () => {
    render(Contacts);
    const text = screen.getByText(/No contacts yet/).textContent!.replace(/\s+/g, " ");
    expect(text).toBe(
      "No contacts yet. Add someone from Discover, connect with a ticket in the Network tab, or accept an incoming request.",
    );
    expect(screen.getByText("Discover").tagName).toBe("EM");
    expect(text).toContain("ticket in the Network tab");
    expect(text).toContain("accept an incoming request");
  });

  it("shows the notice from the store and clears it on destroy", () => {
    contactsNotice.set("Removed “Ann”. Your message history with them was kept.");
    const { unmount } = render(Contacts);
    expect(screen.getByRole("status").textContent).toBe("Removed “Ann”. Your message history with them was kept.");
    unmount();
    expect(get(contactsNotice)).toBeNull();
  });

  it("shows no status when there is no notice", () => {
    render(Contacts);
    expect(screen.queryByRole("status")).toBeNull();
  });
});
