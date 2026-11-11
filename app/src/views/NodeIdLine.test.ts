import { describe, it, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import NodeIdLine from "./NodeIdLine.svelte";

const ID = "0123456789" + "x".repeat(48) + "abcdef";

afterEach(cleanup);

describe("NodeIdLine", () => {
  it("shows a truncated id with the full id in title", () => {
    render(NodeIdLine, { nodeId: ID });
    const short = screen.getByTestId("node-id-short");
    expect(short.textContent).toBe("0123456789…abcdef");
    expect(short.getAttribute("title")).toBe(ID);
    expect(screen.queryByTestId("node-id-full")).toBeNull();
  });

  it("Show full reveals the whole id as selectable text", async () => {
    render(NodeIdLine, { nodeId: ID });
    const toggle = screen.getByRole("button", { name: "Show full NodeId" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    const full = screen.getByTestId("node-id-full");
    expect(full.tagName).toBe("CODE");
    expect(full.textContent).toBe(ID);
    expect(screen.getByText("Select it and use your system's copy shortcut.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Hide full NodeId" }));
    expect(screen.queryByTestId("node-id-full")).toBeNull();
  });

  it("renders a hostile id inert", async () => {
    const hostile = '<img src=x onerror="alert(1)">' + "‮" + "z".repeat(30);
    const { container } = render(NodeIdLine, { nodeId: hostile });
    await fireEvent.click(screen.getByRole("button", { name: "Show full NodeId" }));
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByTestId("node-id-full").textContent).toBe(hostile);
  });
});
