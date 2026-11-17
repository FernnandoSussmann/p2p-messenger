<script lang="ts">
  import { onMount } from "svelte";
  import { netPresenceGet, netPresenceSet, describeNetError } from "../net";

  let enabled = $state(true);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      enabled = (await netPresenceGet()).enabled;
      loaded = true;
    } catch (e) {
      error = describeNetError(e);
    }
  });

  async function toggle(ev: Event) {
    const input = ev.currentTarget as HTMLInputElement;
    const next = input.checked;
    error = null;
    busy = true;
    try {
      enabled = (await netPresenceSet(next)).enabled;
    } catch (e) {
      enabled = !next;
      input.checked = enabled;
      error = describeNetError(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="stack">
  <div class="row presence-row">
    <span class="muted" aria-hidden="true">Off</span>
    <label class="switch" class:is-disabled={!loaded || busy}>
      <input type="checkbox" role="switch" checked={enabled} disabled={!loaded || busy} onchange={toggle} />
      Visible on this network
    </label>
    <span class="muted" aria-hidden="true">On</span>
  </div>
  <p class="muted">
    Off = other devices on this network won't see you in their list. You are <strong>not</strong> offline:
    anyone who already has your ticket can still connect, and you can still connect to others.
  </p>
  {#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
</div>
