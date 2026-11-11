<script lang="ts">
  import { shortNodeId } from "../lib/nodeId";

  let { nodeId }: { nodeId: string } = $props();
  let open = $state(false);
</script>

<div class="node-id-line">
  <small class="node-id" title={nodeId} data-testid="node-id-short">{shortNodeId(nodeId)}</small>
  <button type="button" class="link" aria-expanded={open} onclick={() => (open = !open)}>
    {open ? "Hide full NodeId" : "Show full NodeId"}</button
  >
  {#if open}
    <code class="selectable" data-testid="node-id-full">{nodeId}</code>
    <small class="hint">Select it and use your system's copy shortcut.</small>
  {/if}
</div>

<style>
  .node-id {
    color: gray;
    font-size: 0.8em;
  }
  .selectable {
    user-select: all;
    word-break: break-all;
  }
</style>
