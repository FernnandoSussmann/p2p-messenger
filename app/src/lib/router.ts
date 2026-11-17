import { writable } from "svelte/store";

export type AppView =
  | { kind: "loading" }
  | { kind: "blocked"; message: string }
  | { kind: "storage"; message: string }
  | { kind: "wizard" }
  | { kind: "main" };

export type TabId = "contacts" | "discover" | "network" | "my-info";
export type Screen = { kind: "tab"; tab: TabId } | { kind: "chat"; nodeId: string };

const HOME: Screen = { kind: "tab", tab: "contacts" };

export const appView = writable<AppView>({ kind: "loading" });
export const screen = writable<Screen>(HOME);

/** Selecting a tab also clears any drill-in screen. */
export const showTab = (tab: TabId) => screen.set({ kind: "tab", tab });
export const showChat = (nodeId: string) => screen.set({ kind: "chat", nodeId });
export const resetScreen = () => screen.set(HOME);

/** Back from a drill-in screen: chat returns to Contacts. */
export function back(): void {
  screen.set(HOME);
}
