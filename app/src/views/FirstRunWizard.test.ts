import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const provision = vi.fn();
const setName = vi.fn();
vi.mock("../identity", () => ({
  identityProvision: () => provision(),
  identitySetDisplayName: (n: string) => setName(n),
  describeError: (e: { type?: string }) => `err:${e?.type}`,
}));

import FirstRunWizard from "./FirstRunWizard.svelte";

async function toNameStep() {
  const onDone = vi.fn();
  provision.mockResolvedValue({ node_id: "n", account_id: "a" });
  render(FirstRunWizard, { onDone });
  await fireEvent.click(screen.getByRole("button", { name: "Create identity" }));
  await screen.findByLabelText("Display name");
  return onDone;
}

async function submit(value: string) {
  await fireEvent.input(screen.getByLabelText("Display name"), { target: { value } });
  await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
}

describe("FirstRunWizard", () => {
  beforeEach(() => {
    provision.mockReset();
    setName.mockReset();
  });
  afterEach(cleanup);

  it("blocks whitespace-only names without calling core", async () => {
    const onDone = await toNameStep();
    await submit("   ");
    expect((await screen.findByRole("alert")).textContent).toContain("empty");
    expect(setName).not.toHaveBeenCalled();
    expect(onDone).not.toHaveBeenCalled();
  });

  it.each([
    ["TooLong", "too long"],
    ["ControlCharacter", "control characters"],
    ["BidiOverride", "direction"],
    ["Empty", "empty"],
  ])("blocks on core rejection %s", async (kind, text) => {
    const onDone = await toNameStep();
    setName.mockRejectedValue({ type: "InvalidDisplayName", kind });
    await submit("x");
    expect((await screen.findByRole("alert")).textContent).toContain(text);
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Display name")).toBeTruthy();
  });

  it("advances on a valid name", async () => {
    const onDone = await toNameStep();
    setName.mockResolvedValue(undefined);
    await submit("Alice");
    await screen.findByText("All set");
    expect(setName).toHaveBeenCalledWith("Alice");
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("routes AlreadyProvisioned to onDone", async () => {
    const onDone = vi.fn();
    provision.mockRejectedValue({ type: "AlreadyProvisioned" });
    render(FirstRunWizard, { onDone });
    await fireEvent.click(screen.getByRole("button", { name: "Create identity" }));
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
  });

  it("shows a recoverable error on provision failure and allows retry", async () => {
    const onDone = vi.fn();
    provision.mockRejectedValue({ type: "RecoverableErrorState", reason: "disk" });
    render(FirstRunWizard, { onDone });
    await fireEvent.click(screen.getByRole("button", { name: "Create identity" }));
    expect((await screen.findByRole("alert")).textContent).toContain("RecoverableErrorState");
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Create identity" })).toBeTruthy();
  });
});
