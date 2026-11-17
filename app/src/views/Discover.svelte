<script lang="ts">
  import LanPeers from "./LanPeers.svelte";
  import NodeIdLine from "./NodeIdLine.svelte";
  import TrustBadge from "./TrustBadge.svelte";
  import { connections } from "../lib/netStore";
  import { contactIds, requestNaming } from "../lib/contactsStore";

  const others = $derived(Object.values($connections).filter((c) => !$contactIds.has(c.node_id)));
</script>

<section class="stack">
  <h2 class="visually-hidden" tabindex="-1" data-screen-focus>Discover</h2>
  <LanPeers />
  {#if others.length > 0}
    <h3>Connections</h3>
    <ul class="list">
      {#each others as c (c.node_id)}
        <li class="stack" data-testid="connection">
          <TrustBadge verified={false} />
          <NodeIdLine nodeId={c.node_id} />
          <span class="muted">
            <bdi data-testid="conn-direction">{c.direction}</bdi> |
            <bdi data-testid="conn-state">{c.state}</bdi>{#if c.reason} | <bdi data-testid="conn-reason">{c.reason}</bdi>{/if}
          </span>
          {#if c.state === "connected"}
            <button type="button" class="btn btn-secondary btn-sm" onclick={() => requestNaming(c.node_id, c.display_name)}>Add contact…</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
