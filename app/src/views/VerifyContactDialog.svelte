<script lang="ts">
  import type { ContactDto } from "../contacts";
  import { contactsSetVerified, describeContactsError } from "../contacts";
  import { refreshContacts } from "../lib/contactsStore";

  interface Props {
    contact: ContactDto;
    onDone: () => void;
    onCancel: () => void;
  }
  let { contact, onDone, onCancel }: Props = $props();

  let busy = $state(false);
  let error = $state<string | null>(null);

  async function confirm() {
    busy = true;
    error = null;
    try {
      await contactsSetVerified(contact.node_id, true);
      await refreshContacts();
      onDone();
    } catch (e) {
      error = describeContactsError(e);
    } finally {
      busy = false;
    }
  }
</script>

<div role="dialog" aria-label="Mark contact as verified">
  <p><strong>Mark “<bdi>{contact.nickname}</bdi>” as verified?</strong></p>
  <p>
    Only do this if you have confirmed, through a channel you trust (for example in person, or a
    ticket they handed you directly), that this NodeId belongs to the person you call “<bdi>{contact.nickname}</bdi>”:
  </p>
  <p><code class="selectable" data-testid="verify-node-id">{contact.node_id}</code></p>
  <p>
    This is your own statement. The app does not check it cryptographically and cannot tell if you
    were given the wrong ticket. You can undo it at any time.
  </p>
  {#if error}<p role="alert">{error}</p>{/if}
  <button type="button" disabled={busy} onclick={confirm}>Yes, I checked — mark verified</button>
  <button type="button" onclick={onCancel}>Cancel</button>
</div>

<style>
  .selectable {
    user-select: all;
    word-break: break-all;
  }
</style>
