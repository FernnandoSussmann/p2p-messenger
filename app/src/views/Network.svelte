<script lang="ts">
  import { connections } from "../lib/netStore";
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
      await netConnectTicket(value);
      ticket = "";
    } catch (e) {
      error = describeNetError(e);
    } finally {
      busy = false;
    }
  }
</script>

<section>
  <h2>Network</h2>
  <form onsubmit={connect}>
    <label>
      Connect by ticket
      <textarea bind:value={ticket} rows="4"></textarea>
    </label>
    <p>Once they reply, this device is remembered and can reconnect without asking you again.</p>
    <button type="submit" disabled={busy || ticket.trim() === ""}>Connect and remember this device</button>
    {#if error}<p role="alert">{error}</p>{/if}
  </form>

  <h3>Connections</h3>
  {#if list.length === 0}
    <p>No connections yet.</p>
  {:else}
    <ul>
      {#each list as c (c.node_id)}
        <li data-testid="connection">
          <code style="word-break: break-all">{c.node_id}</code>
          <span data-testid="conn-direction">{c.direction}</span>
          <span data-testid="conn-state">{c.state}</span>
          {#if c.reason}<span data-testid="conn-reason">{c.reason}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
