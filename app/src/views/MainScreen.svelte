<script lang="ts">
  import { onMount } from "svelte";
  import { activeTab, tabs } from "../lib/router";
  import { startNetEvents } from "../lib/netStore";
  import { startContacts } from "../lib/contactsStore";
  import { activeChat, closeChat, startChatEvents } from "../lib/chatStore";
  import Chat from "./Chat.svelte";
  import IncomingRequestPrompt from "./IncomingRequestPrompt.svelte";
  import NamingPrompt from "./NamingPrompt.svelte";

  const current = $derived(tabs.find((t) => t.id === $activeTab) ?? tabs[0]);
  const Active = $derived(current.component);

  onMount(() => {
    const started = [startNetEvents(), startContacts(), startChatEvents()];
    return () => {
      started.forEach((s) => s.then((unlisten) => unlisten()));
    };
  });
</script>

<IncomingRequestPrompt />
<NamingPrompt />
<div role="tablist">
  {#each tabs as tab (tab.id)}
    <button
      role="tab"
      aria-selected={tab.id === current.id}
      onclick={() => {
        closeChat();
        activeTab.set(tab.id);
      }}>{tab.label}</button
    >
  {/each}
</div>
<div role="tabpanel">
  {#if $activeChat}
    <Chat nodeId={$activeChat} onBack={closeChat} />
  {:else}
    <Active />
  {/if}
</div>
