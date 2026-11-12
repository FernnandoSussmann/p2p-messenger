import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const status = vi.fn();
const myInfo = vi.fn();
const provision = vi.fn();
const setName = vi.fn();
vi.mock("./identity", () => ({
  identityStatus: () => status(),
  identityMyInfo: () => myInfo(),
  identityProvision: () => provision(),
  identitySetDisplayName: (n: string) => setName(n),
  describeError: () => "boom",
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
    for (const m of [status, myInfo, provision, setName]) m.mockReset();
    myInfo.mockResolvedValue(info("Ada"));
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
});
