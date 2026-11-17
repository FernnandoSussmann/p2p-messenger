import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import ThemePicker from "./ThemePicker.svelte";

afterEach(cleanup);

describe("ThemePicker", () => {
  it("renders a named radiogroup with the current theme checked", () => {
    render(ThemePicker, { current: "dark", onChoose: vi.fn() });
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeTruthy();
    const radios = screen.getAllByRole("radio");
    expect(radios.map((r) => r.getAttribute("aria-label"))).toEqual(["Midnight", "Dark", "Light"]);
    expect(screen.getByRole("radio", { name: "Dark" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: "Light" }).getAttribute("aria-checked")).toBe("false");
  });

  it("click chooses", async () => {
    const onChoose = vi.fn();
    render(ThemePicker, { current: "midnight", onChoose });
    await fireEvent.click(screen.getByRole("radio", { name: "Light" }));
    expect(onChoose).toHaveBeenCalledWith("light");
  });

  it("arrows move focus and wrap", async () => {
    const onChoose = vi.fn();
    render(ThemePicker, { current: "midnight", onChoose });
    screen.getByRole("radio", { name: "Midnight" }).focus();
    await fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    expect(onChoose).toHaveBeenLastCalledWith("light");
    expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Light" }));
    await fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(onChoose).toHaveBeenLastCalledWith("midnight");
    expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Midnight" }));
  });
});
