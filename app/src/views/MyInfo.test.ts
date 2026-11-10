import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../identity", async (orig) => ({
  ...(await orig<typeof import("../identity")>()),
  identityMyInfo: vi.fn(),
  identitySetDisplayName: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn() }));

import MyInfo from "./MyInfo.svelte";
import { identityMyInfo, identitySetDisplayName } from "../identity";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

const dto = {
  display_name: "Alice",
  node_id: "nodeid-abcdef0123456789",
  ticket: "ticket-xyz-123",
  qr_svg: '<svg data-testid="qr-svg"></svg>',
};

afterEach(cleanup);

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(identityMyInfo).mockResolvedValue({ ...dto });
});

describe("MyInfo", () => {
  it("renders name, NodeId, ticket and QR", async () => {
    render(MyInfo);
    expect(await screen.findByTestId("display-name")).toHaveTextContent("Alice");
    expect(screen.getByTestId("node-id")).toHaveTextContent(dto.node_id);
    expect(screen.getByTestId("ticket")).toHaveTextContent(dto.ticket);
    expect(screen.getByTestId("qr-svg")).toBeTruthy();
    expect(identityMyInfo).toHaveBeenCalledTimes(1);
  });

  it("copies exactly the ticket and the NodeId", async () => {
    render(MyInfo);
    await fireEvent.click(await screen.findByRole("button", { name: "Copy ticket" }));
    expect(writeText).toHaveBeenLastCalledWith(dto.ticket);
    await fireEvent.click(screen.getByRole("button", { name: "Copy NodeId" }));
    expect(writeText).toHaveBeenLastCalledWith(dto.node_id);
    expect(writeText).toHaveBeenCalledTimes(2);
  });

  it("renders an HTML name as inert text", async () => {
    const evil = "<script>window.__pwned=1</script><b>x</b>";
    vi.mocked(identityMyInfo).mockResolvedValue({ ...dto, display_name: evil });
    const { container } = render(MyInfo);
    expect(await screen.findByTestId("display-name")).toHaveTextContent(evil);
    expect(container.querySelector("script")).toBeNull();
    expect(screen.getByTestId("display-name").querySelector("b")).toBeNull();
  });

  it("shows an inline error for an invalid name", async () => {
    vi.mocked(identitySetDisplayName).mockRejectedValue({
      type: "InvalidDisplayName",
      kind: "Empty",
    });
    render(MyInfo);
    const input = await screen.findByLabelText("Display name");
    await fireEvent.input(input, { target: { value: "" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Invalid display name (Empty)");
  });

  it("saves a valid name and reloads", async () => {
    vi.mocked(identitySetDisplayName).mockResolvedValue(undefined);
    render(MyInfo);
    const input = await screen.findByLabelText("Display name");
    await fireEvent.input(input, { target: { value: "Bob" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(identitySetDisplayName).toHaveBeenCalledWith("Bob"));
    await waitFor(() => expect(identityMyInfo).toHaveBeenCalledTimes(2));
  });
});
