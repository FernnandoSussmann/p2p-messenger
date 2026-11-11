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
  {#if $contactsError}<p role="alert">{$contactsError}</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if $contacts.length === 0}
    <p>
      No contacts yet. Add someone from <em>On this network</em> below, connect with a ticket in the
      Network tab, or accept an incoming request.
    </p>
  {:else}
    <ul class="contacts">
      {#each $contacts as c (c.node_id)}
        <ContactRow contact={c} onNotice={(m) => (notice = m)} />
      {/each}
    </ul>
  {/if}
  <LanPeers />
</section>

<style>
  .contacts {
    list-style: none;
    padding: 0;
  }
</style>
