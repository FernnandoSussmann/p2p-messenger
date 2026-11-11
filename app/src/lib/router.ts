import { writable } from "svelte/store";
import type { Component } from "svelte";
import FirstRunWizard from "../views/FirstRunWizard.svelte";
import Contacts from "../views/Contacts.svelte";
import MyInfo from "../views/MyInfo.svelte";
import Network from "../views/Network.svelte";

export type AppView =
  | { kind: "loading" }
  | { kind: "blocked"; message: string }
  | { kind: "wizard" }
  | { kind: "main" };

export type TabId = "contacts" | "my-info" | "network";

export interface Tab {
  id: TabId;
  label: string;
  component: Component;
}

export const appView = writable<AppView>({ kind: "loading" });
export const activeTab = writable<TabId>("contacts");

// FirstRunWizard receives `onDone` (see views/FirstRunWizard.svelte).
export const wizardView: Component<{ onDone: () => void }> = FirstRunWizard;

// Tabs rendered by MainScreen; each component takes no props.
export const tabs: Tab[] = [
  { id: "contacts", label: "Contacts", component: Contacts },
  { id: "my-info", label: "My Info", component: MyInfo },
  { id: "network", label: "Network", component: Network },
];
