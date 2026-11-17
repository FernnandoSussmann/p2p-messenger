<script lang="ts">
  import { shortNodeId } from "../lib/nodeId";

  let { nodeId }: { nodeId: string } = $props();
  let open = $state(false);
</script>

<div class="stack">
  <small class="node-id" title={nodeId} data-testid="node-id-short">{shortNodeId(nodeId)}</small>
  <button type="button" class="btn btn-ghost btn-sm" aria-expanded={open} onclick={() => (open = !open)}>
    {open ? "Hide full NodeId" : "Show full NodeId"}</button
  >
  {#if open}
    <code class="selectable" data-testid="node-id-full">{nodeId}</code>
    <small class="hint">Select it and use your system's copy shortcut.</small>
  {/if}
</div>
