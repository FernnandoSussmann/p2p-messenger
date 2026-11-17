<script lang="ts">
  import PresenceToggle from "./PresenceToggle.svelte";
  import { expectNaming } from "../lib/contactsStore";
  import { netConnectTicket, describeNetError } from "../net";

  let ticket = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

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
  <h2 class="visually-hidden" tabindex="-1" data-screen-focus>Network</h2>
  <PresenceToggle />
  <hr class="divider" />
  <form class="stack" onsubmit={connect}>
    <label class="field">
      Connect by ticket
      <textarea class="textarea" bind:value={ticket} rows="4"></textarea>
    </label>
    <div class="row end">
      <button type="submit" class="btn btn-primary" disabled={busy || ticket.trim() === ""}>Connect and remember this device</button>
    </div>
    <p class="muted">Once they reply, they're saved as an unverified contact under the name they gave, which you can change, and they can reconnect without asking you again.</p>
    {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
  </form>
</section>
