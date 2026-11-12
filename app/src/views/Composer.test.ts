import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";
import Composer from "./Composer.svelte";
import { MAX_BODY_BYTES } from "../chat";

const onSend = vi.fn();
const box = () => screen.getByLabelText("Message") as HTMLTextAreaElement;
const type = (v: string) => fireEvent.input(box(), { target: { value: v } });
const send = () => screen.getByRole("button", { name: "Send" });

afterEach(cleanup);
beforeEach(() => {
  onSend.mockReset();
  onSend.mockResolvedValue(undefined);
  render(Composer, { onSend });
});

describe("Composer", () => {
  it("send_disabled_for_empty_and_whitespace", async () => {
    expect(send()).toBeDisabled();
    await type("  \n ");
    expect(send()).toBeDisabled();
    await type("hi");
    expect(send()).toBeEnabled();
  });

  it("send_disabled_over_byte_limit_with_counter", async () => {
    await type("a".repeat(100));
    expect(screen.queryByTestId("byte-counter")).toBeNull();
    await type("a".repeat(Math.ceil(MAX_BODY_BYTES * 0.95)));
    expect(screen.getByTestId("byte-counter")).toBeTruthy();
    expect(send()).toBeEnabled();
    await type("👋".repeat(MAX_BODY_BYTES / 4 + 1));
    expect(send()).toBeDisabled();
  });

  it("ctrl_enter_sends_enter_inserts_newline", async () => {
    await type("hello\nworld");
    await fireEvent.keyDown(box(), { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
    await fireEvent.keyDown(box(), { key: "Enter", ctrlKey: true });
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("hello\nworld"));
    await type("again");
    await fireEvent.keyDown(box(), { key: "Enter", metaKey: true });
    await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2));
  });

  it("error_keeps_text_and_shows_alert", async () => {
    onSend.mockRejectedValue({ type: "NotAContact" });
    await type("secret text");
    await fireEvent.click(send());
    expect((await screen.findByRole("alert")).textContent).toBe("You can only message your contacts.");
    expect(box().value).toBe("secret text");
  });

  it("success_clears", async () => {
    await type("hi");
    await fireEvent.click(send());
    await waitFor(() => expect(box().value).toBe(""));
    expect(send()).toBeDisabled();
  });
});
