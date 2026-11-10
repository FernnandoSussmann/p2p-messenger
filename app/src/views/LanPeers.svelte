<script lang="ts">
  import { lanPeers } from "../lib/netStore";
  import { netConnectTicket, describeNetError } from "../net";

  let error = $state<string | null>(null);
  let pending = $state<string | null>(null);

  async function connect(nodeId: string, ticket: string) {
    error = null;
    pending = nodeId;
    try {
      await netConnectTicket(ticket);
    } catch (e) {
      error = describeNetError(e);
    } finally {
      pending = null;
    }
  }
</script>

<section>
  <h3>Peers on this network</h3>
  {#if $lanPeers.length === 0}
    <p>No peers found on this network.</p>
  {:else}
    <ul>
      {#each $lanPeers as peer (peer.node_id)}
        <li data-testid="lan-peer">
          <span data-testid="lan-peer-name">{peer.display_name ?? "(no name)"}</span>
          {#if !peer.verified}<span data-testid="lan-peer-unverified">Unverified</span>{/if}
          <small style="color: gray; word-break: break-all" data-testid="lan-peer-id">{peer.node_id}</small>
          <button type="button" disabled={pending === peer.node_id} onclick={() => connect(peer.node_id, peer.ticket)}>
            Connect
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
