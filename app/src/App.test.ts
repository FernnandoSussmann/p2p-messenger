import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const status = vi.fn();
const myInfo = vi.fn();
const provision = vi.fn();
const setName = vi.fn();
const themeGet = vi.fn();
vi.mock("./identity", () => ({
  identityStatus: () => status(),
  identityMyInfo: () => myInfo(),
  identityProvision: () => provision(),
  identitySetDisplayName: (n: string) => setName(n),
  describeError: () => "boom",
  identityThemeGet: () => themeGet(),
  identityThemeSet: vi.fn(),
}));
const storage = vi.fn();
const storageRetry = vi.fn();
vi.mock("./storage", () => ({
  storageStatus: () => storage(),
  storageRetry: () => storageRetry(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn() }));

const info = (display_name: string | null) => ({
  display_name,
  node_id: "nodeid-abc",
  ticket: "ticket-xyz",
  qr_svg: "<svg></svg>",
});

import App from "./App.svelte";

describe("App view machine", () => {
  beforeEach(() => {
    for (const m of [status, myInfo, provision, setName, storage, storageRetry, themeGet]) m.mockReset();
    myInfo.mockResolvedValue(info("Ada"));
    storage.mockResolvedValue({ type: "Ready" });
    themeGet.mockResolvedValue({ theme: "midnight" });
  });
  afterEach(cleanup);

  it("shows the wizard when absent", async () => {
    status.mockResolvedValue({ type: "Absent" });
    render(App);
    await screen.findByText("Welcome");
  });

  it("shows main screen with My Info tab when present", async () => {
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    render(App);
    await screen.findByRole("tab", { name: "My Info" });
  });

  it("applies the stored theme before showing main", async () => {
    themeGet.mockResolvedValue({ theme: "light" });
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    render(App);
    await screen.findByRole("tab", { name: "My Info" });
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("loads the theme before the tab list appears", async () => {
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    render(App);
    await screen.findByRole("tab", { name: "My Info" });
    expect(themeGet).toHaveBeenCalledTimes(1);
    expect(themeGet.mock.invocationCallOrder[0]).toBeLessThan(myInfo.mock.invocationCallOrder[0] ?? Infinity);
  });

  it("a rejected theme read still shows main with the midnight theme", async () => {
    themeGet.mockRejectedValue({ type: "Internal" });
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    render(App);
    await screen.findByRole("tab", { name: "My Info" });
    expect(document.documentElement.dataset.theme).toBe("midnight");
  });

  it("never reads the theme when the identity is absent", async () => {
    status.mockResolvedValue({ type: "Absent" });
    render(App);
    await screen.findByText("Welcome");
    expect(themeGet).not.toHaveBeenCalled();
  });

  it("blocks on error with retry and never provisions", async () => {
    status.mockResolvedValue({ type: "Error", reason: "locked" });
    render(App);
    await screen.findByText("locked");
    status.mockResolvedValue({ type: "Absent" });
    screen.getByRole("button", { name: "Retry" }).click();
    await waitFor(() => expect(screen.getByText("Welcome")).toBeTruthy());
  });

  it("relaunch with Present status and a stored name goes to main, not the wizard", async () => {
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    render(App);
    await fireEvent.click(await screen.findByRole("tab", { name: "My Info" }));
    expect((await screen.findByTestId("display-name")).textContent).toBe("Ada");
    expect(screen.queryByText("Welcome")).toBeNull();
    expect(provision).not.toHaveBeenCalled();
    expect(setName).not.toHaveBeenCalled();
  });

  it("Present but no stored name still reaches My Info and can be edited", async () => {
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    myInfo.mockResolvedValue(info(null));
    setName.mockResolvedValue(undefined);
    render(App);
    await fireEvent.click(await screen.findByRole("tab", { name: "My Info" }));
    const input = (await screen.findByLabelText("Display name")) as HTMLInputElement;
    expect(screen.queryByText("Welcome")).toBeNull();
    myInfo.mockResolvedValue(info("Bob"));
    await fireEvent.input(input, { target: { value: "Bob" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(setName).toHaveBeenCalledWith("Bob"));
    expect(provision).not.toHaveBeenCalled();
  });

  it("shows the storage screen, not main, when the store is unavailable", async () => {
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    storage.mockResolvedValue({ type: "Error", kind: "Missing", reason: "db is gone" });
    render(App);
    await screen.findByText("Encrypted storage unavailable");
    expect(screen.getByText("db is gone")).toBeTruthy();
    expect(screen.queryByRole("tab", { name: "My Info" })).toBeNull();
  });

  it("Retry leading to Ready shows main", async () => {
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    storage.mockResolvedValue({ type: "Error", kind: "Missing", reason: "db is gone" });
    render(App);
    await screen.findByText("Encrypted storage unavailable");
    storage.mockResolvedValue({ type: "Ready" });
    storageRetry.mockResolvedValue({ type: "Ready" });
    await fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByRole("tab", { name: "My Info" });
  });

  it("re-checks storage when the wizard finishes", async () => {
    status.mockResolvedValue({ type: "Absent" });
    provision.mockResolvedValue({ node_id: "n", account_id: "a" });
    setName.mockResolvedValue(undefined);
    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "Create identity" }));
    const input = await screen.findByLabelText("Display name");
    status.mockResolvedValue({ type: "Present", account_id: "a", node_id: "n" });
    storage.mockResolvedValue({ type: "Error", kind: "Io", reason: "no access" });
    await fireEvent.input(input, { target: { value: "Ada" } });
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByText("Encrypted storage unavailable");
    expect(screen.getByText("no access")).toBeTruthy();
  });
});
