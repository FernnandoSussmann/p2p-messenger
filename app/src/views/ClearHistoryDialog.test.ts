import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

vi.mock("../chat", async (orig) => ({
  ...(await orig<typeof import("../chat")>()),
  historyClear: vi.fn(),
}));

import ClearHistoryDialog from "./ClearHistoryDialog.svelte";
import { historyClear } from "../chat";
import { HISTORY_DELETE_GUARANTEE } from "../lib/historyCopy";

const onDone = vi.fn();
const onCancel = vi.fn();
const show = () => render(ClearHistoryDialog, { nodeId: "id", nickname: "Bob", onDone, onCancel });
const clearBtn = () => screen.getByRole("button", { name: "Clear history" });

afterEach(cleanup);
beforeEach(() => vi.clearAllMocks());

describe("ClearHistoryDialog", () => {
  it("shows the honest guarantee text and focuses Cancel first", () => {
    show();
    const dialog = screen.getByRole("dialog", { name: "Clear history" });
    expect(dialog.textContent).toContain("Clear message history with “Bob”?");
    expect(dialog.textContent).toContain(HISTORY_DELETE_GUARANTEE);
    expect(dialog.textContent).toContain("This cannot be undone.");
    const buttons = screen.getAllByRole("button");
    expect(buttons[0]).toHaveTextContent("Cancel");
    expect(buttons[0]).toHaveFocus();
  });

  it("Cancel does not invoke", async () => {
    show();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalled();
    expect(historyClear).not.toHaveBeenCalled();
  });

  it("Confirm invokes historyClear with the node id", async () => {
    vi.mocked(historyClear).mockResolvedValue(undefined);
    show();
    await fireEvent.click(clearBtn());
    await waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(historyClear).toHaveBeenCalledWith("id");
  });

  it("an error keeps the dialog open with an alert", async () => {
    vi.mocked(historyClear).mockRejectedValue({ type: "HistoryClearFailed" });
    show();
    await fireEvent.click(clearBtn());
    expect((await screen.findByRole("alert")).textContent).toContain("History could not be cleared");
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(onDone).not.toHaveBeenCalled();
  });

  it("buttons are disabled while busy", async () => {
    let finish!: () => void;
    vi.mocked(historyClear).mockReturnValue(new Promise<void>((r) => (finish = r)));
    show();
    await fireEvent.click(clearBtn());
    expect(clearBtn()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    expect(screen.getByRole("status").textContent).toContain("This may take a moment.");
    finish();
    await waitFor(() => expect(onDone).toHaveBeenCalled());
  });
});
