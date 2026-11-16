<script lang="ts">
  import type { ContactDto, RemoveContactDto } from "../contacts";
  import { contactsRemove, describeContactsError } from "../contacts";
  import { refreshContacts } from "../lib/contactsStore";
  import { HISTORY_DELETE_GUARANTEE } from "../lib/historyCopy";

  interface Props {
    contact: ContactDto;
    onDone: (r: RemoveContactDto) => void;
    onCancel: () => void;
  }
  let { contact, onDone, onCancel }: Props = $props();

  let eraseHistory = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function confirm() {
    busy = true;
    error = null;
    try {
      const result = await contactsRemove(contact.node_id, eraseHistory);
      await refreshContacts();
      onDone(result);
    } catch (e) {
      error = describeContactsError(e);
    } finally {
      busy = false;
    }
  }
</script>

<div role="dialog" aria-label="Remove contact">
  <p><strong>Remove “<bdi>{contact.nickname}</bdi>”?</strong></p>
  <p>
    They will no longer be a contact. If they connect again, you will be asked to Accept or Decline
    them like someone new. Any open connection with them is closed now.
  </p>
  <label>
    <input type="checkbox" bind:checked={eraseHistory} />
    <strong>Also delete our message history with “<bdi>{contact.nickname}</bdi>”</strong>
  </label>
  {#if eraseHistory}
    <p>Your message history with them will be <strong>permanently deleted</strong>. This cannot be undone.</p>
    <p>{HISTORY_DELETE_GUARANTEE}</p>
  {:else}
    <p>Your message history with them will be <strong>kept</strong>.</p>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
  <button type="button" disabled={busy} onclick={confirm}>
    {eraseHistory ? "Remove contact and delete history" : "Remove contact"}
  </button>
  <button type="button" onclick={onCancel}>Cancel</button>
</div>
