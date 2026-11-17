<script lang="ts">
  import { onMount } from "svelte";
  import { contacts, contactsError, refreshContacts } from "../lib/contactsStore";
  import ContactRow from "./ContactRow.svelte";
  import LanPeers from "./LanPeers.svelte";

  let notice = $state<string | null>(null);

  onMount(refreshContacts);
</script>

<section>
  <h2>Contacts</h2>
  {#if $contactsError}<p role="alert" class="alert alert-danger">{$contactsError}</p>{/if}
  {#if notice}<p role="status" class="alert alert-info">{notice}</p>{/if}
  {#if $contacts.length === 0}
    <p class="muted">
      No contacts yet. Add someone from <em>On this network</em> below, connect with a ticket in the
      Network tab, or accept an incoming request.
    </p>
  {:else}
    <ul class="list">
      {#each $contacts as c (c.node_id)}
        <ContactRow contact={c} onNotice={(m) => (notice = m)} />
      {/each}
    </ul>
  {/if}
  <LanPeers />
</section>
