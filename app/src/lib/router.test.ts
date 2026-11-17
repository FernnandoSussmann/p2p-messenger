import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { back, resetScreen, screen, showChat, showContactInfo, showTab } from "./router";

beforeEach(resetScreen);

describe("router", () => {
  it("starts on Contacts", () => {
    expect(get(screen)).toEqual({ kind: "tab", tab: "contacts" });
  });

  it("showChat opens a drill-in and back returns to Contacts", () => {
    showChat("n1");
    expect(get(screen)).toEqual({ kind: "chat", nodeId: "n1" });
    back();
    expect(get(screen)).toEqual({ kind: "tab", tab: "contacts" });
  });

  it("showTab from a drill-in goes to that tab and clears it", () => {
    showChat("n1");
    showTab("network");
    expect(get(screen)).toEqual({ kind: "tab", tab: "network" });
  });

  it("resetScreen returns home", () => {
    showTab("my-info");
    resetScreen();
    expect(get(screen)).toEqual({ kind: "tab", tab: "contacts" });
  });
});

describe("contact-info", () => {
  it("back returns to the same chat, then to Contacts", () => {
    showContactInfo("abc");
    back();
    expect(get(screen)).toEqual({ kind: "chat", nodeId: "abc" });
    back();
    expect(get(screen)).toEqual({ kind: "tab", tab: "contacts" });
  });

  it("showTab from contact-info clears the drill-in", () => {
    showContactInfo("abc");
    showTab("network");
    expect(get(screen)).toEqual({ kind: "tab", tab: "network" });
    back();
    expect(get(screen)).toEqual({ kind: "tab", tab: "contacts" });
  });
});
