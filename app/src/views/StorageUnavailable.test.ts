import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const retry = vi.fn();
vi.mock("../storage", () => ({ storageRetry: () => retry() }));

import StorageUnavailable from "./StorageUnavailable.svelte";

describe("StorageUnavailable", () => {
  beforeEach(() => retry.mockReset());
  afterEach(cleanup);

  it("shows the reason and the reassurance", () => {
    render(StorageUnavailable, { message: "key is wrong", onResolved: vi.fn() });
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(screen.getByText("Encrypted storage unavailable")).toBeTruthy();
    expect(screen.getByText("key is wrong")).toBeTruthy();
    expect(screen.getByText(/were not deleted, reset or replaced/)).toBeTruthy();
  });

  it("offers Retry only, no reset or recreate action", () => {
    render(StorageUnavailable, { message: "m", onResolved: vi.fn() });
    expect(screen.getAllByRole("button").map((b) => b.textContent)).toEqual(["Retry"]);
  });

  it("an Error result updates the text and does not resolve", async () => {
    const onResolved = vi.fn();
    retry.mockResolvedValue({ type: "Error", kind: "Corrupt", reason: "still damaged" });
    render(StorageUnavailable, { message: "first", onResolved });
    await fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("still damaged");
    expect(onResolved).not.toHaveBeenCalled();
  });

  it("Ready calls onResolved", async () => {
    const onResolved = vi.fn();
    retry.mockResolvedValue({ type: "Ready" });
    render(StorageUnavailable, { message: "first", onResolved });
    await fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await vi.waitFor(() => expect(onResolved).toHaveBeenCalledTimes(1));
    expect(retry).toHaveBeenCalledTimes(1);
  });
});
