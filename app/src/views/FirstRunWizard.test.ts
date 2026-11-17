import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { get } from "svelte/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
const provision = vi.fn();
const setName = vi.fn();
const setTheme = vi.fn();
vi.mock("../identity", () => ({
  identityProvision: () => provision(),
  identitySetDisplayName: (n: string) => setName(n),
  identityThemeSet: (t: string) => setTheme(t),
  describeError: (e: { type?: string }) => `err:${e?.type}`,
}));

import { pendingTheme } from "../lib/theme";
import FirstRunWizard from "./FirstRunWizard.svelte";

function setup() {
  const onDone = vi.fn(() => calls.push("done"));
  render(FirstRunWizard, { onDone });
  return onDone;
}

const continueBtn = () => screen.queryByRole("button", { name: "Continue" });

async function typeName(value: string) {
  await fireEvent.input(screen.getByLabelText("Display name"), { target: { value } });
}

async function submit(value: string) {
  await typeName(value);
  await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
}

describe("FirstRunWizard", () => {
  beforeEach(() => {
    calls.length = 0;
    pendingTheme.set(null);
    provision.mockReset().mockImplementation(async () => {
      calls.push("provision");
      return { node_id: "n", account_id: "a" };
    });
    setName.mockReset().mockImplementation(async () => void calls.push("name"));
    setTheme.mockReset().mockImplementation(async () => void calls.push("theme"));
  });
  afterEach(cleanup);

  it("preselects Midnight and applies a chosen theme without saving", async () => {
    setup();
    expect(screen.getByRole("radio", { name: "Midnight" }).getAttribute("aria-checked")).toBe(
      "true",
    );
    await fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(get(pendingTheme)).toBe("dark");
    expect(setTheme).not.toHaveBeenCalled();
  });

  it("drops the removed texts and shows the new one", () => {
    setup();
    expect(screen.getByText("Choose a theme")).toBeTruthy();
    for (const t of ["Choose a display name", "Create identity", "All set"]) {
      expect(screen.queryByText(t)).toBeNull();
    }
  });

  it("shows the arrow only for a non-blank name", async () => {
    setup();
    expect(continueBtn()).toBeNull();
    await typeName("   ");
    expect(continueBtn()).toBeNull();
    await typeName("A");
    expect(continueBtn()).toBeTruthy();
  });

  it("Enter on an empty name shows the Empty message without calling core", async () => {
    setup();
    await fireEvent.submit(screen.getByLabelText("Display name").closest("form")!);
    expect((await screen.findByRole("alert")).textContent).toContain("empty");
    expect(provision).not.toHaveBeenCalled();
  });

  it("happy path: provision, name, theme, onDone in order", async () => {
    const onDone = setup();
    await fireEvent.click(screen.getByRole("radio", { name: "Light" }));
    await submit("Alice");
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(calls).toEqual(["provision", "name", "theme", "done"]);
    expect(setName).toHaveBeenCalledWith("Alice");
    expect(setTheme).toHaveBeenCalledWith("light");
    expect(get(pendingTheme)).toBeNull();
  });

  it("saves Midnight when no theme was picked", async () => {
    const onDone = setup();
    await submit("Alice");
    await vi.waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(setTheme).toHaveBeenCalledWith("midnight");
  });

  it("AlreadyProvisioned calls onDone and writes nothing", async () => {
    provision.mockRejectedValue({ type: "AlreadyProvisioned" });
    const onDone = setup();
    await fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
    await submit("Alice");
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(setName).not.toHaveBeenCalled();
    expect(setTheme).not.toHaveBeenCalled();
    expect(get(pendingTheme)).toBeNull();
  });

  it.each([
    ["TooLong", "too long"],
    ["ControlCharacter", "control characters"],
    ["BidiOverride", "direction"],
    ["InvisibleCharacter", "invisible characters"],
    ["Empty", "empty"],
  ])("blocks on core rejection %s and retry does not re-provision", async (kind, text) => {
    const onDone = setup();
    setName.mockRejectedValueOnce({ type: "InvalidDisplayName", kind });
    await submit("x");
    expect((await screen.findByRole("alert")).textContent).toContain(text);
    expect(onDone).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(provision).toHaveBeenCalledTimes(1);
  });

  it("a theme-save failure still calls onDone and keeps pendingTheme", async () => {
    setTheme.mockRejectedValue({ type: "Internal" });
    const onDone = setup();
    await fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
    await submit("Alice");
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(get(pendingTheme)).toBe("dark");
  });

  it("shows describeError on other provision errors", async () => {
    provision.mockRejectedValue({ type: "RecoverableErrorState", reason: "disk" });
    const onDone = setup();
    await submit("Alice");
    expect((await screen.findByRole("alert")).textContent).toContain("RecoverableErrorState");
    expect(onDone).not.toHaveBeenCalled();
    expect(setName).not.toHaveBeenCalled();
  });
});
