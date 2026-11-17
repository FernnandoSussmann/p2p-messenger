<script lang="ts">
  import PresenceToggle from "./PresenceToggle.svelte";
  import NodeIdLine from "./NodeIdLine.svelte";
  import { connections } from "../lib/netStore";
  import { contactIds, expectNaming, requestNaming } from "../lib/contactsStore";
  import { netConnectTicket, describeNetError } from "../net";

  let ticket = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  const list = $derived(Object.values($connections));

  async function connect(ev: Event) {
    ev.preventDefault();
    const value = ticket.trim();
    if (!value) return;
    error = null;
    busy = true;
    try {
      const res = await netConnectTicket(value);
      expectNaming(res.node_id);
      ticket = "";
    } catch (e) {
      error = describeNetError(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="stack">
  <h2>Network</h2>
  <PresenceToggle />
  <form class="stack" onsubmit={connect}>
    <label class="field">
      Connect by ticket
      <textarea class="textarea" bind:value={ticket} rows="4"></textarea>
    </label>
    <p class="muted">Once they reply, they're saved as an unverified contact under the name they gave, which you can change, and they can reconnect without asking you again.</p>
    <button type="submit" class="btn btn-primary" disabled={busy || ticket.trim() === ""}>Connect and remember this device</button>
    {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
  </form>

  <h3>Connections</h3>
  {#if list.length === 0}
    <p class="muted">No connections yet.</p>
  {:else}
    <ul class="list">
      {#each list as c (c.node_id)}
        <li class="stack" data-testid="connection">
          <NodeIdLine nodeId={c.node_id} />
          <span data-testid="conn-direction">{c.direction}</span>
          <span data-testid="conn-state">{c.state}</span>
          {#if c.reason}<span data-testid="conn-reason">{c.reason}</span>{/if}
          {#if c.state === "connected" && !$contactIds.has(c.node_id)}
            <button type="button" class="btn btn-secondary btn-sm" onclick={() => requestNaming(c.node_id, c.display_name)}>Add contact…</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
