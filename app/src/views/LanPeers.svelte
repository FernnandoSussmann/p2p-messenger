<script lang="ts">
  import { lanPeers } from "../lib/netStore";
  import { contactIds, refreshContacts } from "../lib/contactsStore";
  import { contactsAdd, describeContactsError } from "../contacts";
  import { netConnectTicket, describeNetError } from "../net";
  import type { LanPeerDto } from "../net";
  import NicknameForm from "./NicknameForm.svelte";
  import NodeIdLine from "./NodeIdLine.svelte";
  import TrustBadge from "./TrustBadge.svelte";

  let busy = $state(false);
  let error = $state<string | null>(null);

  const others = $derived($lanPeers.filter((p) => !$contactIds.has(p.node_id)));

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
        <li class="stack lan-peer" data-testid="lan-peer">
          <div class="row">
            <TrustBadge verified={false} />
            <span class="grow">{#if peer.display_name}calls themselves “<bdi data-testid="lan-peer-name">{peer.display_name}</bdi>”{:else}<span data-testid="lan-peer-name">(no name)</span>{/if}</span>
          </div>
          <NicknameForm
            selfName={peer.display_name}
            submitLabel="Add contact and connect"
            {busy}
            onSubmit={(n) => add(peer, n)}
          />
          <NodeIdLine nodeId={peer.node_id} />
        </li>
      {/each}
    </ul>
  {/if}
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
</section>
