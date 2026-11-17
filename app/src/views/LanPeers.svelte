<script lang="ts">
  import { lanPeers } from "../lib/netStore";
  import { contactIds, refreshContacts } from "../lib/contactsStore";
  import { contactsAdd, describeContactsError } from "../contacts";
  import { netConnectTicket, describeNetError } from "../net";
  import type { LanPeerDto } from "../net";
  import NicknameForm from "./NicknameForm.svelte";
  import NodeIdLine from "./NodeIdLine.svelte";
  import TrustBadge from "./TrustBadge.svelte";

  let adding = $state<string | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  const others = $derived($lanPeers.filter((p) => !$contactIds.has(p.node_id)));

  function open(id: string) {
    adding = id;
    error = null;
  }

  async function add(peer: LanPeerDto, nickname: string) {
    error = null;
    busy = true;
    try {
      try {
        await contactsAdd(peer.node_id, nickname);
      } catch (e) {
        error = describeContactsError(e);
        return;
      }
      await refreshContacts();
      adding = null;
      try {
        await netConnectTicket(peer.ticket);
      } catch (e) {
        error = `Added “${nickname}”, but connecting failed: ${describeNetError(e)}`;
      }
    } finally {
      busy = false;
    }
  }
</script>

<section class="stack">
  <h3>On this network</h3>
  <p class="muted">People nearby announcing themselves. Names are chosen by them and are not verified.</p>
  {#if others.length === 0}
    <p class="muted">No one new on this network.</p>
  {:else}
    <ul class="list">
      {#each others as peer (peer.node_id)}
        <li class="stack" data-testid="lan-peer">
          <TrustBadge verified={false} />
          {#if peer.display_name}calls themselves “<bdi data-testid="lan-peer-name">{peer.display_name}</bdi>”{:else}<span data-testid="lan-peer-name">(no name)</span>{/if}
          <NodeIdLine nodeId={peer.node_id} />
          <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => open(peer.node_id)}>Add…</button>
          {#if adding === peer.node_id}
            <NicknameForm
              selfName={peer.display_name}
              submitLabel="Add contact and connect"
              {busy}
              onSubmit={(n) => add(peer, n)}
              onCancel={() => (adding = null)}
            />
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
</section>
