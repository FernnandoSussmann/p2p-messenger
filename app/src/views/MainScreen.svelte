<script lang="ts">
  import { onMount, tick } from "svelte";
  import { back, screen, showTab, type TabId } from "../lib/router";
  import { startNetEvents } from "../lib/netStore";
  import { startContacts } from "../lib/contactsStore";
  import { startChatEvents } from "../lib/chatStore";
  import type { Component } from "svelte";
  import Chat from "./Chat.svelte";
  import Contacts from "./Contacts.svelte";
  import Discover from "./Discover.svelte";
  import IncomingRequestPrompt from "./IncomingRequestPrompt.svelte";
  import MyInfo from "./MyInfo.svelte";
  import NamingPrompt from "./NamingPrompt.svelte";
  import Network from "./Network.svelte";

  const tabs: { id: TabId; label: string; component: Component }[] = [
    { id: "contacts", label: "Contacts", component: Contacts },
    { id: "discover", label: "Discover", component: Discover },
    { id: "network", label: "Network", component: Network },
    { id: "my-info", label: "My Info", component: MyInfo },
  ];

  const current = $derived($screen.kind === "tab" ? tabs.find((t) => t.id === $screen.tab) : undefined);
  const Active = $derived(current?.component);

  let content: HTMLElement | undefined = $state();
  let keepFocus = false;
  let first = true;

  $effect(() => {
    void $screen;
    if (first) {
      first = false;
      return;
    }
    if (keepFocus) {
      keepFocus = false;
      return;
    }
    void tick().then(() => content?.querySelector<HTMLElement>("[data-screen-focus]")?.focus());
  });

  function onKeydown(e: KeyboardEvent, index: number) {
    const last = tabs.length - 1;
    let next: number;
    if (e.key === "ArrowRight") next = index === last ? 0 : index + 1;
    else if (e.key === "ArrowLeft") next = index === 0 ? last : index - 1;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = last;
    else return;
    e.preventDefault();
    keepFocus = true;
    showTab(tabs[next].id);
    void tick().then(() => document.getElementById(`tab-${tabs[next].id}`)?.focus());
  }

  onMount(() => {
    const started = [startNetEvents(), startContacts(), startChatEvents()];
    return () => {
      started.forEach((s) => s.then((unlisten) => unlisten()));
    };
  });
</script>

{#if current}
  <div role="tablist" class="tabs">
    {#each tabs as tab, i (tab.id)}
      <button
        role="tab"
        class="tab"
        id="tab-{tab.id}"
        aria-selected={tab.id === current.id}
        aria-controls="panel-{tab.id}"
        tabindex={tab.id === current.id ? 0 : -1}
        onclick={() => showTab(tab.id)}
        onkeydown={(e) => onKeydown(e, i)}>{tab.label}</button
      >
    {/each}
  </div>
{/if}
<div class="shell-prompts">
  <IncomingRequestPrompt />
  <NamingPrompt />
</div>
<div class="shell-content" bind:this={content}>
  {#if current && Active}
    <div role="tabpanel" id="panel-{current.id}" aria-labelledby="tab-{current.id}">
      <Active />
    </div>
  {:else if $screen.kind === "chat"}
    <Chat nodeId={$screen.nodeId} onBack={back} />
  {/if}
</div>
{#if current}
  <footer class="app-footer">p2p-messenger</footer>
{/if}
