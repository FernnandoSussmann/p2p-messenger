import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../identity", async (orig) => ({
  ...(await orig<typeof import("../identity")>()),
  identityMyInfo: vi.fn(),
  identitySetDisplayName: vi.fn(),
}));
vi.mock("../net", () => ({ netMyTicket: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn() }));

import MyInfo from "./MyInfo.svelte";
import { identityMyInfo, identitySetDisplayName } from "../identity";
import { netMyTicket } from "../net";
import { listen } from "@tauri-apps/api/event";
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
  vi.mocked(netMyTicket).mockResolvedValue({ ticket: dto.ticket, qr_svg: dto.qr_svg });
  vi.mocked(listen).mockResolvedValue(() => {});
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

  it("shows the live ticket and QR from netMyTicket", async () => {
    vi.mocked(netMyTicket).mockResolvedValue({
      ticket: "live-ticket",
      qr_svg: '<svg data-testid="live-qr"></svg>',
    });
    render(MyInfo);
    expect(await screen.findByTestId("ticket")).toHaveTextContent("live-ticket");
    expect(screen.getByTestId("live-qr")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Copy ticket" }));
    expect(writeText).toHaveBeenLastCalledWith("live-ticket");
  });

  it("falls back to the identity ticket when the network is not ready", async () => {
    vi.mocked(netMyTicket).mockRejectedValue({ type: "NotReady" });
    render(MyInfo);
    expect(await screen.findByTestId("ticket")).toHaveTextContent(dto.ticket);
    expect(screen.getByText(/Network starting/)).toBeTruthy();
  });

  it("re-fetches on my_ticket_changed and unlistens on unmount", async () => {
    let handler: () => void = () => {};
    const unlisten = vi.fn();
    vi.mocked(listen).mockImplementation(async (_name, h) => {
      handler = h as () => void;
      return unlisten;
    });
    const { unmount } = render(MyInfo);
    await screen.findByTestId("ticket");
    expect(listen).toHaveBeenCalledWith("my_ticket_changed", expect.any(Function));
    vi.mocked(netMyTicket).mockResolvedValue({ ticket: "new-ticket", qr_svg: "<svg></svg>" });
    handler();
    await waitFor(() => expect(screen.getByTestId("ticket")).toHaveTextContent("new-ticket"));
    await fireEvent.click(screen.getByRole("button", { name: "Copy ticket" }));
    expect(writeText).toHaveBeenLastCalledWith("new-ticket");
    unmount();
    await waitFor(() => expect(unlisten).toHaveBeenCalled());
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
    expect(await screen.findByRole("alert")).toHaveTextContent("Display name cannot be empty.");
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
