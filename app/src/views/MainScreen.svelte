<script lang="ts">
  import { activeTab, tabs } from "../lib/router";

  const current = $derived(tabs.find((t) => t.id === $activeTab) ?? tabs[0]);
  const Active = $derived(current.component);
</script>

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
