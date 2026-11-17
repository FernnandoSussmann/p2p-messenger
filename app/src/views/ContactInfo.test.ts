import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { get } from "svelte/store";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../chat", async (orig) => ({
  ...(await orig<typeof import("../chat")>()),
  chatHistory: vi.fn(),
  historySearch: vi.fn(),
  historyClear: vi.fn(),
}));
vi.mock("../contacts", async (orig) => ({
  ...(await orig<typeof import("../contacts")>()),
  contactsList: vi.fn(),
  contactsRename: vi.fn(),
  contactsSetVerified: vi.fn(),
  contactsRemove: vi.fn(),
}));
vi.mock("../net", async (orig) => ({
  ...(await orig<typeof import("../net")>()),
  netConnectTicket: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));

import ContactInfo from "./ContactInfo.svelte";
import { historyClear, historySearch, type MessageDto } from "../chat";
import { contactsList, contactsRemove, contactsRename, contactsSetVerified, type ContactDto } from "../contacts";
import { netConnectTicket } from "../net";
import { contacts, contactsNotice } from "../lib/contactsStore";
import { connections, lanPeers } from "../lib/netStore";
import { screen as routerScreen, showContactInfo } from "../lib/router";

const ID = "a".repeat(64);
const mkContact = (o: Partial<ContactDto> = {}): ContactDto => ({
  node_id: ID, nickname: "Bob", self_name: null, verified: false, added_at_ms: 0, last_seen_ms: null, ...o,
});
const mk = (i: number, o: Partial<MessageDto> = {}): MessageDto => ({
  node_id: ID, id: `m${i}`, seq: i, cursor: `c${i}`, direction: "in", body: `body ${i}`,
  sent_at_ms: 1_700_000_000_000 + i * 1000, received_at_ms: null, delivered_at_ms: null, status: "received", ...o,
});

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  contacts.set([mkContact()]);
  contactsNotice.set(null);
  connections.set({});
  lanPeers.set([]);
  showContactInfo(ID);
});

describe("ContactInfo", () => {
  it("shows_name_badge_identity_and_unknown_self_name", async () => {
    render(ContactInfo, { nodeId: ID });
    expect(screen.getByRole("heading", { name: "Bob" })).toBeTruthy();
    expect(screen.getByTestId("badge-unverified")).toBeTruthy();
    expect(screen.getByText("They did not send a name")).toBeTruthy();
    expect(screen.getByText("Not connected")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Show full NodeId" }));
    expect(screen.getByTestId("node-id-full").textContent).toBe(ID);
    expect(screen.getByText("Select it and use your system's copy shortcut.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /copy/i })).toBeNull();
  });

  it("shows_peer_self_name_as_text", () => {
    contacts.set([mkContact({ self_name: "<i>Bobby</i>" })]);
    render(ContactInfo, { nodeId: ID });
    expect(screen.getByTestId("peer-name").textContent).toBe("<i>Bobby</i>");
    expect(screen.getByTestId("self-name-context").textContent).toContain("chosen by them, not verified");
  });

  it("back_returns_to_conversation", async () => {
    render(ContactInfo, { nodeId: ID });
    await fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(get(routerScreen)).toEqual({ kind: "chat", nodeId: ID });
  });

  it("connection_state_and_connect_only_when_ticket_and_not_connected", async () => {
    connections.set({ [ID]: { node_id: ID, state: "failed", direction: "outbound", reason: "timeout" } as never });
    lanPeers.set([{ node_id: ID, display_name: null, verified: false, ticket: "tk" }]);
    vi.mocked(netConnectTicket).mockResolvedValue({ node_id: ID });
    render(ContactInfo, { nodeId: ID });
    expect(screen.getByTestId("contact-connection").textContent).toBe("outbound | failed | timeout");
    await fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(netConnectTicket).toHaveBeenCalledWith("tk");
    connections.set({ [ID]: { node_id: ID, state: "connected", direction: "outbound" } as never });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Connect" })).toBeNull());
  });

  it("rename_saves_and_refreshes", async () => {
    vi.mocked(contactsRename).mockResolvedValue(mkContact({ nickname: "Rob" }));
    vi.mocked(contactsList).mockResolvedValue([mkContact({ nickname: "Rob" })]);
    render(ContactInfo, { nodeId: ID });
    await fireEvent.click(screen.getByRole("button", { name: "Rename" }));
    const box = screen.getByLabelText("Your name for them");
    await fireEvent.input(box, { target: { value: "Rob" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save nickname" }));
    await waitFor(() => expect(contactsRename).toHaveBeenCalledWith(ID, "Rob"));
    expect(await screen.findByRole("heading", { name: "Rob" })).toBeTruthy();
    expect(screen.queryByLabelText("Your name for them")).toBeNull();
  });

  it("unverify_calls_command", async () => {
    contacts.set([mkContact({ verified: true })]);
    vi.mocked(contactsSetVerified).mockResolvedValue(mkContact());
    vi.mocked(contactsList).mockResolvedValue([mkContact()]);
    render(ContactInfo, { nodeId: ID });
    await fireEvent.click(screen.getByRole("button", { name: "Mark unverified" }));
    await waitFor(() => expect(contactsSetVerified).toHaveBeenCalledWith(ID, false));
  });

  it("mark_verified_opens_dialog", async () => {
    render(ContactInfo, { nodeId: ID });
    await fireEvent.click(screen.getByRole("button", { name: "Mark verified…" }));
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("remove_sets_notice_and_goes_to_contacts_without_early_redirect", async () => {
    vi.mocked(contactsRemove).mockResolvedValue({ history_erased: true } as never);
    vi.mocked(contactsList).mockResolvedValue([]);
    render(ContactInfo, { nodeId: ID });
    await fireEvent.click(screen.getByRole("button", { name: "Remove…" }));
    expect(screen.getByRole("dialog", { name: "Remove contact" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Remove contact" }));
    await waitFor(() => expect(contactsRemove).toHaveBeenCalled());
    await waitFor(() => expect(get(contactsNotice)).toBe("Removed “Bob”. Your message history with them was deleted."));
    expect(get(routerScreen)).toEqual({ kind: "tab", tab: "contacts" });
  });

  it("external_removal_while_remove_dialog_open_disables_everything_but_back", async () => {
    render(ContactInfo, { nodeId: ID });
    await fireEvent.click(screen.getByRole("button", { name: "Remove…" }));
    contacts.set([mkContact({ node_id: "b".repeat(64) })]);
    await waitFor(() => expect(screen.getByRole("button", { name: "Rename" })).toBeDisabled());
    for (const el of [
      screen.getByRole("button", { name: "Remove…" }),
      screen.getByRole("button", { name: "Clear history…" }),
      screen.getByLabelText("Search this conversation"),
      screen.getByRole("button", { name: "Remove contact" }),
    ]) {
      expect(el).toBeDisabled();
    }
    expect(screen.getByRole("button", { name: "Back" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /Mark verified/ })).toBeNull();
  });

  it("removed_contact_redirects_to_contacts_and_renders_nothing", async () => {
    contacts.set([mkContact({ node_id: "b".repeat(64) })]);
    const { container } = render(ContactInfo, { nodeId: ID });
    await waitFor(() => expect(get(routerScreen)).toEqual({ kind: "tab", tab: "contacts" }));
    expect(container.textContent?.trim()).toBe("");
    expect(historySearch).not.toHaveBeenCalled();
  });

  it("search_shows_results_as_text", async () => {
    render(ContactInfo, { nodeId: ID });
    const hit = mk(9, { body: "<b>needle</b>" });
    vi.mocked(historySearch).mockResolvedValue([hit]);
    const box = screen.getByLabelText("Search this conversation");
    await fireEvent.input(box, { target: { value: "needle" } });
    const list = await screen.findByTestId("search-results");
    expect(historySearch).toHaveBeenCalledWith("needle", ID);
    expect(list.querySelector("b")).toBeNull();
    expect(list.textContent).toContain("<b>needle</b>");
    expect(list.textContent).toContain("Them");
    await fireEvent.keyDown(box, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("search-results")).toBeNull());
    expect((box as HTMLInputElement).value).toBe("");
  });

  it("search_no_matches", async () => {
    render(ContactInfo, { nodeId: ID });
    vi.mocked(historySearch).mockResolvedValue([]);
    await fireEvent.input(screen.getByLabelText("Search this conversation"), { target: { value: "zz" } });
    expect(await screen.findByText("No matches.")).toBeTruthy();
  });

  it("clear_history_is_reachable_for_contacts", () => {
    render(ContactInfo, { nodeId: ID });
    expect(screen.getByRole("button", { name: "Clear history…" })).toBeEnabled();
  });

  it("clear_history_empties_search_and_confirms", async () => {
    vi.mocked(historyClear).mockResolvedValue(undefined);
    render(ContactInfo, { nodeId: ID });
    vi.mocked(historySearch).mockResolvedValue([mk(1)]);
    await fireEvent.input(screen.getByLabelText("Search this conversation"), { target: { value: "body" } });
    await screen.findByTestId("search-results");
    const open = screen.getByRole("button", { name: "Clear history…" });
    await fireEvent.click(open);
    expect(open).toBeDisabled();
    expect(screen.getByRole("dialog", { name: "Clear history" }).textContent).toContain("Bob");
    await fireEvent.click(screen.getByRole("button", { name: "Clear history" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("History cleared."));
    expect(historyClear).toHaveBeenCalledWith(ID);
    expect(screen.queryByTestId("search-results")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("remove_notice_for_kept_history", async () => {
    vi.mocked(contactsRemove).mockResolvedValue({ history_erased: false } as never);
    vi.mocked(contactsList).mockResolvedValue([]);
    render(ContactInfo, { nodeId: ID });
    await fireEvent.click(screen.getByRole("button", { name: "Remove…" }));
    await fireEvent.click(screen.getByRole("button", { name: "Remove contact" }));
    await waitFor(() => expect(get(contactsNotice)).toBe("Removed “Bob”. Your message history with them was kept."));
  });

  it("stale_search_response_is_dropped", async () => {
    render(ContactInfo, { nodeId: ID });
    let first!: (v: MessageDto[]) => void;
    vi.mocked(historySearch)
      .mockImplementationOnce(() => new Promise((r) => (first = r)))
      .mockResolvedValueOnce([mk(2, { body: "fresh" })]);
    const box = screen.getByLabelText("Search this conversation");
    await fireEvent.input(box, { target: { value: "a" } });
    await fireEvent.input(box, { target: { value: "ab" } });
    await screen.findByText("fresh");
    first([mk(1, { body: "stale" })]);
    await Promise.resolve();
    expect(screen.queryByText("stale")).toBeNull();
    expect(screen.getByText("fresh")).toBeTruthy();
  });

  it("escape_clears_search", async () => {
    render(ContactInfo, { nodeId: ID });
    vi.mocked(historySearch).mockResolvedValue([mk(1)]);
    const box = screen.getByLabelText("Search this conversation") as HTMLInputElement;
    await fireEvent.input(box, { target: { value: "body" } });
    await screen.findByTestId("search-results");
    await fireEvent.keyDown(box, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("search-results")).toBeNull());
    expect(box.value).toBe("");
  });

  it("search_error_shows_alert", async () => {
    render(ContactInfo, { nodeId: ID });
    vi.mocked(historySearch).mockRejectedValue({ kind: "Internal" });
    await fireEvent.input(screen.getByLabelText("Search this conversation"), { target: { value: "x" } });
    expect((await screen.findByRole("alert")).textContent?.length).toBeGreaterThan(0);
    expect(screen.queryByTestId("search-results")).toBeNull();
  });

  it("no_connect_button_when_connected", () => {
    connections.set({ [ID]: { node_id: ID, state: "connected", direction: "outbound" } as never });
    lanPeers.set([{ node_id: ID, display_name: null, verified: false, ticket: "tk" }]);
    render(ContactInfo, { nodeId: ID });
    expect(screen.queryByRole("button", { name: "Connect" })).toBeNull();
    expect(netConnectTicket).not.toHaveBeenCalled();
  });

  it("does_not_redirect_before_contacts_load", async () => {
    contacts.set([]);
    render(ContactInfo, { nodeId: ID });
    await Promise.resolve();
    expect(get(routerScreen)).toEqual({ kind: "contact-info", nodeId: ID });
    contacts.set([mkContact()]);
    await screen.findByRole("heading", { name: "Bob" });
  });

  it("removed_elsewhere_after_seen_redirects_without_command", async () => {
    render(ContactInfo, { nodeId: ID });
    contacts.set([]);
    await waitFor(() => expect(get(routerScreen)).toEqual({ kind: "tab", tab: "contacts" }));
    expect(contactsRemove).not.toHaveBeenCalled();
    expect(historyClear).not.toHaveBeenCalled();
  });
});
