<script lang="ts">
  import { incomingRequests, dropIncoming } from "../lib/netStore";
  import { describeNetError, netRespondIncoming } from "../net";
  import NicknameForm from "./NicknameForm.svelte";

  const first = $derived($incomingRequests[0]);
  const more = $derived($incomingRequests.length - 1);
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    void first?.node_id;
    error = null;
  });

  async function respond(nodeId: string, accept: boolean, nickname?: string) {
    busy = true;
    error = null;
    try {
      if (accept) await netRespondIncoming(nodeId, true, nickname);
      else await netRespondIncoming(nodeId, false);
    } catch (e) {
      const type = (e as { type?: string } | undefined)?.type;
      busy = false;
      if (type !== "NoSuchRequest") {
        error = describeNetError(e);
        return;
      }
    }
    busy = false;
    dropIncoming(nodeId);
  }
</script>

{#if first}
  <div role="dialog" aria-label="Incoming connection request" data-testid="incoming-prompt">
    <p><strong data-testid="unverified-badge">Unverified</strong> — someone is asking to connect</p>
    <code data-testid="peer-node-id" style="word-break: break-all">{first.node_id}</code>
    {#if more > 0}<p>+{more} more</p>{/if}
    {#key first.node_id}
      <NicknameForm
        initial={first.suggested_nickname}
        selfName={first.display_name}
        submitLabel="Accept and add contact"
        {busy}
        {error}
        onSubmit={(nickname) => respond(first.node_id, true, nickname)}
      />
    {/key}
    <button type="button" disabled={busy} onclick={() => respond(first.node_id, false)}>Decline</button>
  </div>
{/if}
