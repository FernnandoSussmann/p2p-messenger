<script lang="ts">
  import { incomingRequests, dropIncoming } from "../lib/netStore";
  import { netRespondIncoming } from "../net";

  const first = $derived($incomingRequests[0]);
  const more = $derived($incomingRequests.length - 1);

  async function respond(nodeId: string, accept: boolean) {
    try {
      await netRespondIncoming(nodeId, accept);
    } catch (e) {
      if ((e as { type?: string } | undefined)?.type !== "NoSuchRequest") return;
    }
    dropIncoming(nodeId);
  }
</script>

{#if first}
  <div role="dialog" aria-label="Incoming connection request" data-testid="incoming-prompt">
    <p><strong data-testid="unverified-badge">Unverified</strong> — someone is asking to connect</p>
    <p>
      {#if first.display_name}calls themselves “<span data-testid="peer-name">{first.display_name}</span>”{:else}(no name){/if}
    </p>
    <code data-testid="peer-node-id" style="word-break: break-all">{first.node_id}</code>
    {#if more > 0}<p>+{more} more</p>{/if}
    <button type="button" onclick={() => respond(first.node_id, true)}>Accept</button>
    <button type="button" onclick={() => respond(first.node_id, false)}>Decline</button>
  </div>
{/if}
