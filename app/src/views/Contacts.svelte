<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { contacts, contactsError, contactsNotice, refreshContacts } from "../lib/contactsStore";
  import ContactRow from "./ContactRow.svelte";

  onMount(refreshContacts);
  onDestroy(() => contactsNotice.set(null));
</script>

<section>
  <h2 class="visually-hidden" tabindex="-1" data-screen-focus>Contacts</h2>
  {#if $contactsError}<p role="alert" class="alert alert-danger">{$contactsError}</p>{/if}
  {#if $contactsNotice}<p role="status" class="alert alert-info">{$contactsNotice}</p>{/if}
  {#if $contacts.length === 0}
    <p class="muted">
      No contacts yet. Add someone from <em>Discover</em>, connect with a ticket in the
      Network tab, or accept an incoming request.
    </p>
  {:else}
    <ul class="list">
      {#each $contacts as c (c.node_id)}
        <ContactRow contact={c} />
      {/each}
    </ul>
  {/if}
</section>
