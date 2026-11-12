import { describe, it, expect, vi, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";

import NicknameForm from "./NicknameForm.svelte";

afterEach(cleanup);

const field = () => screen.getByLabelText("Your name for them") as HTMLInputElement;

describe("NicknameForm", () => {
  it("trims and submits", async () => {
    const onSubmit = vi.fn();
    render(NicknameForm, { selfName: "Eve", submitLabel: "Save", onSubmit });
    await fireEvent.input(field(), { target: { value: "  Bobby  " } });
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSubmit).toHaveBeenCalledWith("Bobby");
  });

  it("cancel calls onCancel", async () => {
    const onCancel = vi.fn();
    render(NicknameForm, { selfName: null, submitLabel: "Save", onSubmit: vi.fn(), onCancel });
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalled();
  });

  it("shows the self-asserted name as unverified context", () => {
    render(NicknameForm, { selfName: "Eve", submitLabel: "Save", onSubmit: vi.fn() });
    expect(screen.getByTestId("self-name-context")).toHaveTextContent(
      "They call themselves “Eve” (chosen by them, not verified)",
    );
  });

  it("says so when no name was sent", () => {
    render(NicknameForm, { selfName: null, submitLabel: "Save", onSubmit: vi.fn() });
    expect(screen.getByText("They did not send a name")).toBeTruthy();
  });

  it("initial pre-fills only when given and stays editable", async () => {
    const { unmount } = render(NicknameForm, { selfName: null, submitLabel: "Save", onSubmit: vi.fn() });
    expect(field().value).toBe("");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    unmount();
    render(NicknameForm, {
      initial: "Eve",
      selfName: null,
      submitLabel: "Save",
      onSubmit: vi.fn(),
    });
    expect(field().value).toBe("Eve");
    await fireEvent.input(field(), { target: { value: "Evie" } });
    expect(field().value).toBe("Evie");
  });

  it("is disabled while busy and shows an error", () => {
    render(NicknameForm, {
      initial: "Eve",
      selfName: null,
      submitLabel: "Save",
      busy: true,
      error: "Nope",
      onSubmit: vi.fn(),
    });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Nope");
  });
});
