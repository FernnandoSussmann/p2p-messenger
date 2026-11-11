<script lang="ts">
  import { onMount } from "svelte";
  import { activeTab, tabs } from "../lib/router";
  import { startNetEvents } from "../lib/netStore";
  import IncomingRequestPrompt from "./IncomingRequestPrompt.svelte";

  const current = $derived(tabs.find((t) => t.id === $activeTab) ?? tabs[0]);
  const Active = $derived(current.component);

  onMount(() => {
    const started = startNetEvents();
    return () => {
      started.then((unlisten) => unlisten());
    };
  });
</script>

<IncomingRequestPrompt />
<div role="tablist">
  {#each tabs as tab (tab.id)}
    <button
      role="tab"
      aria-selected={tab.id === current.id}
      onclick={() => activeTab.set(tab.id)}>{tab.label}</button
    >
  {/each}
</div>
<div role="tabpanel">
  <Active />
</div>
